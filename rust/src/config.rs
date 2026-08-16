//! Process configuration and deterministic path resolution.
//!
//! Path selection mirrors the published Node CLI. Resolution is lexical: it
//! does not require the path to exist and it never follows symlinks.

use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::identity::SourceId;

pub const PROGRAM_NAME: &str = "shlog";
pub const LEGACY_PROGRAM_NAME: &str = "cxs";
pub const DB_FILE_NAME: &str = "index.sqlite";

/// The Rust tokenizer is intentionally index-incompatible with the v7 Node
/// tokenizer: it uses Unicode scalar values for CJK bigrams and a pinned UAX
/// #29 word segmenter. A new version prevents a mixed binary/index deployment
/// from silently returning incomplete recall.
pub const INDEX_VERSION: &str = "shlog-v8-unicode-word-cjk-scalar";

const CAPTURED_ENV_KEYS: &[&str] = &[
    "HOME",
    "SHLOG_DATA_DIR",
    "CXS_DATA_DIR",
    "XDG_STATE_HOME",
    "SHLOG_STATS",
    "CXS_STATS",
    "SHLOG_DEBUG_TIMING",
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EnvSnapshot {
    values: HashMap<OsString, OsString>,
}

impl EnvSnapshot {
    pub fn from_current_process() -> Self {
        let values = CAPTURED_ENV_KEYS
            .iter()
            .filter_map(|key| env::var_os(key).map(|value| (OsString::from(key), value)))
            .collect();
        Self { values }
    }

    pub fn from_pairs<K, V, I>(pairs: I) -> Self
    where
        K: Into<OsString>,
        V: Into<OsString>,
        I: IntoIterator<Item = (K, V)>,
    {
        Self {
            values: pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&OsStr> {
        self.values.get(OsStr::new(key)).map(OsString::as_os_str)
    }

    pub fn get_non_empty(&self, key: &str) -> Option<&OsStr> {
        self.get(key).filter(|value| !value.is_empty())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub default_codex_dir: PathBuf,
    pub default_claude_code_dir: PathBuf,
    pub default_pi_dir: PathBuf,
    pub default_dsh_dir: PathBuf,
    pub legacy_data_dirs: Vec<PathBuf>,
}

impl ResolvedPaths {
    pub fn default_source_root(&self, source: SourceId) -> &Path {
        match source {
            SourceId::Codex => &self.default_codex_dir,
            SourceId::ClaudeCode => &self.default_claude_code_dir,
            SourceId::Pi => &self.default_pi_dir,
            SourceId::Dsh => &self.default_dsh_dir,
        }
    }
}

pub fn resolve_paths(env: &EnvSnapshot, cwd: &Path, home: &Path) -> ResolvedPaths {
    let data_dir = if let Some(path) = env.get_non_empty("SHLOG_DATA_DIR") {
        resolve_lexical(path, cwd)
    } else if let Some(path) = env.get_non_empty("CXS_DATA_DIR") {
        resolve_lexical(path, cwd)
    } else if let Some(state_home) = env.get_non_empty("XDG_STATE_HOME") {
        resolve_lexical(Path::new(state_home).join(PROGRAM_NAME), cwd)
    } else {
        resolve_lexical(home.join(".local/state").join(PROGRAM_NAME), cwd)
    };

    let legacy_state_dir = if let Some(state_home) = env.get_non_empty("XDG_STATE_HOME") {
        resolve_lexical(Path::new(state_home).join(LEGACY_PROGRAM_NAME), cwd)
    } else {
        resolve_lexical(home.join(".local/state").join(LEGACY_PROGRAM_NAME), cwd)
    };
    let legacy_cache_dir = resolve_lexical(home.join(".cache").join(LEGACY_PROGRAM_NAME), cwd);
    let mut legacy_data_dirs = vec![legacy_state_dir];
    if legacy_data_dirs[0] != legacy_cache_dir {
        legacy_data_dirs.push(legacy_cache_dir);
    }

    ResolvedPaths {
        db_path: data_dir.join(DB_FILE_NAME),
        data_dir,
        default_codex_dir: resolve_lexical(home.join(".codex/sessions"), cwd),
        default_claude_code_dir: resolve_lexical(home.join(".claude/projects"), cwd),
        default_pi_dir: resolve_lexical(home.join(".pi/agent/sessions"), cwd),
        default_dsh_dir: resolve_lexical(home.join(".dsh/sessions"), cwd),
        legacy_data_dirs,
    }
}

pub fn resolve_source_root(
    source: SourceId,
    override_path: Option<&Path>,
    paths: &ResolvedPaths,
    cwd: &Path,
) -> PathBuf {
    override_path
        .map(|path| resolve_lexical(path, cwd))
        .unwrap_or_else(|| paths.default_source_root(source).to_path_buf())
}

pub fn default_db_path() -> PathBuf {
    let env = EnvSnapshot::from_current_process();
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = env
        .get_non_empty("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.clone());
    resolve_paths(&env, &cwd, &home).db_path
}

pub fn is_current_index_version(value: Option<&str>) -> bool {
    value == Some(INDEX_VERSION)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LegacyDataDirMigration {
    NotNeeded,
    Migrated {
        legacy_dir: PathBuf,
        destination_dir: PathBuf,
    },
}

#[derive(Debug, Error)]
pub enum LegacyDataDirMigrationError {
    #[error(
        "multiple legacy Sherlog state directories exist ({legacy_dirs:?}); refusing to choose one for {destination_dir}"
    )]
    MultipleLegacyDirectories {
        legacy_dirs: Vec<PathBuf>,
        destination_dir: PathBuf,
    },
    #[error(
        "legacy Sherlog state directory {legacy_dir} and destination {destination_dir} both exist"
    )]
    Conflict {
        legacy_dir: PathBuf,
        destination_dir: PathBuf,
    },
    #[error("failed to {operation} {}: {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Move the one unambiguous historical state directory into the current
/// location. This is a writer-only transition: callers must surface conflicts
/// and I/O failures instead of bootstrapping a second state directory.
pub fn migrate_legacy_data_dir_if_needed(
    paths: &ResolvedPaths,
) -> Result<LegacyDataDirMigration, LegacyDataDirMigrationError> {
    let mut existing_legacy_dirs = Vec::new();
    for legacy_dir in &paths.legacy_data_dirs {
        if legacy_dir == &paths.data_dir || existing_legacy_dirs.contains(legacy_dir) {
            continue;
        }
        if path_exists(legacy_dir, "inspect legacy state directory")? {
            existing_legacy_dirs.push(legacy_dir.clone());
        }
    }

    if existing_legacy_dirs.len() > 1 {
        return Err(LegacyDataDirMigrationError::MultipleLegacyDirectories {
            legacy_dirs: existing_legacy_dirs,
            destination_dir: paths.data_dir.clone(),
        });
    }

    let Some(legacy_dir) = existing_legacy_dirs.first() else {
        return Ok(LegacyDataDirMigration::NotNeeded);
    };
    migrate_legacy_data_dir(legacy_dir, &paths.data_dir)
}

pub fn migrate_legacy_data_dir(
    legacy_dir: &Path,
    destination_dir: &Path,
) -> Result<LegacyDataDirMigration, LegacyDataDirMigrationError> {
    migrate_legacy_data_dir_with(legacy_dir, destination_dir, |source, destination| {
        fs::rename(source, destination)
    })
}

fn migrate_legacy_data_dir_with(
    legacy_dir: &Path,
    destination_dir: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<LegacyDataDirMigration, LegacyDataDirMigrationError> {
    if legacy_dir == destination_dir {
        return Ok(LegacyDataDirMigration::NotNeeded);
    }
    let Some(legacy_metadata) = path_metadata(legacy_dir, "inspect legacy state directory")? else {
        return Ok(LegacyDataDirMigration::NotNeeded);
    };
    if !legacy_metadata.is_dir() {
        return Err(migration_io_error(
            "inspect legacy state directory",
            legacy_dir,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "legacy state path is not a directory",
            ),
        ));
    }
    if path_exists(destination_dir, "inspect destination state directory")? {
        return Err(LegacyDataDirMigrationError::Conflict {
            legacy_dir: legacy_dir.to_path_buf(),
            destination_dir: destination_dir.to_path_buf(),
        });
    }
    let parent = destination_dir
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            migration_io_error(
                "resolve destination state directory parent",
                destination_dir,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "destination state directory has no parent",
                ),
            )
        })?;
    fs::create_dir_all(parent)
        .map_err(|source| migration_io_error("create destination parent", parent, source))?;

    // Do not add a copy fallback here. In particular, EXDEV means the legacy
    // state remains authoritative and the writer must stop until the operator
    // selects a same-filesystem destination or moves it explicitly.
    rename(legacy_dir, destination_dir).map_err(|source| {
        migration_io_error("rename legacy state directory", legacy_dir, source)
    })?;

    if path_exists(legacy_dir, "verify legacy state directory removal")? {
        return Err(migration_io_error(
            "verify legacy state directory removal",
            legacy_dir,
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "legacy state directory still exists after rename",
            ),
        ));
    }
    if !path_exists(destination_dir, "verify destination state directory")? {
        return Err(migration_io_error(
            "verify destination state directory",
            destination_dir,
            io::Error::new(
                io::ErrorKind::NotFound,
                "destination state directory is missing after rename",
            ),
        ));
    }

    Ok(LegacyDataDirMigration::Migrated {
        legacy_dir: legacy_dir.to_path_buf(),
        destination_dir: destination_dir.to_path_buf(),
    })
}

fn path_exists(path: &Path, operation: &'static str) -> Result<bool, LegacyDataDirMigrationError> {
    path_metadata(path, operation).map(|metadata| metadata.is_some())
}

fn path_metadata(
    path: &Path,
    operation: &'static str,
) -> Result<Option<fs::Metadata>, LegacyDataDirMigrationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(migration_io_error(operation, path, source)),
    }
}

fn migration_io_error(
    operation: &'static str,
    path: &Path,
    source: io::Error,
) -> LegacyDataDirMigrationError {
    LegacyDataDirMigrationError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

pub fn stats_readout_enabled(env: &EnvSnapshot) -> bool {
    let value = env
        .get("SHLOG_STATS")
        .or_else(|| env.get("CXS_STATS"))
        .unwrap_or_default()
        .to_string_lossy()
        .trim()
        .to_ascii_lowercase();
    !matches!(value.as_str(), "0" | "off" | "false" | "no")
}

pub fn coverage_debug_timing_enabled(env: &EnvSnapshot) -> bool {
    let value = env
        .get("SHLOG_DEBUG_TIMING")
        .unwrap_or_default()
        .to_string_lossy()
        .trim()
        .to_ascii_lowercase();
    matches!(value.as_str(), "1" | "true" | "yes" | "on")
}

/// Resolve a path against `cwd` and remove `.`/`..` components without
/// touching the filesystem. Callers should pass an absolute `cwd`.
pub fn resolve_lexical(path: impl AsRef<Path>, cwd: &Path) -> PathBuf {
    let path = path.as_ref();
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    normalize_lexical(&joined)
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    let rooted = path.is_absolute();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => output.push(prefix.as_os_str()),
            Component::RootDir => output.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = matches!(output.components().next_back(), Some(Component::Normal(_)));
                if can_pop {
                    output.pop();
                } else if !rooted {
                    output.push(component.as_os_str());
                }
            }
            Component::Normal(part) => output.push(part),
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(env: EnvSnapshot) -> ResolvedPaths {
        resolve_paths(&env, Path::new("/work/repo"), Path::new("/Users/tester"))
    }

    #[test]
    fn data_dir_uses_new_then_legacy_then_xdg_then_home_precedence() {
        let all = paths(EnvSnapshot::from_pairs([
            ("SHLOG_DATA_DIR", "../new-state"),
            ("CXS_DATA_DIR", "/legacy-override"),
            ("XDG_STATE_HOME", "/xdg-state"),
        ]));
        assert_eq!(all.data_dir, Path::new("/work/new-state"));

        let legacy = paths(EnvSnapshot::from_pairs([
            ("SHLOG_DATA_DIR", ""),
            ("CXS_DATA_DIR", "/legacy-override"),
            ("XDG_STATE_HOME", "/xdg-state"),
        ]));
        assert_eq!(legacy.data_dir, Path::new("/legacy-override"));

        let xdg = paths(EnvSnapshot::from_pairs([(
            "XDG_STATE_HOME",
            "../xdg-state",
        )]));
        assert_eq!(xdg.data_dir, Path::new("/work/xdg-state/shlog"));

        let fallback = paths(EnvSnapshot::default());
        assert_eq!(
            fallback.data_dir,
            Path::new("/Users/tester/.local/state/shlog")
        );
        assert_eq!(
            fallback.db_path,
            Path::new("/Users/tester/.local/state/shlog/index.sqlite")
        );
    }

    #[test]
    fn exposes_default_source_and_legacy_roots() {
        let resolved = paths(EnvSnapshot::from_pairs([("XDG_STATE_HOME", "/state")]));
        assert_eq!(
            resolved.default_source_root(SourceId::Codex),
            Path::new("/Users/tester/.codex/sessions")
        );
        assert_eq!(
            resolved.default_source_root(SourceId::ClaudeCode),
            Path::new("/Users/tester/.claude/projects")
        );
        assert_eq!(
            resolved.default_source_root(SourceId::Pi),
            Path::new("/Users/tester/.pi/agent/sessions")
        );
        assert_eq!(
            resolved.default_source_root(SourceId::Dsh),
            Path::new("/Users/tester/.dsh/sessions")
        );
        assert_eq!(
            resolved.legacy_data_dirs,
            [
                PathBuf::from("/state/cxs"),
                PathBuf::from("/Users/tester/.cache/cxs")
            ]
        );
    }

    #[test]
    fn source_override_is_lexically_resolved() {
        let resolved = paths(EnvSnapshot::default());
        assert_eq!(
            resolve_source_root(
                SourceId::Pi,
                Some(Path::new("../sessions/./pi")),
                &resolved,
                Path::new("/work/repo"),
            ),
            Path::new("/work/sessions/pi")
        );
    }

    #[test]
    fn stats_new_env_wins_even_when_empty() {
        let new_disabled =
            EnvSnapshot::from_pairs([("SHLOG_STATS", " off "), ("CXS_STATS", "yes")]);
        assert!(!stats_readout_enabled(&new_disabled));

        let empty_new = EnvSnapshot::from_pairs([("SHLOG_STATS", ""), ("CXS_STATS", "off")]);
        assert!(stats_readout_enabled(&empty_new));

        let legacy_disabled = EnvSnapshot::from_pairs([("CXS_STATS", "FALSE")]);
        assert!(!stats_readout_enabled(&legacy_disabled));
    }

    #[test]
    fn debug_timing_is_opt_in() {
        for enabled in ["1", "true", "YES", " on "] {
            assert!(coverage_debug_timing_enabled(&EnvSnapshot::from_pairs([(
                "SHLOG_DEBUG_TIMING",
                enabled
            )])));
        }
        for disabled in ["", "0", "false", "anything"] {
            assert!(!coverage_debug_timing_enabled(&EnvSnapshot::from_pairs([
                ("SHLOG_DEBUG_TIMING", disabled)
            ])));
        }
    }

    #[test]
    fn version_is_deliberately_not_v7_compatible() {
        assert!(is_current_index_version(Some(INDEX_VERSION)));
        assert!(!is_current_index_version(Some("shlog-v7-source-identity")));
        assert!(!is_current_index_version(Some("cxs-v7-source-identity")));
        assert!(!is_current_index_version(None));
    }

    #[test]
    fn migrates_a_legacy_state_dir_and_reports_the_exact_move() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("legacy/cxs");
        let destination = base.path().join("state/shlog");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join(DB_FILE_NAME), "legacy").unwrap();

        assert_eq!(
            migrate_legacy_data_dir(&legacy, &destination).unwrap(),
            LegacyDataDirMigration::Migrated {
                legacy_dir: legacy.clone(),
                destination_dir: destination.clone(),
            }
        );
        assert!(!legacy.exists());
        assert_eq!(
            fs::read_to_string(destination.join(DB_FILE_NAME)).unwrap(),
            "legacy"
        );
    }

    #[test]
    fn legacy_and_destination_conflict_fails_closed_without_clobbering() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("legacy/cxs");
        let destination = base.path().join("state/shlog");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(legacy.join(DB_FILE_NAME), "legacy").unwrap();
        fs::write(destination.join(DB_FILE_NAME), "current").unwrap();

        let error = migrate_legacy_data_dir(&legacy, &destination).unwrap_err();
        assert!(matches!(
            error,
            LegacyDataDirMigrationError::Conflict {
                legacy_dir,
                destination_dir,
            } if legacy_dir == legacy && destination_dir == destination
        ));
        assert!(legacy.exists());
        assert_eq!(
            fs::read_to_string(destination.join(DB_FILE_NAME)).unwrap(),
            "current"
        );
    }

    #[test]
    fn migration_is_a_noop_for_missing_or_identical_paths() {
        let base = tempfile::tempdir().unwrap();
        let missing = base.path().join("missing");
        let destination = base.path().join("destination");
        assert_eq!(
            migrate_legacy_data_dir(&missing, &destination).unwrap(),
            LegacyDataDirMigration::NotNeeded
        );
        assert!(!destination.exists());

        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join(DB_FILE_NAME), "keep").unwrap();
        assert_eq!(
            migrate_legacy_data_dir(&destination, &destination).unwrap(),
            LegacyDataDirMigration::NotNeeded
        );
        assert_eq!(
            fs::read_to_string(destination.join(DB_FILE_NAME)).unwrap(),
            "keep"
        );
    }

    #[test]
    fn multiple_legacy_state_dirs_are_an_error_even_when_destination_is_missing() {
        let base = tempfile::tempdir().unwrap();
        let first = base.path().join("state/cxs");
        let second = base.path().join("cache/cxs");
        let destination = base.path().join("state/shlog");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let resolved = ResolvedPaths {
            data_dir: destination.clone(),
            db_path: destination.join(DB_FILE_NAME),
            default_codex_dir: base.path().join("codex"),
            default_claude_code_dir: base.path().join("claude"),
            default_pi_dir: base.path().join("pi"),
            default_dsh_dir: base.path().join("dsh"),
            legacy_data_dirs: vec![first.clone(), second.clone()],
        };

        let error = migrate_legacy_data_dir_if_needed(&resolved).unwrap_err();
        assert!(matches!(
            error,
            LegacyDataDirMigrationError::MultipleLegacyDirectories {
                legacy_dirs,
                destination_dir,
            } if legacy_dirs == vec![first.clone(), second.clone()]
                && destination_dir == destination
        ));
        assert!(first.exists());
        assert!(second.exists());
        assert!(!destination.exists());
    }

    #[test]
    fn duplicate_legacy_candidates_are_considered_once() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("state/cxs");
        let destination = base.path().join("state/shlog");
        fs::create_dir_all(&legacy).unwrap();
        let resolved = ResolvedPaths {
            data_dir: destination.clone(),
            db_path: destination.join(DB_FILE_NAME),
            default_codex_dir: base.path().join("codex"),
            default_claude_code_dir: base.path().join("claude"),
            default_pi_dir: base.path().join("pi"),
            default_dsh_dir: base.path().join("dsh"),
            legacy_data_dirs: vec![legacy.clone(), legacy.clone()],
        };

        assert_eq!(
            migrate_legacy_data_dir_if_needed(&resolved).unwrap(),
            LegacyDataDirMigration::Migrated {
                legacy_dir: legacy,
                destination_dir: destination,
            }
        );
    }

    #[test]
    fn cross_volume_rename_failure_is_typed_and_never_falls_back_to_copying() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("cxs");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join(DB_FILE_NAME), "legacy").unwrap();
        let destination = base.path().join("new-volume/shlog");

        let error = migrate_legacy_data_dir_with(&legacy, &destination, |_, _| {
            Err(io::Error::new(
                io::ErrorKind::CrossesDevices,
                "injected EXDEV",
            ))
        })
        .unwrap_err();
        assert!(matches!(
            error,
            LegacyDataDirMigrationError::Io {
                operation: "rename legacy state directory",
                path,
                source,
            } if path == legacy && source.kind() == io::ErrorKind::CrossesDevices
        ));
        assert!(legacy.exists());
        assert_eq!(
            fs::read_to_string(legacy.join(DB_FILE_NAME)).unwrap(),
            "legacy"
        );
        assert!(!destination.exists());
    }

    #[test]
    fn non_directory_legacy_path_is_a_typed_io_error() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("cxs");
        let destination = base.path().join("shlog");
        fs::write(&legacy, "not a directory").unwrap();

        let error = migrate_legacy_data_dir(&legacy, &destination).unwrap_err();
        assert!(matches!(
            error,
            LegacyDataDirMigrationError::Io {
                operation: "inspect legacy state directory",
                path,
                source,
            } if path == legacy && source.kind() == io::ErrorKind::InvalidInput
        ));
        assert!(!destination.exists());
    }
}

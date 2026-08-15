//! Persistent configuration for user-registered cold session roots.

use std::fmt;
use std::fs;
#[cfg(test)]
use std::fs::OpenOptions;
#[cfg(test)]
use std::io::{self, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::config::resolve_lexical;

pub const COLD_ROOTS_VERSION: u64 = 1;
pub const COLD_ROOTS_FILE_NAME: &str = "cold-roots.json";
const LEGACY_ADDED_AT: &str = "1970-01-01T00:00:00.000Z";
#[cfg(test)]
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColdRootEntry {
    pub source_id: String,
    pub root: String,
    pub added_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ColdRootsConfig {
    pub version: u64,
    pub roots: Vec<ColdRootEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColdRootError {
    message: String,
}

impl ColdRootError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ColdRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ColdRootError {}

pub fn cold_roots_path_for_db(db_path: &Path, cwd: &Path) -> PathBuf {
    let db_path = resolve_lexical(db_path, cwd);
    db_path.parent().unwrap_or(cwd).join(COLD_ROOTS_FILE_NAME)
}

pub fn empty_cold_roots_config() -> ColdRootsConfig {
    ColdRootsConfig {
        version: COLD_ROOTS_VERSION,
        roots: vec![],
    }
}

/// A missing, unreadable, or malformed file is treated as empty, matching the
/// published CLI's recoverable-config behavior.
pub fn load_cold_roots_config(config_path: &Path, cwd: &Path) -> ColdRootsConfig {
    let config_path = resolve_lexical(config_path, cwd);
    let Ok(contents) = fs::read_to_string(config_path) else {
        return empty_cold_roots_config();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return empty_cold_roots_config();
    };
    let Some(items) = value.get("roots").and_then(serde_json::Value::as_array) else {
        return empty_cold_roots_config();
    };

    let roots = items
        .iter()
        .filter_map(|item| normalize_loaded_entry(item, cwd))
        .collect();
    ColdRootsConfig {
        version: COLD_ROOTS_VERSION,
        roots,
    }
}

#[cfg(test)]
pub fn save_cold_roots_config(
    config_path: &Path,
    config: &ColdRootsConfig,
    cwd: &Path,
) -> Result<(), ColdRootError> {
    let config_path = resolve_lexical(config_path, cwd);
    let normalized = ColdRootsConfig {
        version: COLD_ROOTS_VERSION,
        roots: config
            .roots
            .iter()
            .map(|entry| {
                Ok(ColdRootEntry {
                    source_id: entry.source_id.clone(),
                    root: path_string(resolve_lexical(&entry.root, cwd))?,
                    added_at: entry.added_at.clone(),
                })
            })
            .collect::<Result<Vec<_>, ColdRootError>>()?,
    };
    let mut bytes = serde_json::to_vec_pretty(&normalized).map_err(|error| {
        ColdRootError::new(format!("failed to encode cold root config: {error}"))
    })?;
    bytes.push(b'\n');
    atomic_write(&config_path, &bytes)
}

pub fn list_cold_root_entries(
    config_path: &Path,
    source_id: Option<&str>,
    cwd: &Path,
) -> Vec<ColdRootEntry> {
    let config = load_cold_roots_config(config_path, cwd);
    match source_id {
        Some(source_id) => config
            .roots
            .into_iter()
            .filter(|entry| entry.source_id == source_id)
            .collect(),
        None => config.roots,
    }
}

#[cfg(test)]
pub fn add_cold_root(
    config_path: &Path,
    root: &Path,
    source_id: &str,
    added_at: &str,
    cwd: &Path,
) -> Result<ColdRootEntry, ColdRootError> {
    let resolved_root = resolve_lexical(root, cwd);
    let root_string = path_string(&resolved_root)?;
    let metadata = match fs::metadata(&resolved_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ColdRootError::new(format!(
                "cold root does not exist: {root_string}"
            )));
        }
        Err(error) => {
            return Err(ColdRootError::new(format!(
                "cannot inspect cold root {root_string}: {error}"
            )));
        }
    };
    if !metadata.is_dir() {
        return Err(ColdRootError::new(format!(
            "cold root is not a directory: {root_string}"
        )));
    }

    let mut config = load_cold_roots_config(config_path, cwd);
    if let Some(existing) = config
        .roots
        .iter()
        .find(|entry| entry.source_id == source_id && entry.root == root_string)
    {
        return Ok(existing.clone());
    }

    let entry = ColdRootEntry {
        source_id: source_id.to_owned(),
        root: root_string,
        added_at: added_at.to_owned(),
    };
    config.roots.push(entry.clone());
    save_cold_roots_config(config_path, &config, cwd)?;
    Ok(entry)
}

#[cfg(test)]
pub fn remove_cold_root(
    config_path: &Path,
    root: &Path,
    source_id: &str,
    cwd: &Path,
) -> Result<bool, ColdRootError> {
    let resolved_root = path_string(resolve_lexical(root, cwd))?;
    let mut config = load_cold_roots_config(config_path, cwd);
    let original_len = config.roots.len();
    config
        .roots
        .retain(|entry| !(entry.source_id == source_id && entry.root == resolved_root));
    if config.roots.len() == original_len {
        return Ok(false);
    }
    save_cold_roots_config(config_path, &config, cwd)?;
    Ok(true)
}

pub fn current_timestamp_millis() -> String {
    Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

pub fn path_string(path: impl AsRef<Path>) -> Result<String, ColdRootError> {
    path.as_ref()
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| ColdRootError::new("cold root path must be valid UTF-8"))
}

fn normalize_loaded_entry(value: &serde_json::Value, cwd: &Path) -> Option<ColdRootEntry> {
    let root = value.get("root")?.as_str()?.trim();
    if root.is_empty() {
        return None;
    }
    let source_id = value
        .get("sourceId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("codex")
        .to_owned();
    let added_at = value
        .get("addedAt")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(LEGACY_ADDED_AT)
        .to_owned();
    Some(ColdRootEntry {
        source_id,
        root: path_string(resolve_lexical(root, cwd)).ok()?,
        added_at,
    })
}

#[cfg(test)]
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ColdRootError> {
    let parent = path
        .parent()
        .ok_or_else(|| ColdRootError::new("cold root config path has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|error| config_io_error("create", path, error))?;

    let (temporary_path, mut temporary_file) = create_temporary_file(parent, path)?;
    let write_result = temporary_file
        .write_all(bytes)
        .and_then(|()| temporary_file.sync_all());
    drop(temporary_file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(config_io_error("write", path, error));
    }
    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(config_io_error("replace", path, error));
    }
    Ok(())
}

#[cfg(test)]
fn create_temporary_file(
    parent: &Path,
    config_path: &Path,
) -> Result<(PathBuf, fs::File), ColdRootError> {
    for _ in 0..100 {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".cold-roots.{}.{}.tmp", process::id(), sequence));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(config_io_error("create", config_path, error)),
        }
    }
    Err(ColdRootError::new(format!(
        "failed to create temporary cold root config next to {}",
        config_path.display()
    )))
}

#[cfg(test)]
fn config_io_error(action: &str, path: &Path, error: io::Error) -> ColdRootError {
    ColdRootError::new(format!(
        "failed to {action} cold root config {}: {error}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST_TIMESTAMP: &str = "2026-08-15T01:02:03.004Z";

    #[test]
    fn config_path_is_absolute_and_adjacent_to_db() {
        assert_eq!(
            cold_roots_path_for_db(Path::new("state/../state/index.sqlite"), Path::new("/work")),
            Path::new("/work/state/cold-roots.json")
        );
    }

    #[test]
    fn add_list_duplicate_and_remove_are_source_aware() {
        let base = tempfile::tempdir().unwrap();
        let cwd = base.path();
        let config_path = base.path().join("state/cold-roots.json");
        let root = base.path().join("archived sessions");
        fs::create_dir_all(&root).unwrap();

        let first = add_cold_root(&config_path, &root, "codex", FIRST_TIMESTAMP, cwd).unwrap();
        let duplicate = add_cold_root(
            &config_path,
            &root,
            "codex",
            "2099-01-01T00:00:00.000Z",
            cwd,
        )
        .unwrap();
        assert_eq!(duplicate, first);

        let pi = add_cold_root(&config_path, &root, "pi", "2026-08-15T01:02:03.005Z", cwd).unwrap();
        assert_eq!(
            list_cold_root_entries(&config_path, None, cwd),
            [first.clone(), pi]
        );
        assert_eq!(
            list_cold_root_entries(&config_path, Some("codex"), cwd),
            [first]
        );

        assert!(remove_cold_root(&config_path, &root, "codex", cwd).unwrap());
        assert!(!remove_cold_root(&config_path, &root, "codex", cwd).unwrap());
        assert_eq!(
            list_cold_root_entries(&config_path, None, cwd)[0].source_id,
            "pi"
        );
    }

    #[test]
    fn saved_config_is_pretty_normalized_and_has_no_temp_file() {
        let base = tempfile::tempdir().unwrap();
        let cwd = base.path();
        let config_path = base.path().join("state/cold-roots.json");
        let root = base.path().join("cold");
        fs::create_dir_all(&root).unwrap();
        add_cold_root(
            &config_path,
            Path::new("cold"),
            "codex",
            FIRST_TIMESTAMP,
            cwd,
        )
        .unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        assert!(contents.ends_with('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&contents).unwrap(),
            serde_json::json!({
                "version": 1,
                "roots": [{
                    "sourceId": "codex",
                    "root": root.to_str().unwrap(),
                    "addedAt": FIRST_TIMESTAMP
                }]
            })
        );
        let state_entries = fs::read_dir(config_path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(
            state_entries,
            [std::ffi::OsString::from(COLD_ROOTS_FILE_NAME)]
        );
    }

    #[test]
    fn malformed_config_is_empty_and_legacy_entries_are_normalized() {
        let base = tempfile::tempdir().unwrap();
        let cwd = base.path();
        let config_path = base.path().join(COLD_ROOTS_FILE_NAME);
        fs::write(&config_path, "not json").unwrap();
        assert_eq!(
            load_cold_roots_config(&config_path, cwd),
            empty_cold_roots_config()
        );

        fs::write(
            &config_path,
            r#"{"version":99,"roots":[{"root":"relative"},{"root":" "},{"bad":true}]}"#,
        )
        .unwrap();
        assert_eq!(
            load_cold_roots_config(&config_path, cwd),
            ColdRootsConfig {
                version: COLD_ROOTS_VERSION,
                roots: vec![ColdRootEntry {
                    source_id: "codex".to_owned(),
                    root: base.path().join("relative").to_str().unwrap().to_owned(),
                    added_at: LEGACY_ADDED_AT.to_owned(),
                }],
            }
        );
    }

    #[test]
    fn add_rejects_missing_roots_and_regular_files() {
        let base = tempfile::tempdir().unwrap();
        let config_path = base.path().join(COLD_ROOTS_FILE_NAME);
        let missing = base.path().join("missing");
        let error = add_cold_root(
            &config_path,
            &missing,
            "codex",
            FIRST_TIMESTAMP,
            base.path(),
        )
        .unwrap_err();
        assert!(error.message().starts_with("cold root does not exist:"));

        let file = base.path().join("file.jsonl");
        fs::write(&file, "line").unwrap();
        let error =
            add_cold_root(&config_path, &file, "codex", FIRST_TIMESTAMP, base.path()).unwrap_err();
        assert!(error.message().starts_with("cold root is not a directory:"));
    }
}

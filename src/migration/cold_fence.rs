use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, symlink};

use serde::{Deserialize, Serialize};

use crate::cold::ColdRootEntry;
use crate::sync::LegacyCutover;

use super::artifacts::{ensure_supported_platform, sync_file, sync_parent};
use super::error::{MigrationError, MigrationResult};

const FENCE_FORMAT_VERSION: u64 = 1;
const STATE_SUFFIX: &str = ".v8-tombstone.";
const STAGED_SUFFIX: &str = ".v8-fence.next.";
const MARKER_FILE: &str = "transition.json";
const BACKUP_FILE: &str = "cold-roots.v7.json";
static FENCE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FenceMarker {
    format_version: u64,
    config_name: String,
    state_directory: String,
    original_existed: bool,
    initial_digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FencePhase {
    Initial {
        original_existed: bool,
    },
    Staged {
        state_directory: PathBuf,
        staged_link: PathBuf,
    },
    Published {
        state_directory: PathBuf,
    },
}

/// Crash-recoverable filesystem cutover for the legacy cold-roots JSON file.
///
/// `inspect` is read-only, so callers may recover the pending roots before a
/// sync operation acquires its writer lock. `preflight` revalidates that exact
/// snapshot under the lock and prepares a private durable backup. `publish`
/// atomically replaces the legacy pathname with a symlink to the private state
/// directory. Old `writeFileSync` callers then fail with `EISDIR`, while a
/// subsequent Rust writer can recover the complete snapshot from the backup.
///
/// A published fence is permanent. In particular, database failure after
/// `publish` must never restore or unlink the legacy JSON pathname.
#[derive(Debug)]
pub(crate) struct ColdConfigFence {
    config_path: PathBuf,
    snapshot: Option<Vec<u8>>,
    source_identity: Option<(u64, u64)>,
    phase: FencePhase,
}

impl ColdConfigFence {
    /// Inspect a regular/missing legacy config or recover an owned published
    /// fence. This method performs no writes.
    pub(crate) fn inspect(config_path: &Path) -> MigrationResult<Self> {
        ensure_supported_platform()?;
        require_config_components(config_path)?;
        match fs::symlink_metadata(config_path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                let bytes = read_stable_regular_file(config_path)?;
                let source_identity = file_identity(config_path)?;
                Ok(Self {
                    config_path: config_path.to_path_buf(),
                    snapshot: Some(bytes),
                    source_identity: Some(source_identity),
                    phase: FencePhase::Initial {
                        original_existed: true,
                    },
                })
            }
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let (state_directory, snapshot) = recover_published(config_path)?;
                Ok(Self {
                    config_path: config_path.to_path_buf(),
                    snapshot,
                    source_identity: None,
                    phase: FencePhase::Published { state_directory },
                })
            }
            Ok(_) => Err(cold_error(format!(
                "{} must be a regular legacy config, a missing path, or an owned v8 fence",
                config_path.display()
            ))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                config_path: config_path.to_path_buf(),
                snapshot: None,
                source_identity: None,
                phase: FencePhase::Initial {
                    original_existed: false,
                },
            }),
            Err(error) => Err(cold_io("inspect", config_path, error)),
        }
    }

    pub(crate) fn snapshot_bytes(&self) -> Option<&[u8]> {
        self.snapshot.as_deref()
    }

    pub(crate) fn cold_roots(&self, cwd: &Path) -> MigrationResult<Vec<ColdRootEntry>> {
        super::load_cold_roots_strict(self.snapshot_bytes(), &self.config_path, cwd)
    }

    pub(crate) fn is_published(&self) -> bool {
        matches!(self.phase, FencePhase::Published { .. })
    }

    pub(crate) fn recovery_backup_path(&self) -> Option<PathBuf> {
        self.snapshot.as_ref()?;
        match &self.phase {
            FencePhase::Initial { .. } => None,
            FencePhase::Staged {
                state_directory, ..
            }
            | FencePhase::Published { state_directory } => Some(state_directory.join(BACKUP_FILE)),
        }
    }

    /// Prepare the recovery directory and staged symlink. The legacy pathname
    /// remains a regular file (or missing) until `publish`.
    pub(crate) fn preflight(&mut self) -> MigrationResult<()> {
        match &self.phase {
            FencePhase::Published { .. } => return self.verify(),
            FencePhase::Staged { .. } => return self.verify(),
            FencePhase::Initial { .. } => {}
        }

        self.verify_unpublished_source()?;
        let original_existed = matches!(
            self.phase,
            FencePhase::Initial {
                original_existed: true
            }
        );
        let (state_directory, staged_link) = self.create_stage(original_existed)?;
        self.phase = FencePhase::Staged {
            state_directory,
            staged_link,
        };
        self.verify()
    }

    /// Atomically retire the legacy pathname. Once rename succeeds the fence
    /// remains published even if durability confirmation or a later database
    /// operation fails.
    pub(crate) fn publish(&mut self) -> MigrationResult<()> {
        if matches!(self.phase, FencePhase::Initial { .. }) {
            self.preflight()?;
        }
        if matches!(self.phase, FencePhase::Published { .. }) {
            return self.verify();
        }

        self.verify()?;
        let (state_directory, staged_link) = match &self.phase {
            FencePhase::Staged {
                state_directory,
                staged_link,
            } => (state_directory.clone(), staged_link.clone()),
            FencePhase::Initial { .. } | FencePhase::Published { .. } => unreachable!(),
        };
        if self.snapshot.is_some() {
            fs::rename(&staged_link, &self.config_path).map_err(|error| {
                cold_io(
                    &format!(
                        "atomically publish cold-config fence {} ->",
                        staged_link.display()
                    ),
                    &self.config_path,
                    error,
                )
            })?;
        } else {
            // A rename-over-existing would silently discard a legacy config
            // created after our missing-path check. Direct symlink creation is
            // create-if-absent: EEXIST makes this transition fail and retry
            // from the newly written JSON instead of losing registrations.
            let target = state_directory
                .file_name()
                .ok_or_else(|| MigrationError::NonUtf8Path(state_directory.to_path_buf()))?;
            #[cfg(unix)]
            symlink(Path::new(target), &self.config_path).map_err(|error| {
                cold_io(
                    "atomically create missing cold-config fence",
                    &self.config_path,
                    error,
                )
            })?;
            #[cfg(not(unix))]
            return Err(MigrationError::UnsupportedPlatform(
                "cold-config symlink fence is supported only on Unix".to_owned(),
            ));
        }
        // The canonical rename/symlink creation is the commit point. Record it
        // before any fallible fsync so a retry on this same object never treats
        // the legacy pathname as open.
        self.phase = FencePhase::Published {
            state_directory: state_directory.clone(),
        };
        let confirmation = (|| {
            sync_parent(&self.config_path)?;
            if self.snapshot.is_none() && fs::remove_file(&staged_link).is_ok() {
                // This link was never the commit object in the missing-source
                // branch; its cleanup is cosmetic and may not turn success
                // into a failure after the canonical fence is durable.
                let _ = sync_parent(&staged_link);
            }
            self.sync_recovery_state()?;
            self.verify()
        })();
        confirmation.map_err(|error| MigrationError::ColdFencePublished {
            config: self.config_path.clone(),
            recovery: state_directory,
            detail: error.to_string(),
        })
    }

    /// Revalidate the final symlink and recovery bytes. This is deliberately
    /// non-destructive: the recovery directory and backup survive success.
    pub(crate) fn complete(&self) -> MigrationResult<()> {
        self.sync_recovery_state()?;
        self.verify()
    }

    pub(crate) fn verify(&self) -> MigrationResult<()> {
        match &self.phase {
            FencePhase::Initial { .. } => self.verify_unpublished_source(),
            FencePhase::Staged {
                state_directory,
                staged_link,
            } => {
                self.verify_unpublished_source()?;
                validate_staged_link(staged_link, state_directory)?;
                if self.snapshot.is_some() {
                    require_same_inode(&self.config_path, &state_directory.join(BACKUP_FILE))?;
                }
                validate_state(&self.config_path, state_directory, self.snapshot.as_deref())
            }
            FencePhase::Published { state_directory } => {
                let (actual_directory, actual_snapshot) = recover_published(&self.config_path)?;
                require(
                    &actual_directory == state_directory,
                    format!(
                        "published cold-config fence target changed from {} to {}",
                        state_directory.display(),
                        actual_directory.display()
                    ),
                )?;
                require(
                    actual_snapshot.as_deref() == self.snapshot.as_deref(),
                    format!(
                        "cold-config recovery snapshot changed during the database transition at {}",
                        self.config_path.display()
                    ),
                )
            }
        }
    }

    fn verify_unpublished_source(&self) -> MigrationResult<()> {
        match self.snapshot.as_deref() {
            Some(expected) => {
                let actual = read_stable_regular_file(&self.config_path)?;
                require(
                    actual == expected,
                    format!(
                        "legacy cold config {} changed before its fence was published",
                        self.config_path.display()
                    ),
                )?;
                let actual_identity = file_identity(&self.config_path)?;
                require(
                    Some(actual_identity) == self.source_identity,
                    format!(
                        "legacy cold config {} changed inode before its fence was published",
                        self.config_path.display()
                    ),
                )
            }
            None => match fs::symlink_metadata(&self.config_path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Ok(_) => Err(cold_error(format!(
                    "legacy cold config {} appeared before its fence was published",
                    self.config_path.display()
                ))),
                Err(error) => Err(cold_io("reinspect", &self.config_path, error)),
            },
        }
    }

    fn create_stage(&self, original_existed: bool) -> MigrationResult<(PathBuf, PathBuf)> {
        let parent = self.config_path.parent().ok_or_else(|| {
            cold_error(format!(
                "cold config {} has no parent directory",
                self.config_path.display()
            ))
        })?;
        let parent_metadata = fs::metadata(parent)
            .map_err(|error| cold_io("inspect parent of", &self.config_path, error))?;
        require(
            parent_metadata.is_dir(),
            format!("cold config parent {} is not a directory", parent.display()),
        )?;
        let config_file_name = config_name(&self.config_path)?;

        let (state_directory, staged_link, state_name) = loop {
            let nonce = fence_nonce();
            let state_name = format!("{config_file_name}{STATE_SUFFIX}{nonce}");
            let state_directory = parent.join(&state_name);
            let staged_link = parent.join(format!("{config_file_name}{STAGED_SUFFIX}{nonce}"));
            let mut builder = DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            match builder.create(&state_directory) {
                Ok(()) => {
                    #[cfg(unix)]
                    fs::set_permissions(&state_directory, fs::Permissions::from_mode(0o700))
                        .map_err(|error| {
                            cold_io(
                                "set private permissions on recovery directory for",
                                &self.config_path,
                                error,
                            )
                        })?;
                    break (state_directory, staged_link, state_name);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(cold_io(
                        "create recovery directory for",
                        &self.config_path,
                        error,
                    ));
                }
            }
        };
        sync_parent(&state_directory)?;

        let backup = state_directory.join(BACKUP_FILE);
        if original_existed {
            fs::hard_link(&self.config_path, &backup).map_err(|error| {
                cold_io("create recovery hard-link for", &self.config_path, error)
            })?;
            let backup_bytes = read_stable_regular_file(&backup)?;
            require(
                Some(backup_bytes.as_slice()) == self.snapshot.as_deref(),
                format!(
                    "legacy cold config {} changed while its recovery backup was created",
                    self.config_path.display()
                ),
            )?;
            require_same_inode(&self.config_path, &backup)?;
            sync_file(&backup)?;
        }

        let marker = FenceMarker {
            format_version: FENCE_FORMAT_VERSION,
            config_name: config_name(&self.config_path)?.to_owned(),
            state_directory: state_name.clone(),
            original_existed,
            initial_digest: self.snapshot.as_deref().map(digest),
        };
        let marker_path = state_directory.join(MARKER_FILE);
        write_marker(&marker_path, &marker)?;
        sync_directory(&state_directory)?;

        #[cfg(unix)]
        symlink(Path::new(&state_name), &staged_link)
            .map_err(|error| cold_io("create staged symlink for", &self.config_path, error))?;
        #[cfg(not(unix))]
        return Err(MigrationError::UnsupportedPlatform(
            "cold-config symlink fence is supported only on Unix".to_owned(),
        ));
        sync_parent(&staged_link)?;
        Ok((state_directory, staged_link))
    }

    fn sync_recovery_state(&self) -> MigrationResult<()> {
        let state_directory = match &self.phase {
            FencePhase::Initial { .. } => return Ok(()),
            FencePhase::Staged {
                state_directory, ..
            }
            | FencePhase::Published { state_directory } => state_directory,
        };
        if let Some(backup) = self.recovery_backup_path() {
            sync_file(&backup)?;
        }
        sync_directory(state_directory)
    }
}

impl LegacyCutover for ColdConfigFence {
    fn preflight(&mut self) -> Result<(), String> {
        ColdConfigFence::preflight(self).map_err(|error| error.to_string())
    }

    fn publish(&mut self) -> Result<(), String> {
        ColdConfigFence::publish(self).map_err(|error| error.to_string())
    }

    fn complete(&mut self) -> Result<(), String> {
        ColdConfigFence::complete(self).map_err(|error| error.to_string())
    }
}

fn recover_published(config_path: &Path) -> MigrationResult<(PathBuf, Option<Vec<u8>>)> {
    let target = fs::read_link(config_path)
        .map_err(|error| cold_io("read published fence", config_path, error))?;
    let target_name = single_normal_component(&target).ok_or_else(|| {
        cold_error(format!(
            "published cold-config fence {} must use a single relative target component",
            config_path.display()
        ))
    })?;
    let expected_prefix = format!("{}{}", config_name(config_path)?, STATE_SUFFIX);
    require(
        target_name.starts_with(&expected_prefix),
        format!(
            "cold-config symlink {} is not an owned v8 fence",
            config_path.display()
        ),
    )?;
    let parent = config_path.parent().ok_or_else(|| {
        cold_error(format!(
            "cold config {} has no parent directory",
            config_path.display()
        ))
    })?;
    let state_directory = parent.join(target_name);
    let snapshot = read_and_validate_state(config_path, &state_directory)?;
    Ok((state_directory, snapshot))
}

fn read_and_validate_state(
    config_path: &Path,
    state_directory: &Path,
) -> MigrationResult<Option<Vec<u8>>> {
    let metadata = fs::symlink_metadata(state_directory)
        .map_err(|error| cold_io("inspect fence state for", config_path, error))?;
    require(
        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
        format!(
            "cold-config fence state {} is not a real directory",
            state_directory.display()
        ),
    )?;
    require_private_directory(state_directory, &metadata)?;
    let marker_path = state_directory.join(MARKER_FILE);
    let marker_bytes = read_stable_regular_file(&marker_path)?;
    let marker: FenceMarker = serde_json::from_slice(&marker_bytes).map_err(|error| {
        cold_error(format!(
            "cannot parse cold-config fence marker {}: {error}",
            marker_path.display()
        ))
    })?;
    require(
        marker.format_version == FENCE_FORMAT_VERSION,
        format!(
            "cold-config fence marker {} has unsupported version {}",
            marker_path.display(),
            marker.format_version
        ),
    )?;
    require(
        marker.config_name == config_name(config_path)?,
        format!(
            "cold-config fence marker {} belongs to a different config file",
            marker_path.display()
        ),
    )?;
    let actual_state_name = state_directory
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| MigrationError::NonUtf8Path(state_directory.to_path_buf()))?;
    require(
        marker.state_directory == actual_state_name,
        format!(
            "cold-config fence marker {} names the wrong state directory",
            marker_path.display()
        ),
    )?;

    let backup = state_directory.join(BACKUP_FILE);
    if marker.original_existed {
        let bytes = read_stable_regular_file(&backup)?;
        // `initial_digest` records the pre-fence snapshot for audit. A legacy
        // writer that opened the old inode before rename may legitimately
        // update the hard-link backup afterward; recovery must use those newer
        // complete bytes rather than discarding the registration.
        require(
            marker.initial_digest.is_some(),
            format!(
                "cold-config fence marker {} omits its initial digest",
                marker_path.display()
            ),
        )?;
        Ok(Some(bytes))
    } else {
        require(
            marker.initial_digest.is_none(),
            format!(
                "cold-config fence marker {} claims a digest for a missing legacy config",
                marker_path.display()
            ),
        )?;
        match fs::symlink_metadata(&backup) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Ok(_) => Err(cold_error(format!(
                "cold-config fence {} unexpectedly contains a recovery backup",
                state_directory.display()
            ))),
            Err(error) => Err(cold_io("inspect recovery backup", &backup, error)),
        }
    }
}

fn validate_state(
    config_path: &Path,
    state_directory: &Path,
    expected_snapshot: Option<&[u8]>,
) -> MigrationResult<()> {
    let actual = read_and_validate_state(config_path, state_directory)?;
    require(
        actual.as_deref() == expected_snapshot,
        format!(
            "cold-config recovery snapshot changed while staging {}",
            config_path.display()
        ),
    )
}

fn validate_staged_link(staged_link: &Path, state_directory: &Path) -> MigrationResult<()> {
    let metadata = fs::symlink_metadata(staged_link)
        .map_err(|error| cold_io("inspect staged cold-config fence", staged_link, error))?;
    require(
        metadata.file_type().is_symlink(),
        format!(
            "staged cold-config fence {} is not a symlink",
            staged_link.display()
        ),
    )?;
    let target = fs::read_link(staged_link)
        .map_err(|error| cold_io("read staged cold-config fence", staged_link, error))?;
    let expected = state_directory
        .file_name()
        .ok_or_else(|| MigrationError::NonUtf8Path(state_directory.to_path_buf()))?;
    require(
        target == Path::new(expected),
        format!(
            "staged cold-config fence {} points to the wrong recovery directory",
            staged_link.display()
        ),
    )
}

fn read_stable_regular_file(path: &Path) -> MigrationResult<Vec<u8>> {
    let path_before = fs::symlink_metadata(path)
        .map_err(|error| cold_io("inspect regular file pathname", path, error))?;
    require(
        path_before.file_type().is_file() && !path_before.file_type().is_symlink(),
        format!("{} is not a real regular file", path.display()),
    )?;
    let mut file = File::open(path).map_err(|error| cold_io("open regular file", path, error))?;
    let before = file
        .metadata()
        .map_err(|error| cold_io("inspect open regular file", path, error))?;
    require(
        before.file_type().is_file() && !before.file_type().is_symlink(),
        format!("{} is not a regular file", path.display()),
    )?;
    require_same_metadata_identity(path, &path_before, &before)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| cold_io("read", path, error))?;
    let after = file
        .metadata()
        .map_err(|error| cold_io("reinspect open regular file", path, error))?;
    require(
        after.file_type().is_file() && !after.file_type().is_symlink(),
        format!("{} stopped being a regular file while read", path.display()),
    )?;
    require_same_metadata_identity(path, &before, &after)?;
    require_metadata_unchanged(path, &before, &after)?;
    let path_metadata = fs::symlink_metadata(path)
        .map_err(|error| cold_io("reinspect regular file pathname", path, error))?;
    require(
        path_metadata.file_type().is_file() && !path_metadata.file_type().is_symlink(),
        format!(
            "{} stopped being a real regular file while read",
            path.display()
        ),
    )?;
    require_same_metadata_identity(path, &after, &path_metadata)?;
    Ok(bytes)
}

fn require_same_inode(left: &Path, right: &Path) -> MigrationResult<()> {
    let left_metadata = fs::symlink_metadata(left)
        .map_err(|error| cold_io("inspect hard-link source", left, error))?;
    let right_metadata = fs::symlink_metadata(right)
        .map_err(|error| cold_io("inspect recovery hard-link", right, error))?;
    #[cfg(unix)]
    return require(
        left_metadata.dev() == right_metadata.dev() && left_metadata.ino() == right_metadata.ino(),
        format!(
            "cold-config recovery backup {} is not linked to {}",
            right.display(),
            left.display()
        ),
    );
    #[cfg(not(unix))]
    {
        let _ = (left_metadata, right_metadata);
        Err(MigrationError::UnsupportedPlatform(
            "cold-config hard-link verification is supported only on Unix".to_owned(),
        ))
    }
}

fn file_identity(path: &Path) -> MigrationResult<(u64, u64)> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| cold_io("inspect file identity", path, error))?;
    require(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        format!("{} is not a real regular file", path.display()),
    )?;
    #[cfg(unix)]
    return Ok((metadata.dev(), metadata.ino()));
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Err(MigrationError::UnsupportedPlatform(
            "cold-config file identity is supported only on Unix".to_owned(),
        ))
    }
}

fn require_same_metadata_identity(
    path: &Path,
    before: &fs::Metadata,
    after: &fs::Metadata,
) -> MigrationResult<()> {
    #[cfg(unix)]
    return require(
        before.dev() == after.dev() && before.ino() == after.ino(),
        format!("{} changed identity while it was read", path.display()),
    );
    #[cfg(not(unix))]
    {
        let _ = (path, before, after);
        Err(MigrationError::UnsupportedPlatform(
            "cold-config identity verification is supported only on Unix".to_owned(),
        ))
    }
}

fn require_metadata_unchanged(
    path: &Path,
    before: &fs::Metadata,
    after: &fs::Metadata,
) -> MigrationResult<()> {
    #[cfg(unix)]
    return require(
        before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec(),
        format!("{} changed while it was read", path.display()),
    );
    #[cfg(not(unix))]
    {
        let _ = (path, before, after);
        Err(MigrationError::UnsupportedPlatform(
            "cold-config change verification is supported only on Unix".to_owned(),
        ))
    }
}

fn write_marker(path: &Path, marker: &FenceMarker) -> MigrationResult<()> {
    let mut bytes = serde_json::to_vec(marker)
        .map_err(|error| cold_error(format!("encode cold-config fence marker: {error}")))?;
    bytes.push(b'\n');
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|error| cold_io("create fence marker", path, error))?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| cold_io("set private fence-marker permissions", path, error))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| cold_io("write fence marker", path, error))?;
    Ok(())
}

fn sync_directory(path: &Path) -> MigrationResult<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| cold_io("fsync directory", path, error))
}

fn require_private_directory(path: &Path, metadata: &fs::Metadata) -> MigrationResult<()> {
    #[cfg(unix)]
    {
        require(
            metadata.permissions().mode() & 0o077 == 0,
            format!(
                "cold-config recovery directory {} is accessible to group or other users",
                path.display()
            ),
        )
    }
    #[cfg(not(unix))]
    {
        let _ = (path, metadata);
        Err(MigrationError::UnsupportedPlatform(
            "cold-config recovery permission checks are supported only on Unix".to_owned(),
        ))
    }
}

fn single_normal_component(path: &Path) -> Option<&str> {
    let mut components = path.components();
    let Component::Normal(value) = components.next()? else {
        return None;
    };
    if components.next().is_some() {
        return None;
    }
    value.to_str()
}

fn require_config_components(path: &Path) -> MigrationResult<()> {
    path.parent().ok_or_else(|| {
        cold_error(format!(
            "cold config {} has no parent directory",
            path.display()
        ))
    })?;
    config_name(path).map(|_| ())
}

fn config_name(path: &Path) -> MigrationResult<&str> {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MigrationError::NonUtf8Path(path.to_path_buf()))
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn fence_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = FENCE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{}-{sequence}", process::id())
}

fn require(condition: bool, detail: String) -> MigrationResult<()> {
    if condition {
        Ok(())
    } else {
        Err(cold_error(detail))
    }
}

fn cold_error(detail: String) -> MigrationError {
    MigrationError::ColdConfig(detail)
}

fn cold_io(operation: &str, path: &Path, error: std::io::Error) -> MigrationError {
    cold_error(format!("{operation} {}: {error}", path.display()))
}

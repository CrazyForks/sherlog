use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(all(unix, not(target_os = "linux")))]
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const LOCK_SUFFIX: &str = ".sync.lock";
const DEFAULT_WAIT: Duration = Duration::from_secs(10);
const DEFAULT_POLL: Duration = Duration::from_millis(100);
static CLAIM_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LockInfo {
    pub pid: u32,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ObservedLock {
    Directory(LockInfo),
    EmptyDirectory,
    LegacyFile(LockInfo),
    MalformedLegacyFile,
}

#[derive(Debug, Error)]
pub enum SyncLockError {
    #[error("sync already running: {owner} ({path})")]
    Timeout { path: PathBuf, owner: String },
    #[error("prepare sync lock directory {path:?}: {source}")]
    Prepare {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("access sync lock {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Legacy-compatible app-level writer lock.
///
/// The path and JSON payload match the TypeScript writer. New writers fully
/// persist a private claim and atomically hard-link it into the canonical
/// regular-file path. The regular file makes an older TypeScript writer's
/// `mkdir` fail atomically, avoiding the empty-directory acquisition window,
/// while this implementation still observes and safely reclaims old directory
/// locks.
#[derive(Debug)]
pub(crate) struct SyncLock {
    path: PathBuf,
    owner: LockInfo,
}

impl SyncLock {
    pub(crate) fn acquire(db_path: &Path) -> Result<Self, SyncLockError> {
        Self::acquire_with(db_path, DEFAULT_WAIT, DEFAULT_POLL)
    }

    fn acquire_with(db_path: &Path, wait: Duration, poll: Duration) -> Result<Self, SyncLockError> {
        let path = lock_path(db_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| SyncLockError::Prepare {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let owner = LockInfo {
            pid: std::process::id(),
            created_at: now_timestamp(),
        };
        let claim = ClaimFile::create(&path, &owner)?;
        let deadline = Instant::now() + wait;

        loop {
            match fs::hard_link(&claim.path, &path) {
                Ok(()) => return Ok(Self { path, owner }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(SyncLockError::Io {
                        path: path.clone(),
                        source,
                    });
                }
            }

            let observed = observe(&path)?;
            let reclaimed = match observed.as_ref() {
                None => true,
                Some(ObservedLock::EmptyDirectory) => fs::remove_dir(&path).is_ok(),
                Some(ObservedLock::Directory(info)) if !process_alive(info.pid) => {
                    remove_directory_lock_if_same(&path, info)
                }
                Some(ObservedLock::LegacyFile(info)) if !process_alive(info.pid) => {
                    remove_legacy_lock_if_same(&path, info)
                }
                _ => false,
            };
            if reclaimed {
                continue;
            }

            if Instant::now() >= deadline {
                let owner = match observed {
                    Some(ObservedLock::Directory(info) | ObservedLock::LegacyFile(info)) => {
                        format!("pid {} since {}", info.pid, info.created_at)
                    }
                    _ => "unknown owner".to_owned(),
                };
                return Err(SyncLockError::Timeout { path, owner });
            }
            thread::sleep(poll);
        }
    }
}

struct ClaimFile {
    path: PathBuf,
}

impl ClaimFile {
    fn create(lock_path: &Path, owner: &LockInfo) -> Result<Self, SyncLockError> {
        let encoded = serde_json::to_vec(owner).map_err(|error| SyncLockError::Io {
            path: lock_path.to_path_buf(),
            source: io::Error::other(error),
        })?;
        loop {
            let sequence = CLAIM_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let mut name = lock_path
                .file_name()
                .map_or_else(|| OsString::from("sync.lock"), OsString::from);
            name.push(format!(".claim.{}.{}", owner.pid, sequence));
            let claim_path = lock_path.with_file_name(name);
            let mut file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&claim_path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(SyncLockError::Io {
                        path: claim_path,
                        source,
                    });
                }
            };
            if let Err(source) = file.write_all(&encoded).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(&claim_path);
                return Err(SyncLockError::Io {
                    path: claim_path,
                    source,
                });
            }
            return Ok(Self { path: claim_path });
        }
    }
}

impl Drop for ClaimFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        match observe(&self.path) {
            Ok(Some(ObservedLock::Directory(info))) if info == self.owner => {
                let _ = remove_directory_lock_if_same(&self.path, &self.owner);
            }
            Ok(Some(ObservedLock::LegacyFile(info))) if info == self.owner => {
                let _ = remove_legacy_lock_if_same(&self.path, &self.owner);
            }
            _ => {}
        }
    }
}

pub(crate) fn lock_path(db_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}{}", db_path.to_string_lossy(), LOCK_SUFFIX))
}

fn observe(path: &Path) -> Result<Option<ObservedLock>, SyncLockError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(SyncLockError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !metadata.is_dir() {
        let mut raw = String::new();
        match File::open(path).and_then(|mut file| file.read_to_string(&mut raw)) {
            Ok(_) => match serde_json::from_str::<LockInfo>(&raw) {
                Ok(info) => return Ok(Some(ObservedLock::LegacyFile(info))),
                Err(_) => return Ok(Some(ObservedLock::MalformedLegacyFile)),
            },
            Err(source) => {
                return Err(SyncLockError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
    }

    let entries = fs::read_dir(path).map_err(|source| SyncLockError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| SyncLockError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.ends_with(".json") {
            continue;
        }
        let mut raw = String::new();
        if File::open(entry.path())
            .and_then(|mut file| file.read_to_string(&mut raw))
            .is_ok()
            && let Ok(info) = serde_json::from_str::<LockInfo>(&raw)
        {
            return Ok(Some(ObservedLock::Directory(info)));
        }
        if let Some(info) = info_from_file_name(name) {
            return Ok(Some(ObservedLock::Directory(info)));
        }
    }
    Ok(Some(ObservedLock::EmptyDirectory))
}

fn remove_directory_lock_if_same(path: &Path, expected: &LockInfo) -> bool {
    let info_path = path.join(info_file_name(expected));
    match fs::remove_file(&info_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return false,
    }
    match fs::remove_dir(path) {
        Ok(()) => true,
        Err(error) => error.kind() == io::ErrorKind::NotFound,
    }
}

fn remove_legacy_lock_if_same(path: &Path, expected: &LockInfo) -> bool {
    match observe(path) {
        Ok(Some(ObservedLock::LegacyFile(current))) if current == *expected => {
            match fs::remove_file(path) {
                Ok(()) => true,
                Err(error) => error.kind() == io::ErrorKind::NotFound,
            }
        }
        Ok(None) => true,
        _ => false,
    }
}

fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
    #[cfg(not(unix))]
    {
        // Conservative on platforms without a safe std-only process probe:
        // never steal a lock whose liveness cannot be established.
        let _ = pid;
        true
    }
}

fn info_file_name(info: &LockInfo) -> String {
    let millis = timestamp_millis(&info.created_at).unwrap_or_else(now_millis);
    format!("{}-{millis}.json", info.pid)
}

fn info_from_file_name(name: &str) -> Option<LockInfo> {
    let stem = name.strip_suffix(".json")?;
    let (pid, millis) = stem.split_once('-')?;
    let pid = pid.parse::<u32>().ok().filter(|pid| *pid > 0)?;
    let millis = millis.parse::<i64>().ok().filter(|value| *value >= 0)?;
    let created_at = jiff::Timestamp::from_millisecond(millis).ok()?.to_string();
    Some(LockInfo { pid, created_at })
}

fn timestamp_millis(value: &str) -> Option<i64> {
    Some(value.parse::<jiff::Timestamp>().ok()?.as_millisecond())
}

fn now_timestamp() -> String {
    jiff::Timestamp::now().to_string()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn lock_path_matches_the_legacy_writer() {
        assert_eq!(
            lock_path(Path::new("/tmp/index.sqlite")),
            PathBuf::from("/tmp/index.sqlite.sync.lock")
        );
    }

    #[test]
    fn a_second_writer_times_out_without_stealing_the_live_lock() {
        let temp = tempdir().unwrap();
        let db = temp.path().join("index.sqlite");
        let first =
            SyncLock::acquire_with(&db, Duration::from_millis(50), Duration::from_millis(2))
                .unwrap();
        let error =
            SyncLock::acquire_with(&db, Duration::from_millis(20), Duration::from_millis(2))
                .unwrap_err();
        assert!(matches!(error, SyncLockError::Timeout { .. }));
        assert!(lock_path(&db).is_file());
        drop(first);
        assert!(!lock_path(&db).exists());
    }

    #[test]
    fn atomically_replaces_an_abandoned_empty_directory_with_a_regular_file_claim() {
        let temp = tempdir().unwrap();
        let db = temp.path().join("index.sqlite");
        let path = lock_path(&db);
        fs::create_dir(&path).unwrap();

        let lock =
            SyncLock::acquire_with(&db, Duration::from_millis(100), Duration::from_millis(2))
                .unwrap();

        assert!(path.is_file());
        assert_eq!(
            observe(&path).unwrap(),
            Some(ObservedLock::LegacyFile(lock.owner.clone()))
        );
        assert!(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path.join("contender.json"))
                .is_err()
        );
        drop(lock);
        assert!(!path.exists());
    }

    #[test]
    fn reclaims_a_stale_directory_lock_with_a_partially_written_payload() {
        let temp = tempdir().unwrap();
        let db = temp.path().join("index.sqlite");
        let path = lock_path(&db);
        fs::create_dir(&path).unwrap();
        let stale = LockInfo {
            pid: 999_999,
            created_at: "2026-04-27T00:00:00Z".to_owned(),
        };
        fs::write(path.join(info_file_name(&stale)), "{partial").unwrap();

        let lock =
            SyncLock::acquire_with(&db, Duration::from_millis(100), Duration::from_millis(2))
                .unwrap();
        let observed = observe(&path).unwrap();
        assert_eq!(observed, Some(ObservedLock::LegacyFile(lock.owner.clone())));
    }

    #[test]
    fn reclaims_a_parsed_stale_legacy_file_but_not_a_malformed_one() {
        let temp = tempdir().unwrap();
        let db = temp.path().join("index.sqlite");
        let path = lock_path(&db);
        let stale = LockInfo {
            pid: 999_999,
            created_at: "2026-04-27T00:00:00Z".to_owned(),
        };
        fs::write(&path, serde_json::to_vec(&stale).unwrap()).unwrap();
        let lock =
            SyncLock::acquire_with(&db, Duration::from_millis(100), Duration::from_millis(2))
                .unwrap();
        drop(lock);

        fs::write(&path, "{malformed").unwrap();
        let error =
            SyncLock::acquire_with(&db, Duration::from_millis(15), Duration::from_millis(2))
                .unwrap_err();
        assert!(matches!(error, SyncLockError::Timeout { .. }));
        assert!(path.is_file());
    }

    #[test]
    fn owner_filename_round_trips_even_if_json_is_torn() {
        let info = LockInfo {
            pid: 42,
            created_at: "2026-04-27T00:00:00Z".to_owned(),
        };
        let decoded = info_from_file_name(&info_file_name(&info)).unwrap();
        assert_eq!(decoded.pid, info.pid);
        assert_eq!(
            timestamp_millis(&decoded.created_at),
            timestamp_millis(&info.created_at)
        );
    }
}

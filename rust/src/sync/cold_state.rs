use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::resolve_lexical;
use crate::identity::SourceId;
use crate::index::{ColdRoot, IndexError, IndexLayout, IndexReader, IndexWriter};

use super::cutover::LegacyCutover;
use super::lock::SyncLock;
use super::{
    committed_cutover_error, publish_scratch_index, remove_sqlite_scratch, scratch_index_path,
};

/// A legacy/config registration waiting to become part of the v8 index.
///
/// A first scratch sync imports every source in one transaction. This matters
/// because the legacy JSON can contain registrations for sources other than
/// the selector that happened to create the shared v8 database.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingColdRoot {
    pub source_id: SourceId,
    pub root: PathBuf,
    pub added_at: String,
}

impl PendingColdRoot {
    pub fn new(source_id: SourceId, root: impl Into<PathBuf>, added_at: impl Into<String>) -> Self {
        Self {
            source_id,
            root: root.into(),
            added_at: added_at.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredColdRoot {
    pub source_id: SourceId,
    pub root: String,
    pub added_at: String,
}

impl From<ColdRoot> for RegisteredColdRoot {
    fn from(value: ColdRoot) -> Self {
        Self {
            source_id: value.source_id,
            root: value.root,
            added_at: value.added_at,
        }
    }
}

/// Result shared by idempotent add/remove operations.
///
/// Add always returns `Some(entry)`. Remove returns the prior entry when it
/// existed, otherwise `changed=false, entry=None`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColdRootMutation {
    pub changed: bool,
    pub entry: Option<RegisteredColdRoot>,
}

#[derive(Debug, Error)]
pub enum SyncStateError {
    #[error("sync state is unavailable because the index does not exist: {db_path}")]
    IndexUnavailable { db_path: PathBuf },

    #[error("sync state requires an explicit v8 migration: {db_path}")]
    IndexSchemaUpgradeRequired { db_path: PathBuf },

    #[error("invalid cold root {root}: {message}")]
    InvalidColdRoot { root: PathBuf, message: String },

    #[error("acquire writer lock for {db_path}: {message}")]
    WriterLock { db_path: PathBuf, message: String },

    #[error("update sync state in {db_path}: {message}")]
    IndexFailure { db_path: PathBuf, message: String },

    #[error("publish legacy state fence for {db_path}: {message}")]
    LegacyCutover { db_path: PathBuf, message: String },
}

impl SyncStateError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::IndexUnavailable { .. } => "index_unavailable",
            Self::IndexSchemaUpgradeRequired { .. } => "index_schema_upgrade_required",
            Self::InvalidColdRoot { .. } => "invalid_cold_root",
            Self::WriterLock { .. } => "sync_lock_unavailable",
            Self::IndexFailure { .. } => "index_error",
            Self::LegacyCutover { .. } => "legacy_cutover_failed",
        }
    }
}

/// Register a cold root in an existing v8 index.
///
/// This operation never creates an index. It participates in the exact same
/// legacy-compatible writer lock as sync and migration-facing writers.
pub fn add_cold_root(
    db_path: &Path,
    source_id: SourceId,
    root: &Path,
    added_at: &str,
    cwd: &Path,
) -> Result<ColdRootMutation, SyncStateError> {
    let _lock = acquire_writer_lock(db_path)?;
    let existing = read_registered_roots(db_path, Some(source_id))?;
    let root = validate_add_root(root, cwd)?;
    let root_text = path_text(&root)?;
    let existing = existing.into_iter().find(|entry| entry.root == root_text);
    let added_at_write = existing.is_none().then_some(added_at);
    let entry = existing.clone().unwrap_or_else(|| RegisteredColdRoot {
        source_id,
        root: root_text.clone(),
        added_at: added_at.to_owned(),
    });

    let mut writer = open_writer(db_path)?;
    let mut transaction = writer
        .begin()
        .map_err(|error| index_failure(db_path, error))?;
    transaction
        .upsert_cold_root(source_id, &root_text, added_at_write)
        .map_err(|error| index_failure(db_path, error))?;
    transaction
        .commit()
        .map_err(|error| index_failure(db_path, error))?;

    Ok(ColdRootMutation {
        changed: existing.is_none(),
        entry: Some(entry),
    })
}

/// Remove a cold root from an existing v8 index without touching its retained
/// projection. A later explicit prune decides whether that projection remains.
pub fn remove_cold_root(
    db_path: &Path,
    source_id: SourceId,
    root: &Path,
    cwd: &Path,
) -> Result<ColdRootMutation, SyncStateError> {
    let _lock = acquire_writer_lock(db_path)?;
    let existing = read_registered_roots(db_path, Some(source_id))?;
    let root = resolve_lexical(root, cwd);
    let root_text = path_text(&root)?;
    let entry = existing.into_iter().find(|entry| entry.root == root_text);

    let mut writer = open_writer(db_path)?;
    let mut transaction = writer
        .begin()
        .map_err(|error| index_failure(db_path, error))?;
    let changed = transaction
        .remove_cold_root(source_id, &root_text)
        .map_err(|error| index_failure(db_path, error))?;
    transaction
        .commit()
        .map_err(|error| index_failure(db_path, error))?;

    Ok(ColdRootMutation { changed, entry })
}

/// Register a cold root while atomically retiring optional legacy JSON state.
///
/// When no database exists this creates a metadata-only v8 index, imports all
/// recovered source registrations, and applies the add in one transaction.
/// `bootstrap=None` means there is no legacy state to import; a caller
/// recovering a previously published fence must pass the durable backup as
/// `Some(entries)`.
pub fn add_cold_root_with_cutover(
    db_path: &Path,
    source_id: SourceId,
    root: &Path,
    added_at: &str,
    cwd: &Path,
    bootstrap: Option<&[PendingColdRoot]>,
    cutover: &mut impl LegacyCutover,
) -> Result<ColdRootMutation, SyncStateError> {
    let _lock = acquire_writer_lock(db_path)?;
    let root = validate_add_root(root, cwd)?;
    let root_text = path_text(&root)?;
    mutate_with_cutover(db_path, bootstrap, cutover, |existing, transaction| {
        let prior = existing
            .iter()
            .find(|entry| entry.source_id == source_id && entry.root == root_text)
            .cloned();
        let entry = prior.clone().unwrap_or_else(|| RegisteredColdRoot {
            source_id,
            root: root_text.clone(),
            added_at: added_at.to_owned(),
        });
        transaction
            .upsert_cold_root(source_id, &root_text, prior.is_none().then_some(added_at))
            .map_err(|error| index_failure(db_path, error))?;
        Ok(ColdRootMutation {
            changed: prior.is_none(),
            entry: Some(entry),
        })
    })
}

/// Remove a cold root while atomically retiring optional legacy JSON state.
///
/// With no v8 and `bootstrap=Some(...)`, this creates metadata-only v8 state,
/// imports every recovered registration, removes the target, and publishes the
/// legacy fence. With neither v8 nor legacy state (`bootstrap=None`) it returns
/// `changed=false` without creating a database.
pub fn remove_cold_root_with_cutover(
    db_path: &Path,
    source_id: SourceId,
    root: &Path,
    cwd: &Path,
    bootstrap: Option<&[PendingColdRoot]>,
    cutover: &mut impl LegacyCutover,
) -> Result<ColdRootMutation, SyncStateError> {
    let _lock = acquire_writer_lock(db_path)?;
    let root = resolve_lexical(root, cwd);
    let root_text = path_text(&root)?;
    if !db_path.exists() && bootstrap.is_none() {
        return Ok(ColdRootMutation {
            changed: false,
            entry: None,
        });
    }
    mutate_with_cutover(db_path, bootstrap, cutover, |existing, transaction| {
        let entry = existing
            .iter()
            .find(|entry| entry.source_id == source_id && entry.root == root_text)
            .cloned();
        let changed = transaction
            .remove_cold_root(source_id, &root_text)
            .map_err(|error| index_failure(db_path, error))?;
        Ok(ColdRootMutation { changed, entry })
    })
}

fn mutate_with_cutover(
    db_path: &Path,
    bootstrap: Option<&[PendingColdRoot]>,
    cutover: &mut impl LegacyCutover,
    mutation: impl FnOnce(
        &[RegisteredColdRoot],
        &mut crate::index::IndexTransaction<'_>,
    ) -> Result<ColdRootMutation, SyncStateError>,
) -> Result<ColdRootMutation, SyncStateError> {
    let active_exists = db_path.exists();
    let pending = if active_exists {
        Vec::new()
    } else {
        normalize_pending_roots(bootstrap.unwrap_or_default())?
    };
    let existing = if active_exists {
        read_registered_roots(db_path, None)?
    } else {
        pending.clone()
    };
    cutover
        .preflight()
        .map_err(|message| legacy_cutover_failure(db_path, message))?;

    let scratch = (!active_exists).then(|| scratch_index_path(db_path));
    let write_path = scratch.as_deref().unwrap_or(db_path);
    let mut writer = if active_exists {
        open_writer(write_path)?
    } else {
        IndexWriter::create_v8(write_path).map_err(|error| index_failure(db_path, error))?
    };
    let transaction_result = (|| {
        let mut transaction = writer
            .begin()
            .map_err(|error| index_failure(db_path, error))?;
        for entry in &pending {
            transaction
                .upsert_cold_root(entry.source_id, &entry.root, Some(&entry.added_at))
                .map_err(|error| index_failure(db_path, error))?;
        }
        let result = mutation(&existing, &mut transaction)?;
        cutover
            .publish()
            .map_err(|message| legacy_cutover_failure(db_path, message))?;
        transaction
            .commit()
            .map_err(|error| index_failure(db_path, error))?;
        Ok::<ColdRootMutation, SyncStateError>(result)
    })();
    drop(writer);

    let result = match transaction_result {
        Ok(result) => result,
        Err(error) => {
            if let Some(path) = scratch.as_deref() {
                remove_sqlite_scratch(path);
            }
            return Err(error);
        }
    };
    let publish_error = scratch
        .as_deref()
        .and_then(|path| publish_scratch_index(path, db_path).err());
    let complete_error = cutover.complete().err();
    if let Some(message) = committed_cutover_error(publish_error, complete_error) {
        return Err(legacy_cutover_failure(db_path, message));
    }
    Ok(result)
}

/// Read the authoritative v8 registrations without taking the writer lock.
/// SQLite gives the caller a consistent snapshot before or after a concurrent
/// mutation; the command never falls back to legacy JSON once v8 exists.
pub fn list_cold_roots(
    db_path: &Path,
    source_id: Option<SourceId>,
) -> Result<Vec<RegisteredColdRoot>, SyncStateError> {
    read_registered_roots(db_path, source_id)
}

pub(super) fn normalize_pending_roots(
    entries: &[PendingColdRoot],
) -> Result<Vec<RegisteredColdRoot>, SyncStateError> {
    let mut roots = std::collections::BTreeMap::<(SourceId, String), RegisteredColdRoot>::new();
    for entry in entries {
        if !entry.root.is_absolute() {
            return Err(SyncStateError::InvalidColdRoot {
                root: entry.root.clone(),
                message: "pending cold root must be absolute".to_owned(),
            });
        }
        let root = path_text(&entry.root)?;
        if entry.added_at.trim().is_empty() {
            return Err(SyncStateError::InvalidColdRoot {
                root: entry.root.clone(),
                message: "added_at must be non-empty".to_owned(),
            });
        }
        roots
            .entry((entry.source_id, root.clone()))
            .or_insert_with(|| RegisteredColdRoot {
                source_id: entry.source_id,
                root,
                added_at: entry.added_at.clone(),
            });
    }
    Ok(roots.into_values().collect())
}

fn acquire_writer_lock(db_path: &Path) -> Result<SyncLock, SyncStateError> {
    SyncLock::acquire(db_path).map_err(|error| SyncStateError::WriterLock {
        db_path: db_path.to_path_buf(),
        message: error.to_string(),
    })
}

fn read_registered_roots(
    db_path: &Path,
    source_id: Option<SourceId>,
) -> Result<Vec<RegisteredColdRoot>, SyncStateError> {
    if !db_path.exists() {
        return Err(SyncStateError::IndexUnavailable {
            db_path: db_path.to_path_buf(),
        });
    }
    let reader = IndexReader::open(db_path).map_err(|error| state_read_error(db_path, error))?;
    if reader.layout() != IndexLayout::V8 {
        return Err(SyncStateError::IndexSchemaUpgradeRequired {
            db_path: db_path.to_path_buf(),
        });
    }
    reader
        .cold_roots(source_id)
        .map_err(|error| state_read_error(db_path, error))
        .map(|entries| entries.into_iter().map(RegisteredColdRoot::from).collect())
}

fn open_writer(db_path: &Path) -> Result<IndexWriter, SyncStateError> {
    IndexWriter::open_v8(db_path).map_err(|error| match error {
        IndexError::NotFound(_) => SyncStateError::IndexUnavailable {
            db_path: db_path.to_path_buf(),
        },
        IndexError::UnsupportedSchema { .. } | IndexError::InvalidOperation(_) => {
            SyncStateError::IndexSchemaUpgradeRequired {
                db_path: db_path.to_path_buf(),
            }
        }
        error => index_failure(db_path, error),
    })
}

fn state_read_error(db_path: &Path, error: IndexError) -> SyncStateError {
    match error {
        IndexError::NotFound(_) => SyncStateError::IndexUnavailable {
            db_path: db_path.to_path_buf(),
        },
        IndexError::UnsupportedSchema { .. } | IndexError::InvalidOperation(_) => {
            SyncStateError::IndexSchemaUpgradeRequired {
                db_path: db_path.to_path_buf(),
            }
        }
        error => index_failure(db_path, error),
    }
}

fn index_failure(db_path: &Path, error: impl std::fmt::Display) -> SyncStateError {
    SyncStateError::IndexFailure {
        db_path: db_path.to_path_buf(),
        message: error.to_string(),
    }
}

fn legacy_cutover_failure(db_path: &Path, message: impl Into<String>) -> SyncStateError {
    SyncStateError::LegacyCutover {
        db_path: db_path.to_path_buf(),
        message: message.into(),
    }
}

fn validate_add_root(root: &Path, cwd: &Path) -> Result<PathBuf, SyncStateError> {
    let root = resolve_lexical(root, cwd);
    let metadata = fs::metadata(&root).map_err(|error| SyncStateError::InvalidColdRoot {
        root: root.clone(),
        message: if error.kind() == std::io::ErrorKind::NotFound {
            "cold root does not exist".to_owned()
        } else {
            format!("cannot inspect cold root: {error}")
        },
    })?;
    if !metadata.is_dir() {
        return Err(SyncStateError::InvalidColdRoot {
            root,
            message: "cold root is not a directory".to_owned(),
        });
    }
    Ok(root)
}

fn path_text(path: &Path) -> Result<String, SyncStateError> {
    let text = path
        .to_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| SyncStateError::InvalidColdRoot {
            root: path.to_path_buf(),
            message: "cold root path must be non-empty valid UTF-8".to_owned(),
        })?;
    Ok(text.to_owned())
}

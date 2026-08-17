use std::path::PathBuf;

use thiserror::Error;

use crate::index::IndexError;

pub type MigrationResult<T> = Result<T, MigrationError>;

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("v7 index not found: {0}")]
    NotFound(PathBuf),

    #[error("explicit migration requires a v7 index; active index is already v8: {0}")]
    AlreadyV8(PathBuf),

    #[error("unsupported source index at {path}: {detail}")]
    UnsupportedSource { path: PathBuf, detail: String },

    #[error("migration artifact already exists and was not overwritten: {0}")]
    ArtifactExists(PathBuf),

    #[error("another writer owns the legacy sync lock: {0}")]
    LockBusy(PathBuf),

    #[error("v7 index contains invalid projection data: {0}")]
    InvalidV7(String),

    #[error("v8 copy verification failed: {0}")]
    Verification(String),

    #[error("migration input changed while the copy was being built: {0}")]
    SourceChanged(String),

    #[error("migration publish failed: {0}")]
    Publish(String),

    #[error("migration is unsupported on this platform: {0}")]
    UnsupportedPlatform(String),

    #[error(
        "v8 was atomically published at {active}, but durability could not be confirmed; verified v7 backup remains at {backup}: {detail}"
    )]
    PublishedButDurabilityUnknown {
        active: PathBuf,
        backup: PathBuf,
        detail: String,
    },

    #[error("migration path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),

    #[error("index error: {0}")]
    Index(#[from] IndexError),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("cold-root configuration error: {0}")]
    ColdConfig(String),

    #[error(
        "legacy cold-root path was atomically fenced at {config}, but durability or recovery confirmation failed; the fence must remain in place and recovery state is retained at {recovery}: {detail}"
    )]
    ColdFencePublished {
        config: PathBuf,
        recovery: PathBuf,
        detail: String,
    },

    #[cfg(test)]
    #[error("injected migration failure at {0}")]
    Injected(&'static str),
}

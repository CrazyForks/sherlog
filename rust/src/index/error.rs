use std::path::PathBuf;

use thiserror::Error;

pub type IndexResult<T> = Result<T, IndexError>;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("index not found: {0}")]
    NotFound(PathBuf),

    #[error("index schema is unsupported (user_version={user_version}): {detail}")]
    UnsupportedSchema { user_version: i32, detail: String },

    #[error("index contains invalid data: {0}")]
    InvalidData(String),

    #[error("session not found in index: {0}")]
    SessionNotFound(String),

    #[error("index invariant check failed: {0}")]
    Invariant(String),

    #[error("refusing to overwrite an existing scratch index: {0}")]
    ScratchExists(PathBuf),

    #[error("invalid index operation: {0}")]
    InvalidOperation(String),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("selector JSON is invalid: {0}")]
    SelectorJson(#[from] serde_json::Error),
}

impl IndexError {
    pub(crate) fn unsupported(user_version: i32, detail: impl Into<String>) -> Self {
        Self::UnsupportedSchema {
            user_version,
            detail: detail.into(),
        }
    }
}

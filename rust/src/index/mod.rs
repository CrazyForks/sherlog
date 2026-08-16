//! Concrete SQLite lexical index.
//!
//! This module is deliberately the only place that knows physical table
//! names, FTS rowids, schema generations, and v7 compatibility SQL.  Callers
//! use the concrete [`IndexReader`] and [`IndexWriter`]; there is no repository
//! trait because SQLite is the only implementation.

mod error;
mod reader;
mod schema;
mod types;
mod writer;

pub use error::{IndexError, IndexResult};
pub use reader::IndexReader;
pub use types::{
    CandidateEvidence, ColdRoot, CommitReceipt, CoverageWrite, DocumentKind, IndexLayout,
    IndexMetadata, InvariantReport, MessageWrite, PruneOutcome, RecallOrder, RecallSpec,
    SelectorCounts, SessionBundle, SessionProfileWrite, SessionWrite, SourceFileState,
    StoredDocument, StoredSession,
};
pub use writer::{IndexTransaction, IndexWriter};

pub const SCHEMA_VERSION: i32 = 8;
pub const PROJECTION_EPOCH: i64 = 1;
pub const ANALYZER_EPOCH: i64 = 1;
pub const COVERAGE_EPOCH: i64 = 1;

#[cfg(test)]
mod tests;

use crate::identity::{SessionIdentity, SessionRef, SourceId};
use crate::model::MessageRole;
use crate::selector::Selector;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexLayout {
    V7,
    V8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentKind {
    Message,
    SessionProfile,
}

impl DocumentKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::SessionProfile => "session_profile",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "message" => Some(Self::Message),
            "session_profile" => Some(Self::SessionProfile),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexMetadata {
    pub schema_version: i32,
    pub projection_epoch: i64,
    pub analyzer_epoch: i64,
    pub coverage_epoch: i64,
    pub index_version: String,
    pub created_at: String,
    pub upgraded_at: Option<String>,
    pub migration_receipt: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionWrite {
    pub identity: SessionIdentity,
    pub session_uuid: String,
    pub file_path: String,
    pub source_root: String,
    pub title: String,
    pub summary_text: String,
    pub compact_text: String,
    pub reasoning_summary_text: String,
    pub cwd: String,
    pub model: String,
    pub started_at: String,
    pub ended_at: String,
    pub path_date: String,
    pub raw_file_mtime: i64,
    pub raw_file_size: u64,
    pub index_version: String,
}

impl SessionWrite {
    pub fn profile(&self) -> SessionProfileWrite {
        SessionProfileWrite {
            title_text: self.title.clone(),
            summary_text: self.summary_text.clone(),
            compact_text: self.compact_text.clone(),
            reasoning_text: self.reasoning_summary_text.clone(),
            raw_start: None,
            raw_end: None,
            projection_epoch: super::PROJECTION_EPOCH,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionProfileWrite {
    pub title_text: String,
    pub summary_text: String,
    pub compact_text: String,
    pub reasoning_text: String,
    pub raw_start: Option<u64>,
    pub raw_end: Option<u64>,
    pub projection_epoch: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageWrite {
    pub seq: i64,
    pub role: MessageRole,
    pub timestamp: String,
    pub source_kind: String,
    pub body_text: String,
    pub raw_start: Option<u64>,
    pub raw_end: Option<u64>,
    pub projection_epoch: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceFileState {
    pub source_id: SourceId,
    pub file_path: String,
    pub source_root: String,
    pub source_generation: String,
    pub mtime_ms: f64,
    /// Exact filesystem modification time when known. Legacy v7 imports keep
    /// this NULL so cache/unchanged checks fail safe until the next sync.
    pub mtime_ns: Option<i64>,
    pub size: u64,
    pub indexed_bytes: u64,
    pub head_digest: String,
    pub boundary_digest: String,
    pub next_seq: i64,
    pub reducer_checkpoint: Option<Vec<u8>>,
    pub cwd: String,
    pub path_date: Option<String>,
    pub extra_fingerprint: String,
    pub projection_epoch: i64,
    pub analyzer_epoch: i64,
    pub coverage_epoch: i64,
    pub session: Option<SessionIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColdRoot {
    pub source_id: SourceId,
    pub root: String,
    pub added_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageWrite {
    pub selector: Selector,
    pub source_fingerprint: String,
    pub source_file_set_fingerprint: String,
    pub source_file_count: u64,
    pub indexed_session_count: u64,
    pub indexed_document_count: u64,
    pub source_generation: String,
    pub completed_at: Option<String>,
    pub index_version: String,
    pub projection_epoch: i64,
    pub analyzer_epoch: i64,
    pub coverage_epoch: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSession {
    pub id: i64,
    pub identity: SessionIdentity,
    pub session_uuid: String,
    pub file_path: String,
    pub source_root: String,
    pub title: String,
    pub summary_text: String,
    pub compact_text: String,
    pub reasoning_summary_text: String,
    pub cwd: String,
    pub model: String,
    pub started_at: String,
    pub ended_at: String,
    pub path_date: String,
    pub message_count: u64,
    pub document_count: u64,
    pub raw_file_mtime: i64,
    pub raw_file_size: u64,
    pub index_version: String,
    pub updated_at: String,
}

impl StoredSession {
    pub fn as_write(&self) -> SessionWrite {
        SessionWrite {
            identity: self.identity.clone(),
            session_uuid: self.session_uuid.clone(),
            file_path: self.file_path.clone(),
            source_root: self.source_root.clone(),
            title: self.title.clone(),
            summary_text: self.summary_text.clone(),
            compact_text: self.compact_text.clone(),
            reasoning_summary_text: self.reasoning_summary_text.clone(),
            cwd: self.cwd.clone(),
            model: self.model.clone(),
            started_at: self.started_at.clone(),
            ended_at: self.ended_at.clone(),
            path_date: self.path_date.clone(),
            raw_file_mtime: self.raw_file_mtime,
            raw_file_size: self.raw_file_size,
            index_version: self.index_version.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredDocument {
    pub id: Option<i64>,
    pub kind: DocumentKind,
    pub seq: Option<i64>,
    pub role: Option<MessageRole>,
    pub timestamp: Option<String>,
    pub source_kind: Option<String>,
    pub body_text: String,
    pub title_text: String,
    pub summary_text: String,
    pub compact_text: String,
    pub reasoning_text: String,
    pub raw_start: Option<u64>,
    pub raw_end: Option<u64>,
    pub projection_epoch: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionBundle {
    pub session: StoredSession,
    pub documents: Vec<StoredDocument>,
    pub source_files: Vec<SourceFileState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecallOrder {
    Relevance,
    Ended,
    Started,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecallSpec {
    /// Already analyzed, distinct terms.  The index owns FTS escaping and AND
    /// construction, while retrieval owns query analysis and fallback policy.
    pub terms: Vec<String>,
    /// Literal substring fallback for a non-empty query that produced no FTS
    /// tokens (notably one-scalar CJK). Mutually exclusive with `terms`.
    pub like_needle: Option<String>,
    pub sources: Vec<SourceId>,
    /// Optional exact session scope used by progressive `read-range --query`.
    /// This stays in the index boundary so a common query cannot exhaust a
    /// global candidate limit before reaching the requested session.
    pub session: Option<SessionRef>,
    pub selector: Option<Selector>,
    pub excluded_session_uuids: Vec<String>,
    pub order: RecallOrder,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateEvidence {
    pub document_id: Option<i64>,
    pub session_id: i64,
    pub source_id: SourceId,
    pub session_key: String,
    pub session_uuid: String,
    pub title: String,
    pub summary_text: String,
    pub compact_text: String,
    pub reasoning_summary_text: String,
    pub cwd: String,
    pub started_at: String,
    pub ended_at: String,
    pub session_message_count: u64,
    pub kind: DocumentKind,
    pub seq: Option<i64>,
    pub role: Option<MessageRole>,
    pub timestamp: Option<String>,
    pub body_text: String,
    pub raw_start: Option<u64>,
    pub raw_end: Option<u64>,
    /// Raw FTS5 bm25 polarity: lower is better. Retrieval performs product
    /// scoring and session aggregation.
    pub fts_score: f64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InvariantReport {
    pub session_count: u64,
    pub message_document_count: u64,
    pub profile_document_count: u64,
    pub fts_row_count: u64,
    pub source_file_count: u64,
    pub coverage_count: u64,
    pub violations: Vec<String>,
}

impl InvariantReport {
    pub fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitReceipt {
    pub committed_at: String,
    pub invariants: InvariantReport,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SelectorCounts {
    pub session_count: u64,
    pub message_document_count: u64,
    pub document_count: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PruneOutcome {
    pub removed: u64,
    pub retained_cold: u64,
}

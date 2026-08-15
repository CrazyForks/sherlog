//! Pure retrieval policy shared by SQLite-backed and future document-backed stores.
//!
//! Storage code is responsible only for producing [`CandidateEvidence`] rows.
//! Everything after that point (query planning, snippets, ranking, global merge,
//! progressive-read hints, and zero-result guidance) stays deterministic here.

mod candidate;
mod elision;
mod evidence;
mod global;
mod query;
mod ranking;
mod read;
mod snippet;
mod utf16;
mod zero;

pub use candidate::{CandidateEvidence, SessionFieldTexts, matched_session_fields};
pub use elision::{DEFAULT_MAX_MESSAGE_CHARS, ElisionOptions, elide_messages};
pub use evidence::{EvidenceReadAction, EvidenceReadReason, build_evidence_read_action};
pub use global::{
    FindSourceSelection, merge_find_results, merge_find_summaries, public_find_sources,
};
pub use query::{
    LEGACY_SESSION_FTS_WEIGHTS, QueryAnalysis, RecallMode, analyze_query, build_fts_match,
    escape_like_pattern, quote_fts_term,
};
pub use ranking::{
    RetrievalPlan, rank_candidates, rank_candidates_at, rank_candidates_for_sort,
    rank_candidates_for_sort_at,
};
pub use read::{ReadAnchorError, resolve_read_anchor};
pub use snippet::{make_like_snippet, make_raw_snippet};
pub use zero::{
    ZeroResultRefinement, build_relaxed_recall_queries, build_zero_result_diagnosis,
    build_zero_result_refinement, build_zero_results_next_action,
};

#[cfg(test)]
mod golden_tests;

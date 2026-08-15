use crate::model::FindResult;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("read-range requires an explicit sessionRef plus either --seq or --query")]
pub struct ReadAnchorError;

/// Resolve the pure policy portion of a read-range anchor.
///
/// The store performs the in-session message search and supplies its top hit.
/// A session-level/no-hit query falls back to sequence zero, matching the
/// progressive-read contract instead of failing the read.
pub fn resolve_read_anchor(
    explicit_seq: Option<i64>,
    query: Option<&str>,
    top_hit: Option<&FindResult>,
) -> Result<i64, ReadAnchorError> {
    if let Some(seq) = explicit_seq {
        return Ok(seq);
    }
    if query.is_some_and(|query| !query.is_empty()) {
        return Ok(top_hit.and_then(|result| result.match_seq).unwrap_or(0));
    }
    Err(ReadAnchorError)
}

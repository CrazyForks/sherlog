use crate::model::FindResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReadAnchorError {
    #[error("read-range requires an explicit sessionRef plus either --seq or --query")]
    MissingAnchorSpec,
    #[error("the query matched no message in this session")]
    NoMessageHit,
}

/// Resolve the pure policy portion of a read-range anchor.
///
/// The store performs the in-session message search and supplies its top hit.
/// A session-level/profile-only match must NOT fall back to sequence zero:
/// seq 0 would present unrelated messages as if they were the matched
/// evidence. Callers convert [`ReadAnchorError::NoMessageHit`] into the typed
/// `anchor_not_found` / profile-aware error contract.
pub fn resolve_read_anchor(
    explicit_seq: Option<i64>,
    query: Option<&str>,
    top_hit: Option<&FindResult>,
) -> Result<i64, ReadAnchorError> {
    if let Some(seq) = explicit_seq {
        return Ok(seq);
    }
    if query.is_some_and(|query| !query.is_empty()) {
        return top_hit
            .and_then(|result| result.match_seq)
            .ok_or(ReadAnchorError::NoMessageHit);
    }
    Err(ReadAnchorError::MissingAnchorSpec)
}

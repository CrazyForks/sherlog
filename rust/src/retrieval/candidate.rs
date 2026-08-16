use std::borrow::Cow;

pub use crate::index::CandidateEvidence;
use crate::index::DocumentKind;
use crate::model::{FindMatchRole, FindMatchedField, MatchSource, MessageRole};

pub(crate) fn session_group_key(candidate: &CandidateEvidence) -> String {
    format!("{}\0{}", candidate.source_id, candidate.session_key)
}

pub(crate) fn match_source(candidate: &CandidateEvidence) -> MatchSource {
    match candidate.kind {
        DocumentKind::Message => MatchSource::Message,
        DocumentKind::SessionProfile => MatchSource::Session,
    }
}

pub(crate) fn match_role(candidate: &CandidateEvidence) -> FindMatchRole {
    match (candidate.kind, candidate.role) {
        (DocumentKind::SessionProfile, _) => FindMatchRole::Session,
        (DocumentKind::Message, Some(MessageRole::User)) => FindMatchRole::User,
        // A valid message document always has a role. Treat malformed/missing
        // role as assistant here so it never receives the user-intent bonus.
        (DocumentKind::Message, Some(MessageRole::Assistant) | None) => FindMatchRole::Assistant,
    }
}

pub(crate) fn content_text(candidate: &CandidateEvidence) -> Cow<'_, str> {
    match candidate.kind {
        DocumentKind::Message => Cow::Borrowed(&candidate.body_text),
        DocumentKind::SessionProfile => Cow::Owned(format!(
            "{}\n{}\n{}\n{}",
            candidate.title,
            candidate.summary_text,
            candidate.compact_text,
            candidate.reasoning_summary_text
        )),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SessionFieldTexts<'a> {
    pub title: &'a str,
    pub summary: &'a str,
    pub compact: &'a str,
    pub reasoning_summary: &'a str,
}

/// Best-effort provenance for a session-level document match.
pub fn matched_session_fields(
    fields: SessionFieldTexts<'_>,
    query: &str,
    terms: &[String],
) -> Vec<FindMatchedField> {
    let normalized_query = query.trim().to_lowercase();
    [
        (FindMatchedField::Title, fields.title),
        (FindMatchedField::Summary, fields.summary),
        (FindMatchedField::Compact, fields.compact),
        (FindMatchedField::ReasoningSummary, fields.reasoning_summary),
    ]
    .into_iter()
    .filter_map(|(field, text)| {
        if text.is_empty() {
            return None;
        }
        let lower = text.to_lowercase();
        let term_hit = terms.iter().any(|term| lower.contains(term));
        let phrase_hit = !normalized_query.is_empty() && lower.contains(&normalized_query);
        (term_hit || phrase_hit).then_some(field)
    })
    .collect()
}

pub(crate) fn matched_fields(
    candidate: &CandidateEvidence,
    query: &str,
    terms: &[String],
) -> Vec<FindMatchedField> {
    if candidate.kind == DocumentKind::Message {
        return vec![FindMatchedField::Message];
    }
    matched_session_fields(
        SessionFieldTexts {
            title: &candidate.title,
            summary: &candidate.summary_text,
            compact: &candidate.compact_text,
            reasoning_summary: &candidate.reasoning_summary_text,
        },
        query,
        terms,
    )
}

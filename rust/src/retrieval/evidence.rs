use serde::{Deserialize, Serialize};

use crate::identity::SourceId;
use crate::model::FindResult;

const DEFAULT_READ_RANGE_BEFORE: usize = 2;
const DEFAULT_READ_RANGE_AFTER: usize = 2;
const DEFAULT_SESSION_PAGE_OFFSET: usize = 0;
const DEFAULT_SESSION_PAGE_LIMIT: usize = 40;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceReadReason {
    MessageMatch,
    SessionLevelMatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum EvidenceReadAction {
    ReadRange {
        reason: EvidenceReadReason,
        source_id: SourceId,
        session_ref: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        before: usize,
        after: usize,
        argv: Vec<String>,
    },
    ReadPage {
        reason: EvidenceReadReason,
        source_id: SourceId,
        session_ref: String,
        offset: usize,
        limit: usize,
        argv: Vec<String>,
    },
}

pub fn build_evidence_read_action(result: &FindResult, query: Option<&str>) -> EvidenceReadAction {
    if result.match_seq.is_none() {
        if let Some(query) = query.filter(|query| !query.is_empty()) {
            return EvidenceReadAction::ReadRange {
                reason: EvidenceReadReason::SessionLevelMatch,
                source_id: result.source_id,
                session_ref: result.session_ref.clone(),
                seq: None,
                query: Some(query.to_owned()),
                before: DEFAULT_READ_RANGE_BEFORE,
                after: DEFAULT_READ_RANGE_AFTER,
                argv: vec![
                    "shlog".to_owned(),
                    "read-range".to_owned(),
                    result.session_ref.clone(),
                    "--query".to_owned(),
                    query.to_owned(),
                    "--before".to_owned(),
                    DEFAULT_READ_RANGE_BEFORE.to_string(),
                    "--after".to_owned(),
                    DEFAULT_READ_RANGE_AFTER.to_string(),
                ],
            };
        }
        return EvidenceReadAction::ReadPage {
            reason: EvidenceReadReason::SessionLevelMatch,
            source_id: result.source_id,
            session_ref: result.session_ref.clone(),
            offset: DEFAULT_SESSION_PAGE_OFFSET,
            limit: DEFAULT_SESSION_PAGE_LIMIT,
            argv: vec![
                "shlog".to_owned(),
                "read-page".to_owned(),
                result.session_ref.clone(),
                "--offset".to_owned(),
                DEFAULT_SESSION_PAGE_OFFSET.to_string(),
                "--limit".to_owned(),
                DEFAULT_SESSION_PAGE_LIMIT.to_string(),
            ],
        };
    }

    let seq = result.match_seq.expect("checked above");
    let mut argv = vec![
        "shlog".to_owned(),
        "read-range".to_owned(),
        result.session_ref.clone(),
        "--seq".to_owned(),
        seq.to_string(),
        "--before".to_owned(),
        DEFAULT_READ_RANGE_BEFORE.to_string(),
        "--after".to_owned(),
        DEFAULT_READ_RANGE_AFTER.to_string(),
    ];
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        argv.push("--query".to_owned());
        argv.push(query.to_owned());
    }
    EvidenceReadAction::ReadRange {
        reason: EvidenceReadReason::MessageMatch,
        source_id: result.source_id,
        session_ref: result.session_ref.clone(),
        seq: Some(seq),
        query: query.filter(|query| !query.is_empty()).map(str::to_owned),
        before: DEFAULT_READ_RANGE_BEFORE,
        after: DEFAULT_READ_RANGE_AFTER,
        argv,
    }
}

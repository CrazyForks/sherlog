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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceReadSideEffect {
    ReadIndex,
}

/// Invocation context the evidence action must close over. The agent is
/// expected to execute `executable + args` verbatim, so the exact DB path,
/// source qualifier, and output mode that produced the candidate are all
/// preserved.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceReadCommand {
    /// `"inherit"` means: reuse the binary that emitted this payload (or its
    /// documented prefix) instead of resolving `shlog` from PATH.
    pub executable: String,
    /// Arguments WITHOUT the program name.
    pub args: Vec<String>,
    pub side_effect: EvidenceReadSideEffect,
}

impl EvidenceReadCommand {
    fn new(args: Vec<String>) -> Self {
        Self {
            executable: "inherit".to_owned(),
            args,
            side_effect: EvidenceReadSideEffect::ReadIndex,
        }
    }
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
        command: EvidenceReadCommand,
    },
    ReadPage {
        reason: EvidenceReadReason,
        source_id: SourceId,
        session_ref: String,
        offset: usize,
        limit: usize,
        command: EvidenceReadCommand,
    },
}

/// Closure context for a find invocation. Only these inputs decide the exact
/// command a later `read-*` call must reproduce.
pub struct EvidenceReadContext<'a> {
    pub db_path: &'a str,
    pub json: bool,
}

pub fn build_evidence_read_action(
    result: &FindResult,
    query: Option<&str>,
    context: &EvidenceReadContext<'_>,
) -> EvidenceReadAction {
    let mut scope = vec![
        "--source".to_owned(),
        result.source_id.as_str().to_owned(),
        "--db".to_owned(),
        context.db_path.to_owned(),
    ];
    if context.json {
        scope.push("--json".to_owned());
    }
    if result.match_seq.is_none() {
        if let Some(query) = query.filter(|query| !query.is_empty()) {
            let mut args = vec![
                "read-range".to_owned(),
                result.session_ref.clone(),
                "--query".to_owned(),
                query.to_owned(),
                "--before".to_owned(),
                DEFAULT_READ_RANGE_BEFORE.to_string(),
                "--after".to_owned(),
                DEFAULT_READ_RANGE_AFTER.to_string(),
            ];
            args.extend(scope);
            return EvidenceReadAction::ReadRange {
                reason: EvidenceReadReason::SessionLevelMatch,
                source_id: result.source_id,
                session_ref: result.session_ref.clone(),
                seq: None,
                query: Some(query.to_owned()),
                before: DEFAULT_READ_RANGE_BEFORE,
                after: DEFAULT_READ_RANGE_AFTER,
                command: EvidenceReadCommand::new(args),
            };
        }
        let mut args = vec![
            "read-page".to_owned(),
            result.session_ref.clone(),
            "--offset".to_owned(),
            DEFAULT_SESSION_PAGE_OFFSET.to_string(),
            "--limit".to_owned(),
            DEFAULT_SESSION_PAGE_LIMIT.to_string(),
        ];
        args.extend(scope);
        return EvidenceReadAction::ReadPage {
            reason: EvidenceReadReason::SessionLevelMatch,
            source_id: result.source_id,
            session_ref: result.session_ref.clone(),
            offset: DEFAULT_SESSION_PAGE_OFFSET,
            limit: DEFAULT_SESSION_PAGE_LIMIT,
            command: EvidenceReadCommand::new(args),
        };
    }

    let seq = result.match_seq.expect("checked above");
    let mut args = vec![
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
        args.push("--query".to_owned());
        args.push(query.to_owned());
    }
    args.extend(scope);
    EvidenceReadAction::ReadRange {
        reason: EvidenceReadReason::MessageMatch,
        source_id: result.source_id,
        session_ref: result.session_ref.clone(),
        seq: Some(seq),
        query: query.filter(|query| !query.is_empty()).map(str::to_owned),
        before: DEFAULT_READ_RANGE_BEFORE,
        after: DEFAULT_READ_RANGE_AFTER,
        command: EvidenceReadCommand::new(args),
    }
}

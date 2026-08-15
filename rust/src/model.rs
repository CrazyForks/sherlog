//! Public data contracts shared by CLI, storage, and source adapters.
//!
//! These are intentionally behavior-free serde models. Database rows,
//! parsers, and query execution will be implemented in later migration stages.

use serde::{Deserialize, Serialize};

use crate::identity::SourceId;
use crate::selector::Selector;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchSource {
    Message,
    Session,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FindMatchRole {
    User,
    Assistant,
    Session,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    EventMsg,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedMessage {
    pub role: MessageRole,
    pub content_text: String,
    pub timestamp: String,
    pub seq: i64,
    pub source_kind: SourceKind,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedSession {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<SourceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
    pub session_uuid: String,
    pub file_path: String,
    pub title: String,
    pub summary_text: String,
    pub compact_text: String,
    pub reasoning_summary_text: String,
    pub cwd: String,
    pub model: String,
    pub started_at: String,
    pub ended_at: String,
    pub messages: Vec<ParsedMessage>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReadProof {
    pub byte_count: u64,
    pub content_fingerprint: String,
    pub opened_mtime_ms: f64,
    pub opened_size: u64,
    pub completed_mtime_ms: f64,
    pub completed_size: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum ParseSessionResult {
    Parsed {
        session: Box<ParsedSession>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_read: Option<SourceReadProof>,
    },
    Filtered {
        #[serde(skip_serializing_if = "Option::is_none")]
        source_read: Option<SourceReadProof>,
    },
    Skipped {
        #[serde(skip_serializing_if = "Option::is_none")]
        source_read: Option<SourceReadProof>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncErrorDetail {
    pub file_path: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DateRange {
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInventoryCwdGroup {
    pub cwd: String,
    pub file_count: u64,
    pub path_date_range: DateRange,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInventory {
    pub root: String,
    pub total_files: u64,
    pub path_date_range: DateRange,
    pub cwd_groups: Vec<SourceInventoryCwdGroup>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFileMeta {
    pub file_path: String,
    pub path_date: Option<String>,
    pub cwd: String,
    pub mtime_ms: f64,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedSourceFileMeta {
    pub cwd: String,
    pub path_date: Option<String>,
    pub extra_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSnapshot {
    pub selector: Selector,
    pub fingerprint: String,
    pub file_set_fingerprint: String,
    pub file_count: u64,
    pub files: Vec<SourceFileMeta>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageRecord {
    pub id: i64,
    pub selector: Selector,
    pub source_fingerprint: String,
    pub source_file_set_fingerprint: String,
    pub source_file_count: u64,
    pub indexed_session_count: u64,
    pub completed_at: String,
    pub index_version: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InventoryFreshness {
    Fresh,
    Stale,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryStaleReason {
    None,
    SourceContentChanged,
    SourceSetChanged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageInventoryStatus {
    #[serde(flatten)]
    pub record: CoverageRecord,
    pub freshness: InventoryFreshness,
    pub stale_reason: InventoryStaleReason,
    pub advisory: bool,
    pub current_source_fingerprint: String,
    pub current_source_file_set_fingerprint: String,
    pub current_source_file_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedAction {
    Query,
    Sync,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageWriteStaleReason {
    SourceContentChanged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageWriteSummary {
    pub written: bool,
    pub selector: Selector,
    pub source_fingerprint: String,
    pub source_file_set_fingerprint: String,
    pub source_file_count: u64,
    pub indexed_session_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<CoverageWriteStaleReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_action: Option<RecommendedAction>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageFreshness {
    Fresh,
    Stale,
    Missing,
    NotChecked,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStaleReason {
    None,
    Missing,
    SourceContentChanged,
    SourceSetChanged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageStatus {
    pub requested: Option<Selector>,
    pub complete: bool,
    pub freshness: CoverageFreshness,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<CoverageStaleReason>,
    pub covering_selectors: Vec<CoverageRecord>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryNextActionKind {
    CheckCoverageThenRetry,
    ChooseSelectorThenCheckCoverage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryNextActionReason {
    ZeroResultsWithUnconfirmedSelectorCoverage,
    ZeroResultsWithoutSelector,
    StaleOrMissingCoverage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryNextActionCommand {
    pub label: String,
    pub recommended: bool,
    pub argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<Selector>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryNextAction {
    pub kind: QueryNextActionKind,
    pub reason: QueryNextActionReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<Selector>,
    pub steps: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<QueryNextActionCommand>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestedCoverageFreshness {
    Fresh,
    Stale,
    Missing,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestedCoverageStatus {
    pub requested: Selector,
    pub complete: bool,
    pub freshness: RequestedCoverageFreshness,
    pub stale_reason: CoverageStaleReason,
    pub source_fingerprint: String,
    pub source_file_set_fingerprint: String,
    pub source_file_count: u64,
    pub covering_selectors: Vec<CoverageInventoryStatus>,
    pub recommended_action: RecommendedAction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: i64,
    pub source_id: SourceId,
    pub native_session_id: String,
    pub session_key: String,
    pub session_uuid: String,
    pub file_path: String,
    pub source_root: String,
    pub title: String,
    pub summary_text: String,
    pub cwd: String,
    pub model: String,
    pub started_at: String,
    pub ended_at: String,
    pub path_date: String,
    pub message_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageElisionStrategy {
    HeadTail,
    AroundQuery,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageElision {
    pub original_char_count: u64,
    pub displayed_char_count: u64,
    pub omitted_char_count: u64,
    pub strategy: MessageElisionStrategy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    pub hint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    pub session_uuid: String,
    pub seq: i64,
    pub role: MessageRole,
    pub content_text: String,
    pub timestamp: String,
    pub source_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elision: Option<MessageElision>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FindMatchedField {
    Message,
    Title,
    Summary,
    Compact,
    ReasoningSummary,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindResult {
    pub rank: u64,
    pub source_id: SourceId,
    pub session_uuid: String,
    pub session_ref: String,
    pub title: String,
    pub summary_text: String,
    pub cwd: String,
    pub started_at: String,
    pub ended_at: String,
    pub match_count: u64,
    pub match_source: MatchSource,
    pub match_seq: Option<i64>,
    pub match_role: FindMatchRole,
    pub match_timestamp: Option<String>,
    pub score: f64,
    pub snippet: String,
    pub matched_fields: Vec<FindMatchedField>,
    pub session_message_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroResultsReason {
    FreshMiss,
    StaleOrMissingCoverage,
    CoverageNotConfirmed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZeroResultsDiagnosis {
    pub reason: ZeroResultsReason,
    pub over_constrained: bool,
    pub suggested_queries: Vec<String>,
    pub hints: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FindSort {
    Relevance,
    Ended,
    Started,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCoverageStatus {
    pub source_id: SourceId,
    pub coverage: CoverageStatus,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindSummary {
    pub query: String,
    pub source_ids: Vec<SourceId>,
    pub sort: FindSort,
    pub excluded_sessions: Vec<String>,
    pub results: Vec<FindResult>,
    pub scanned_message_count: u64,
    pub coverage: CoverageStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_by_source: Option<Vec<SourceCoverageStatus>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<QueryNextAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zero_results: Option<ZeroResultsDiagnosis>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub scanned: u64,
    pub added: u64,
    pub updated: u64,
    pub skipped: u64,
    pub filtered: u64,
    pub removed: u64,
    pub retained_cold: u64,
    pub errors: u64,
    pub error_details: Vec<SyncErrorDetail>,
    pub selector: Selector,
    pub coverage: CoverageWriteSummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListEntry {
    pub session_uuid: String,
    pub title: String,
    pub summary_text: String,
    pub cwd: String,
    pub started_at: String,
    pub ended_at: String,
    pub path_date: String,
    pub message_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionListSort {
    Ended,
    Started,
    Messages,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<SourceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<Selector>,
    pub sort: SessionListSort,
    pub limit: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListSummary {
    pub query: SessionListQuery,
    pub results: Vec<SessionListEntry>,
    pub coverage: CoverageStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<QueryNextAction>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CwdCount {
    pub cwd: String,
    pub count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSummary {
    pub session_count: u64,
    pub message_count: u64,
    pub earliest_started_at: Option<String>,
    pub latest_ended_at: Option<String>,
    pub top_cwds: Vec<CwdCount>,
    pub index_version: String,
    pub db_path: String,
    pub db_size_bytes: u64,
    pub last_sync_at: Option<String>,
    pub coverage: Vec<CoverageRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusContext {
    pub cwd: String,
    pub root: String,
    pub db_path: String,
    pub index_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusIndex {
    pub exists: bool,
    pub session_count: u64,
    pub message_count: u64,
    pub earliest_started_at: Option<String>,
    pub latest_ended_at: Option<String>,
    pub db_size_bytes: u64,
    pub last_sync_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSummary {
    pub context: StatusContext,
    pub source_inventory: SourceInventory,
    pub index: StatusIndex,
    pub coverage_count: u64,
    pub coverage: Vec<CoverageInventoryStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_coverage: Option<RequestedCoverageStatus>,
}

/// Shared read payload shape. Freshness is deliberately not inferred here;
/// entries are stored coverage records that contain the selected session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadCoverage {
    pub entries: Vec<CoverageRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadRangeSummary {
    pub session: SessionRecord,
    pub anchor_seq: i64,
    pub range_start_seq: i64,
    pub range_end_seq: i64,
    pub messages: Vec<MessageRecord>,
    pub coverage: ReadCoverage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadPageSummary {
    pub session: SessionRecord,
    pub offset: u64,
    pub limit: u64,
    pub total_count: u64,
    pub has_more: bool,
    pub messages: Vec<MessageRecord>,
    pub coverage: ReadCoverage,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn all_selector() -> Selector {
        Selector::All {
            source: SourceId::Codex,
            root: "/sessions".to_owned(),
        }
    }

    fn empty_coverage() -> CoverageStatus {
        CoverageStatus {
            requested: None,
            complete: false,
            freshness: CoverageFreshness::NotChecked,
            stale_reason: None,
            covering_selectors: vec![],
        }
    }

    #[test]
    fn parsed_session_golden_uses_camel_case_and_omits_undefined_fields() {
        let session = ParsedSession {
            source_id: None,
            native_session_id: None,
            session_key: None,
            session_uuid: "uuid".to_owned(),
            file_path: "/raw/session.jsonl".to_owned(),
            title: "title".to_owned(),
            summary_text: "summary".to_owned(),
            compact_text: "compact".to_owned(),
            reasoning_summary_text: "reasoning".to_owned(),
            cwd: "/repo".to_owned(),
            model: "model".to_owned(),
            started_at: "2026-01-01T00:00:00Z".to_owned(),
            ended_at: "2026-01-01T00:01:00Z".to_owned(),
            messages: vec![ParsedMessage {
                role: MessageRole::User,
                content_text: "hello".to_owned(),
                timestamp: "2026-01-01T00:00:00Z".to_owned(),
                seq: 0,
                source_kind: SourceKind::EventMsg,
            }],
        };

        assert_eq!(
            serde_json::to_string(&session).unwrap(),
            r#"{"sessionUuid":"uuid","filePath":"/raw/session.jsonl","title":"title","summaryText":"summary","compactText":"compact","reasoningSummaryText":"reasoning","cwd":"/repo","model":"model","startedAt":"2026-01-01T00:00:00Z","endedAt":"2026-01-01T00:01:00Z","messages":[{"role":"user","contentText":"hello","timestamp":"2026-01-01T00:00:00Z","seq":0,"sourceKind":"event_msg"}]}"#
        );
    }

    #[test]
    fn coverage_null_and_optional_fields_match_json_contract() {
        let value = serde_json::to_value(empty_coverage()).unwrap();
        assert_eq!(
            value,
            json!({
                "requested": null,
                "complete": false,
                "freshness": "not_checked",
                "coveringSelectors": []
            })
        );
    }

    #[test]
    fn find_result_serializes_provenance_names_exactly() {
        let result = FindResult {
            rank: 1,
            source_id: SourceId::ClaudeCode,
            session_uuid: "compat".to_owned(),
            session_ref: "claude-code:native".to_owned(),
            title: "title".to_owned(),
            summary_text: String::new(),
            cwd: "/repo".to_owned(),
            started_at: "start".to_owned(),
            ended_at: "end".to_owned(),
            match_count: 2,
            match_source: MatchSource::Session,
            match_seq: None,
            match_role: FindMatchRole::Session,
            match_timestamp: None,
            score: 1.5,
            snippet: "hit".to_owned(),
            matched_fields: vec![FindMatchedField::Title, FindMatchedField::ReasoningSummary],
            session_message_count: 42,
        };
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["sourceId"], "claude-code");
        assert_eq!(value["matchSeq"], serde_json::Value::Null);
        assert_eq!(value["matchTimestamp"], serde_json::Value::Null);
        assert_eq!(value["matchedFields"], json!(["title", "reasoningSummary"]));
        assert_eq!(value["sessionMessageCount"], 42);
    }

    #[test]
    fn nested_status_contract_round_trips() {
        let summary = StatusSummary {
            context: StatusContext {
                cwd: "/repo".to_owned(),
                root: "/sessions".to_owned(),
                db_path: "/state/index.sqlite".to_owned(),
                index_version: "v".to_owned(),
            },
            source_inventory: SourceInventory {
                root: "/sessions".to_owned(),
                total_files: 0,
                path_date_range: DateRange {
                    from: None,
                    to: None,
                },
                cwd_groups: vec![],
            },
            index: StatusIndex {
                exists: false,
                session_count: 0,
                message_count: 0,
                earliest_started_at: None,
                latest_ended_at: None,
                db_size_bytes: 0,
                last_sync_at: None,
            },
            coverage_count: 0,
            coverage: vec![],
            requested_coverage: None,
        };
        let encoded = serde_json::to_string(&summary).unwrap();
        assert!(encoded.contains(r#""sourceInventory""#));
        assert!(encoded.contains(r#""pathDateRange""#));
        assert!(encoded.contains(r#""coverageCount""#));
        assert!(!encoded.contains("requestedCoverage"));
        assert_eq!(
            serde_json::from_str::<StatusSummary>(&encoded).unwrap(),
            summary
        );
    }

    #[test]
    fn list_query_omits_unset_filters() {
        let query = SessionListQuery {
            source_id: Some(SourceId::Pi),
            cwd: None,
            since: None,
            selector: Some(all_selector()),
            sort: SessionListSort::Messages,
            limit: 20,
        };
        assert_eq!(
            serde_json::to_value(query).unwrap(),
            json!({
                "sourceId": "pi",
                "selector": {"kind": "all", "source": "codex", "root": "/sessions"},
                "sort": "messages",
                "limit": 20
            })
        );
    }
}

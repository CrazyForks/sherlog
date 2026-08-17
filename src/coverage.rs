//! Coverage is a proof over an indexed selector, not a second content store.
//!
//! Retrieval uses [`indexed_coverage`] and therefore never touches raw logs.
//! `status` and `sync` may perform an explicit source scan and then call
//! [`evaluate_requested_coverage`] to establish freshness.

use crate::config::is_current_index_version;
use crate::identity::SourceId;
use crate::model::{
    CoverageFreshness, CoverageInventoryStatus, CoverageRecord, CoverageStaleReason,
    CoverageStatus, InventoryFreshness, InventoryStaleReason, RecommendedAction,
    RequestedCoverageFreshness, RequestedCoverageStatus, SourceSnapshot,
};
use crate::selector::{Selector, selector_implies};

/// Return the last indexed proof without inspecting the filesystem.
///
/// This is the normal `find` path: it is O(number of coverage rows), never
/// O(number of raw files). `complete` means a compatible covering sync was
/// committed; freshness is intentionally `not_checked` until status/sync
/// verifies the raw source.
pub fn indexed_coverage(
    records: &[CoverageRecord],
    requested: Option<&Selector>,
) -> CoverageStatus {
    let Some(requested) = requested else {
        return CoverageStatus {
            requested: None,
            complete: false,
            freshness: CoverageFreshness::NotChecked,
            stale_reason: None,
            covering_selectors: vec![],
        };
    };
    let covering_selectors = records
        .iter()
        .filter(|record| is_current_index_version(Some(&record.index_version)))
        .filter(|record| selector_implies(&record.selector, requested))
        .cloned()
        .collect::<Vec<_>>();

    CoverageStatus {
        requested: Some(requested.clone()),
        complete: !covering_selectors.is_empty(),
        freshness: CoverageFreshness::NotChecked,
        stale_reason: None,
        covering_selectors,
    }
}

pub fn evaluate_coverage_record(
    record: &CoverageRecord,
    snapshot: &SourceSnapshot,
) -> CoverageInventoryStatus {
    let file_set_matches = record.source_file_set_fingerprint.is_empty()
        || snapshot.file_set_fingerprint == record.source_file_set_fingerprint;
    let fresh = snapshot.fingerprint == record.source_fingerprint
        && file_set_matches
        && snapshot.file_count == record.source_file_count
        && is_current_index_version(Some(&record.index_version));
    let stale_reason = if fresh {
        InventoryStaleReason::None
    } else if !record.source_file_set_fingerprint.is_empty()
        && snapshot.file_set_fingerprint == record.source_file_set_fingerprint
    {
        InventoryStaleReason::SourceContentChanged
    } else {
        InventoryStaleReason::SourceSetChanged
    };

    CoverageInventoryStatus {
        record: record.clone(),
        freshness: if fresh {
            InventoryFreshness::Fresh
        } else {
            InventoryFreshness::Stale
        },
        stale_reason,
        advisory: is_advisory(snapshot.selector.source(), stale_reason),
        current_source_fingerprint: snapshot.fingerprint.clone(),
        current_source_file_set_fingerprint: snapshot.file_set_fingerprint.clone(),
        current_source_file_count: snapshot.file_count,
    }
}

pub fn evaluate_requested_coverage(
    snapshot: &SourceSnapshot,
    evaluated_records: &[CoverageInventoryStatus],
) -> RequestedCoverageStatus {
    let covering_selectors = evaluated_records
        .iter()
        .filter(|entry| is_current_index_version(Some(&entry.record.index_version)))
        .filter(|entry| selector_implies(&entry.record.selector, &snapshot.selector))
        .cloned()
        .collect::<Vec<_>>();
    let has_fresh = covering_selectors
        .iter()
        .any(|entry| entry.freshness == InventoryFreshness::Fresh);
    let freshness = if has_fresh {
        RequestedCoverageFreshness::Fresh
    } else if covering_selectors.is_empty() {
        RequestedCoverageFreshness::Missing
    } else {
        RequestedCoverageFreshness::Stale
    };
    let stale_reason = requested_stale_reason(freshness, &covering_selectors);

    RequestedCoverageStatus {
        requested: snapshot.selector.clone(),
        complete: freshness == RequestedCoverageFreshness::Fresh,
        freshness,
        stale_reason,
        source_fingerprint: snapshot.fingerprint.clone(),
        source_file_set_fingerprint: snapshot.file_set_fingerprint.clone(),
        source_file_count: snapshot.file_count,
        covering_selectors,
        recommended_action: if freshness == RequestedCoverageFreshness::Fresh
            || is_advisory_requested(snapshot.selector.source(), stale_reason)
        {
            RecommendedAction::Query
        } else {
            RecommendedAction::Sync
        },
    }
}

fn requested_stale_reason(
    freshness: RequestedCoverageFreshness,
    covering: &[CoverageInventoryStatus],
) -> CoverageStaleReason {
    match freshness {
        RequestedCoverageFreshness::Fresh => CoverageStaleReason::None,
        RequestedCoverageFreshness::Missing => CoverageStaleReason::Missing,
        RequestedCoverageFreshness::Stale => {
            if covering.iter().any(|entry| {
                !entry.record.source_file_set_fingerprint.is_empty()
                    && entry.current_source_file_set_fingerprint
                        == entry.record.source_file_set_fingerprint
            }) {
                CoverageStaleReason::SourceContentChanged
            } else {
                CoverageStaleReason::SourceSetChanged
            }
        }
    }
}

fn is_advisory(source: SourceId, reason: InventoryStaleReason) -> bool {
    source == SourceId::Codex && reason == InventoryStaleReason::SourceContentChanged
}

fn is_advisory_requested(source: SourceId, reason: CoverageStaleReason) -> bool {
    source == SourceId::Codex && reason == CoverageStaleReason::SourceContentChanged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::INDEX_VERSION;
    use crate::model::SourceFileMeta;

    fn selector() -> Selector {
        Selector::All {
            source: SourceId::Codex,
            root: "/sessions".to_owned(),
        }
    }

    fn record() -> CoverageRecord {
        CoverageRecord {
            id: 1,
            selector: selector(),
            source_fingerprint: "content-a".to_owned(),
            source_file_set_fingerprint: "files-a".to_owned(),
            source_file_count: 1,
            indexed_session_count: 1,
            completed_at: "2026-01-01T00:00:00Z".to_owned(),
            index_version: INDEX_VERSION.to_owned(),
        }
    }

    fn snapshot(content: &str, files: &str) -> SourceSnapshot {
        SourceSnapshot {
            selector: selector(),
            fingerprint: content.to_owned(),
            file_set_fingerprint: files.to_owned(),
            file_count: 1,
            files: vec![SourceFileMeta {
                file_path: "/sessions/a.jsonl".to_owned(),
                path_date: Some("2026-01-01".to_owned()),
                cwd: "/repo".to_owned(),
                mtime_ms: 1.25,
                size: 10,
            }],
        }
    }

    #[test]
    fn query_coverage_never_claims_live_freshness() {
        let status = indexed_coverage(&[record()], Some(&selector()));
        assert!(status.complete);
        assert_eq!(status.freshness, CoverageFreshness::NotChecked);
        assert_eq!(status.covering_selectors.len(), 1);
    }

    #[test]
    fn query_coverage_without_a_requested_selector_is_unconfirmed() {
        let status = indexed_coverage(&[record()], None);
        assert!(!status.complete);
        assert_eq!(status.requested, None);
        assert_eq!(status.freshness, CoverageFreshness::NotChecked);
        assert!(status.covering_selectors.is_empty());
    }

    #[test]
    fn explicit_verification_distinguishes_content_from_file_set_change() {
        let record = record();
        let content_changed = evaluate_coverage_record(&record, &snapshot("content-b", "files-a"));
        assert_eq!(
            content_changed.stale_reason,
            InventoryStaleReason::SourceContentChanged
        );
        assert!(content_changed.advisory);

        let set_changed = evaluate_coverage_record(&record, &snapshot("content-b", "files-b"));
        assert_eq!(
            set_changed.stale_reason,
            InventoryStaleReason::SourceSetChanged
        );
        assert!(!set_changed.advisory);
    }

    #[test]
    fn codex_content_only_change_remains_query_advisory() {
        let record = record();
        let evaluated = evaluate_coverage_record(&record, &snapshot("content-b", "files-a"));
        let requested =
            evaluate_requested_coverage(&snapshot("content-b", "files-a"), &[evaluated]);
        assert_eq!(requested.freshness, RequestedCoverageFreshness::Stale);
        assert_eq!(
            requested.stale_reason,
            CoverageStaleReason::SourceContentChanged
        );
        assert_eq!(requested.recommended_action, RecommendedAction::Query);
    }

    #[test]
    fn incompatible_epoch_is_not_covering() {
        let mut old = record();
        old.index_version = "shlog-v7-source-identity".to_owned();
        let status = indexed_coverage(&[old], Some(&selector()));
        assert!(!status.complete);
        assert!(status.covering_selectors.is_empty());
    }
}

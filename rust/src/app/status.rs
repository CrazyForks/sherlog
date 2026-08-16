use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::cli::StatusArgs;
use crate::config::{INDEX_VERSION, ResolvedPaths, resolve_lexical};
use crate::coverage::{evaluate_coverage_record, evaluate_requested_coverage};
use crate::error::AppError;
use crate::index::{
    ANALYZER_EPOCH, COVERAGE_EPOCH, IndexError, IndexLayout, IndexReader, PROJECTION_EPOCH,
    SourceFileState,
};
use crate::model::{
    CoverageInventoryStatus, CoverageRecord, CoverageStaleReason, InventoryStaleReason,
    RecommendedAction, StatusContext, StatusIndex, StatusSummary,
};
use crate::selector::{Selector, selector_implies};
use crate::sources::{
    CachedSourceMetadata, FileIdentity, ProjectionCheckpoint, SourceCatalog, SourceMetadataCache,
    SourceScan, prefix_sha256,
};

use super::map_index_error;
use super::selectors::{all_selector, status_selector, status_source};

pub(super) fn collect_status(
    args: &StatusArgs,
    paths: &ResolvedPaths,
    cwd: &Path,
) -> Result<StatusSummary, AppError> {
    let source = status_source(args)?;
    let base_selector = all_selector(source, args.root.as_deref(), paths, cwd)?;
    let requested_selector = status_selector(args, source, paths, cwd)?;
    let db_path = resolve_lexical(&args.database.db, cwd);
    let (index, records, proofs) = match IndexReader::open(&db_path) {
        Ok(reader) => {
            let stats = reader
                .stats(source)
                .map_err(|error| map_index_error(error, &db_path, cwd, paths))?;
            let proofs = if reader.layout() == IndexLayout::V8
                && reader.metadata().coverage_epoch == COVERAGE_EPOCH
            {
                let mut states = Vec::new();
                for selector in cache_selectors(
                    &base_selector,
                    requested_selector.as_ref(),
                    &stats.coverage,
                    args.inventory,
                ) {
                    states.extend(
                        reader
                            .source_files_for_selector(&selector)
                            .map_err(|error| map_index_error(error, &db_path, cwd, paths))?,
                    );
                }
                stored_file_proofs(&states)
            } else {
                HashMap::new()
            };
            (
                StatusIndex {
                    exists: true,
                    session_count: stats.session_count,
                    message_count: stats.message_count,
                    earliest_started_at: stats.earliest_started_at,
                    latest_ended_at: stats.latest_ended_at,
                    db_size_bytes: stats.db_size_bytes,
                    last_sync_at: stats.last_sync_at,
                },
                stats.coverage,
                proofs,
            )
        }
        Err(IndexError::NotFound(_)) => (empty_index_status(), Vec::new(), HashMap::new()),
        Err(error) => return Err(map_index_error(error, &db_path, cwd, paths)),
    };

    let cache =
        SourceMetadataCache::from_entries(proofs.values().map(StoredFileProof::cache_entry));
    let catalog = SourceCatalog;
    let mut scans = HashMap::<String, SourceScan>::new();
    let base_scan = scan_cached(&catalog, &cache, &base_selector, &mut scans)?;
    let mut source_inventory = base_scan.inventory.clone();
    if !args.inventory {
        source_inventory.cwd_groups.clear();
    }

    let audit_records = if args.inventory {
        records.iter().collect::<Vec<_>>()
    } else if let Some(requested) = &requested_selector {
        records
            .iter()
            .filter(|record| selector_implies(&record.selector, requested))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut evaluated = Vec::with_capacity(audit_records.len());
    for record in audit_records {
        let scan = scan_cached(&catalog, &cache, &record.selector, &mut scans)?;
        evaluated.push(evaluate_record(record, scan, &proofs)?);
    }
    let coverage = if args.inventory {
        evaluated.clone()
    } else {
        Vec::new()
    };
    let requested_coverage = if let Some(selector) = requested_selector {
        let requested_scan = scan_cached(&catalog, &cache, &selector, &mut scans)?;
        let mut status = evaluate_requested_coverage(&requested_scan.snapshot, &evaluated);
        // A same-file-set content change is only safe to keep as an advisory
        // "query anyway" when every changed file proves append-only against
        // its persisted prefix digest. Truncate, prefix rewrite, and same-size
        // rewrites break the proof and must recommend a same-scope sync.
        if status.stale_reason == CoverageStaleReason::SourceContentChanged
            && !all_changed_files_prove_append(requested_scan, &proofs)
        {
            status.recommended_action = RecommendedAction::Sync;
        }
        Some(status)
    } else {
        None
    };

    Ok(StatusSummary {
        context: StatusContext {
            cwd: cwd.to_string_lossy().into_owned(),
            root: base_selector.root().to_owned(),
            db_path: db_path.to_string_lossy().into_owned(),
            index_version: INDEX_VERSION.to_owned(),
        },
        source_inventory,
        index,
        coverage_count: records.len() as u64,
        coverage,
        requested_coverage,
    })
}

/// Per-file persisted proof surface used to classify content changes.
///
/// Only states whose epochs and checkpoint survive the same validation as the
/// sync-side metadata cache participate; everything else is treated as
/// unprovable, which is the conservative direction.
#[derive(Clone, Debug)]
struct StoredFileProof {
    source_id: crate::identity::SourceId,
    file_path: String,
    mtime_ns: i128,
    size: u64,
    identity: FileIdentity,
    path_date: Option<String>,
    cwd: String,
    accepted_fingerprint: String,
    indexed_bytes: u64,
    boundary_digest: String,
}

impl StoredFileProof {
    fn cache_entry(&self) -> CachedSourceMetadata {
        CachedSourceMetadata {
            source_id: self.source_id,
            file_path: self.file_path.clone().into(),
            mtime_ns: self.mtime_ns,
            size: self.size,
            file_identity: self.identity.clone(),
            path_date: self.path_date.clone(),
            cwd: self.cwd.clone(),
            accepted_fingerprint: self.accepted_fingerprint.clone(),
        }
    }
}

fn stored_file_proofs(states: &[SourceFileState]) -> HashMap<String, StoredFileProof> {
    states
        .iter()
        .filter_map(|state| {
            if state.projection_epoch != PROJECTION_EPOCH
                || state.analyzer_epoch != ANALYZER_EPOCH
                || state.coverage_epoch != COVERAGE_EPOCH
            {
                return None;
            }
            let checkpoint = state
                .reducer_checkpoint
                .as_deref()
                .and_then(|value| serde_json::from_slice::<ProjectionCheckpoint>(value).ok())?;
            if checkpoint.source_id != state.source_id {
                return None;
            }
            let mtime_ns = i128::from(state.mtime_ns.filter(|value| *value > 0)?);
            Some((
                state.file_path.clone(),
                StoredFileProof {
                    source_id: state.source_id,
                    file_path: state.file_path.clone(),
                    mtime_ns,
                    size: state.size,
                    identity: checkpoint.file_identity,
                    path_date: state.path_date.clone(),
                    cwd: state.cwd.clone(),
                    accepted_fingerprint: state.extra_fingerprint.clone(),
                    indexed_bytes: state.indexed_bytes,
                    boundary_digest: state.boundary_digest.clone(),
                },
            ))
        })
        .collect()
}

/// True when every file that changed since the stored state proves append-only:
/// the current file is at least as large, and its persisted indexed prefix
/// still hashes to the stored boundary digest.
fn all_changed_files_prove_append(
    scan: &SourceScan,
    proofs: &HashMap<String, StoredFileProof>,
) -> bool {
    for file in &scan.files {
        let Some(proof) = proofs.get(file.file_path.to_string_lossy().as_ref()) else {
            continue;
        };
        if file.mtime_ns == proof.mtime_ns
            && file.size == proof.size
            && file.identity == proof.identity
        {
            continue;
        }
        if file.size < proof.size || proof.boundary_digest.is_empty() {
            return false;
        }
        let Ok(digest) = prefix_sha256(&file.file_path, proof.indexed_bytes) else {
            return false;
        };
        if digest != proof.boundary_digest {
            return false;
        }
    }
    true
}

fn cache_selectors(
    base: &Selector,
    requested: Option<&Selector>,
    records: &[CoverageRecord],
    inventory: bool,
) -> Vec<Selector> {
    let mut roots = BTreeSet::from([(base.source(), base.root().to_owned())]);
    if let Some(selector) = requested {
        roots.insert((selector.source(), selector.root().to_owned()));
    }
    for record in records.iter().filter(|record| {
        inventory || requested.is_some_and(|target| selector_implies(&record.selector, target))
    }) {
        roots.insert((record.selector.source(), record.selector.root().to_owned()));
    }
    roots
        .into_iter()
        .map(|(source, root)| Selector::All { source, root })
        .collect()
}

fn evaluate_record(
    record: &CoverageRecord,
    scan: &SourceScan,
    proofs: &HashMap<String, StoredFileProof>,
) -> Result<CoverageInventoryStatus, AppError> {
    let mut status = evaluate_coverage_record(record, &scan.snapshot);
    if status.stale_reason == InventoryStaleReason::SourceContentChanged
        && !all_changed_files_prove_append(scan, proofs)
    {
        status.advisory = false;
    }
    Ok(status)
}

fn scan_cached<'a>(
    catalog: &SourceCatalog,
    cache: &SourceMetadataCache,
    selector: &Selector,
    scans: &'a mut HashMap<String, SourceScan>,
) -> Result<&'a SourceScan, AppError> {
    let key = selector.storage_key();
    if !scans.contains_key(&key) {
        let scan = catalog.scan(selector, cache).map_err(AppError::output)?;
        scans.insert(key.clone(), scan);
    }
    Ok(scans.get(&key).expect("scan was inserted above"))
}

fn empty_index_status() -> StatusIndex {
    StatusIndex {
        exists: false,
        session_count: 0,
        message_count: 0,
        earliest_started_at: None,
        latest_ended_at: None,
        db_size_bytes: 0,
        last_sync_at: None,
    }
}

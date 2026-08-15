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
    CoverageInventoryStatus, CoverageRecord, StatusContext, StatusIndex, StatusSummary,
};
use crate::selector::{Selector, selector_implies};
use crate::sources::{
    CachedSourceMetadata, ProjectionCheckpoint, SourceCatalog, SourceMetadataCache, SourceScan,
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
    let (index, records, cache) = match IndexReader::open(&db_path) {
        Ok(reader) => {
            let stats = reader
                .stats(source)
                .map_err(|error| map_index_error(error, &db_path, cwd, paths))?;
            let cache = if reader.layout() == IndexLayout::V8
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
                metadata_cache(&states)
            } else {
                SourceMetadataCache::default()
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
                cache,
            )
        }
        Err(IndexError::NotFound(_)) => (
            empty_index_status(),
            Vec::new(),
            SourceMetadataCache::default(),
        ),
        Err(error) => return Err(map_index_error(error, &db_path, cwd, paths)),
    };

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
        evaluated.push(evaluate_record(record, &catalog, &cache, &mut scans)?);
    }
    let coverage = if args.inventory {
        evaluated.clone()
    } else {
        Vec::new()
    };
    let requested_coverage = if let Some(selector) = requested_selector {
        let requested_scan = scan_cached(&catalog, &cache, &selector, &mut scans)?;
        Some(evaluate_requested_coverage(
            &requested_scan.snapshot,
            &evaluated,
        ))
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

fn metadata_cache(states: &[SourceFileState]) -> SourceMetadataCache {
    SourceMetadataCache::from_entries(states.iter().filter_map(|state| {
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
        Some(CachedSourceMetadata {
            source_id: state.source_id,
            file_path: state.file_path.clone().into(),
            mtime_ns: i128::from(state.mtime_ns.filter(|value| *value > 0)?),
            size: state.size,
            file_identity: checkpoint.file_identity,
            path_date: state.path_date.clone(),
            cwd: state.cwd.clone(),
            accepted_fingerprint: state.extra_fingerprint.clone(),
        })
    }))
}

fn evaluate_record(
    record: &CoverageRecord,
    catalog: &SourceCatalog,
    cache: &SourceMetadataCache,
    scans: &mut HashMap<String, SourceScan>,
) -> Result<CoverageInventoryStatus, AppError> {
    let scan = scan_cached(catalog, cache, &record.selector, scans)?;
    Ok(evaluate_coverage_record(record, &scan.snapshot))
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

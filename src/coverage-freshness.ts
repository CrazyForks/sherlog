import { performance } from "node:perf_hooks";
import { coverageDebugTimingEnabled, isCurrentIndexVersion } from "./env";
import { selectorImplies, selectorSource, selectorStorageKey } from "./selector";
import type {
  CoverageInventoryStatus,
  CoverageRecord,
  RequestedCoverageStatus,
  Selector,
  SourceFileMeta,
  SourceSnapshot,
} from "./types";

export interface CoverageProbeStats {
  dbOpens: number;
  dbMs: number;
  collectFilesMs: number;
  snapshotCalls: number;
  snapshotMs: number;
}

let lastCoverageProbeStats: CoverageProbeStats | null = null;

export function emptyCoverageProbeStats(): CoverageProbeStats {
  return {
    dbOpens: 0,
    dbMs: 0,
    collectFilesMs: 0,
    snapshotCalls: 0,
    snapshotMs: 0,
  };
}

export function getLastCoverageProbeStats(): CoverageProbeStats | null {
  return lastCoverageProbeStats;
}

export function finishCoverageProbe(command: string, stats: CoverageProbeStats): void {
  lastCoverageProbeStats = { ...stats };
  if (!coverageDebugTimingEnabled()) return;
  console.error(
    `[shlog ${command} timing] dbOpens=${stats.dbOpens} dbMs=${round1(stats.dbMs)} collectFilesMs=${round1(stats.collectFilesMs)} snapshotCalls=${stats.snapshotCalls} snapshotMs=${round1(stats.snapshotMs)}`,
  );
}

/**
 * Snapshot each distinct selector at most once. Status/find coverage proofs
 * share this so historical cwd/date_range rows cannot multiply hash work.
 */
export function createCachedSnapshotter(
  snapshotFromFiles: (selector: Selector, files: SourceFileMeta[]) => SourceSnapshot | Promise<SourceSnapshot>,
  files: SourceFileMeta[],
  stats: CoverageProbeStats,
): (selector: Selector) => Promise<SourceSnapshot> {
  const cache = new Map<string, Promise<SourceSnapshot>>();
  return (selector) => {
    const key = selectorStorageKey(selector);
    let pending = cache.get(key);
    if (!pending) {
      stats.snapshotCalls += 1;
      const started = performance.now();
      pending = Promise.resolve(snapshotFromFiles(selector, files)).then((snapshot) => {
        stats.snapshotMs += performance.now() - started;
        return snapshot;
      });
      cache.set(key, pending);
    }
    return pending;
  };
}

/**
 * Freshness proof for one requested selector: snapshot covering selectors
 * only (typically `all(root)` and/or the requested selector itself).
 */
export async function proveRequestedCoverage(
  requestedSnapshot: SourceSnapshot,
  records: CoverageRecord[],
  snapshotFor: (selector: Selector) => Promise<SourceSnapshot>,
): Promise<RequestedCoverageStatus> {
  const covering: CoverageInventoryStatus[] = [];
  for (const record of records) {
    if (!isCurrentIndexVersion(record.indexVersion)) continue;
    if (!selectorImplies(record.selector, requestedSnapshot.selector)) continue;
    covering.push(evaluateCoverageRecord(record, await snapshotFor(record.selector)));
  }
  return evaluateRequestedCoverage(requestedSnapshot, covering);
}

export function evaluateCoverageRecord(
  record: CoverageRecord,
  snapshot: SourceSnapshot,
): CoverageInventoryStatus {
  const fresh = snapshot.fingerprint === record.sourceFingerprint
    && (record.sourceFileSetFingerprint === "" || snapshot.fileSetFingerprint === record.sourceFileSetFingerprint)
    && snapshot.fileCount === record.sourceFileCount
    && isCurrentIndexVersion(record.indexVersion);
  const staleReason: CoverageInventoryStatus["staleReason"] = fresh
    ? "none"
    : record.sourceFileSetFingerprint !== "" && snapshot.fileSetFingerprint === record.sourceFileSetFingerprint
      ? "source_content_changed"
      : "source_set_changed";
  return {
    ...record,
    freshness: fresh ? "fresh" : "stale",
    staleReason,
    advisory: !fresh && isAdvisorySourceContentStale(record.selector, staleReason),
    currentSourceFingerprint: snapshot.fingerprint,
    currentSourceFileSetFingerprint: snapshot.fileSetFingerprint,
    currentSourceFileCount: snapshot.fileCount,
  };
}

export function evaluateRequestedCoverage(
  snapshot: SourceSnapshot,
  coverage: CoverageInventoryStatus[],
): RequestedCoverageStatus {
  const coveringSelectors = coverage.filter((entry) =>
    isCurrentIndexVersion(entry.indexVersion) && selectorImplies(entry.selector, snapshot.selector)
  );
  const hasFreshCovering = coveringSelectors.some((entry) => entry.freshness === "fresh");
  const freshness: RequestedCoverageStatus["freshness"] = hasFreshCovering
    ? "fresh"
    : coveringSelectors.length > 0
      ? "stale"
      : "missing";
  const staleReason = requestedCoverageStaleReason(freshness, coveringSelectors);
  return {
    requested: snapshot.selector,
    complete: freshness === "fresh",
    freshness,
    staleReason,
    sourceFingerprint: snapshot.fingerprint,
    sourceFileSetFingerprint: snapshot.fileSetFingerprint,
    sourceFileCount: snapshot.fileCount,
    coveringSelectors,
    recommendedAction: freshness === "fresh" || isAdvisorySourceContentStale(snapshot.selector, staleReason) ? "query" : "sync",
  };
}

function isAdvisorySourceContentStale(
  selector: SourceSnapshot["selector"],
  staleReason: RequestedCoverageStatus["staleReason"],
): boolean {
  return selectorSource(selector) === "codex" && staleReason === "source_content_changed";
}

function requestedCoverageStaleReason(
  freshness: RequestedCoverageStatus["freshness"],
  coveringSelectors: CoverageInventoryStatus[],
): RequestedCoverageStatus["staleReason"] {
  if (freshness === "fresh") return "none";
  if (freshness === "missing") return "missing";
  return coveringSelectors.some((entry) =>
    entry.sourceFileSetFingerprint !== "" && entry.currentSourceFileSetFingerprint === entry.sourceFileSetFingerprint
  )
    ? "source_content_changed"
    : "source_set_changed";
}

function round1(value: number): string {
  return value.toFixed(1);
}

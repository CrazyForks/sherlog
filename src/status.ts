import { performance } from "node:perf_hooks";
import { INDEX_VERSION, DEFAULT_DB_PATH } from "./env";
import {
  createCachedSnapshotter,
  emptyCoverageProbeStats,
  evaluateCoverageRecord,
  finishCoverageProbe,
  proveRequestedCoverage,
  type CoverageProbeStats,
} from "./coverage-freshness";
import { loadIndexMetadata } from "./index-sidecar";
import { selectorSource } from "./selector";
import { getSessionSourceAdapter } from "./sources";
import type { SessionSourceAdapter } from "./sources/types";
import type {
  CoverageInventoryStatus,
  Selector,
  SessionSourceId,
  SourceFileMeta,
  SourceFileMetaResolver,
  SourceInventory,
  SourceSnapshot,
  StatusSummary,
} from "./types";

export async function collectStatus(options: {
  sourceId?: SessionSourceId;
  rootDir?: string;
  dbPath?: string;
  cwd?: string;
  selector?: Selector;
  inventory?: boolean;
} = {}): Promise<StatusSummary> {
  const source = getSessionSourceAdapter(options.sourceId ?? "codex");
  const root = source.resolveRoot(options.rootDir);
  const dbPath = options.dbPath ?? DEFAULT_DB_PATH;
  const stats = emptyCoverageProbeStats();
  const dbStarted = performance.now();
  const dbState = loadIndexMetadata(dbPath, source.id);
  stats.dbMs = performance.now() - dbStarted;
  stats.dbOpens = dbState.opened ? 1 : 0;

  const contextCache = new Map<string, Promise<StatusSourceContext>>();
  const snapshotters = new Map<string, (selector: Selector) => Promise<SourceSnapshot>>();
  const getContext = (sourceId: SessionSourceId, rootDir: string) =>
    getStatusSourceContext(contextCache, sourceId, rootDir, dbState.metaResolver, stats);
  const snapshotterFor = (context: StatusSourceContext) => {
    const key = statusSourceCacheKey(context.source.id, context.inventory.root);
    let snapshotFor = snapshotters.get(key);
    if (!snapshotFor) {
      snapshotFor = createCachedSnapshotter(
        (selector, files) => context.source.snapshotFromFiles(selector, files),
        context.files,
        stats,
      );
      snapshotters.set(key, snapshotFor);
    }
    return snapshotFor;
  };

  const context = await getContext(source.id, root);
  const sourceInventory = options.inventory
    ? context.inventory
    : compactSourceInventory(context.inventory);

  const coverageStatus: CoverageInventoryStatus[] = [];
  if (options.inventory) {
    for (const record of dbState.coverageRecords) {
      const recordContext = await getContext(selectorSource(record.selector), record.selector.root);
      const snapshot = await snapshotterFor(recordContext)(record.selector);
      coverageStatus.push(evaluateCoverageRecord(record, snapshot));
    }
  }

  const summary: StatusSummary = {
    context: {
      cwd: options.cwd ?? process.cwd(),
      root,
      dbPath,
      indexVersion: INDEX_VERSION,
    },
    sourceInventory,
    index: dbState.index,
    coverageCount: dbState.coverageRecords.length,
    coverage: coverageStatus,
  };
  if (options.selector) {
    const requestedContext = await getContext(selectorSource(options.selector), options.selector.root);
    const requestedSnapshot = await snapshotterFor(requestedContext)(options.selector);
    summary.requestedCoverage = await proveRequestedCoverage(
      requestedSnapshot,
      dbState.coverageRecords,
      snapshotterFor(requestedContext),
    );
  }
  finishCoverageProbe("status", stats);
  return summary;
}

interface StatusSourceContext {
  source: SessionSourceAdapter;
  files: SourceFileMeta[];
  inventory: SourceInventory;
}

function statusSourceCacheKey(sourceId: SessionSourceId, root: string): string {
  return `${sourceId}\0${root}`;
}

function getStatusSourceContext(
  cache: Map<string, Promise<StatusSourceContext>>,
  sourceId: SessionSourceId,
  rootDir: string,
  metaResolver: SourceFileMetaResolver | undefined,
  stats: CoverageProbeStats,
): Promise<StatusSourceContext> {
  const source = getSessionSourceAdapter(sourceId);
  const root = source.resolveRoot(rootDir);
  const key = statusSourceCacheKey(source.id, root);
  let context = cache.get(key);
  if (!context) {
    context = (async () => {
      const collectStarted = performance.now();
      const files = await source.collectFiles(root, { metaResolver });
      stats.collectFilesMs += performance.now() - collectStarted;
      return {
        source,
        files,
        inventory: await source.inventoryFromFiles(root, files),
      };
    })();
    cache.set(key, context);
  }
  return context;
}

function compactSourceInventory(inventory: SourceInventory): SourceInventory {
  return {
    root: inventory.root,
    totalFiles: inventory.totalFiles,
    pathDateRange: inventory.pathDateRange,
    cwdGroups: [],
  };
}

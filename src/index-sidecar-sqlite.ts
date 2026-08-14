import { INDEX_VERSION } from "./env";
import {
  getStatsCounts,
  listCoverageRecords,
  loadSourceFileMetaCache,
  withReadDb,
  type Db,
} from "./db";
import { tableExists } from "./db/sql";
import { buildSourceFileMetaResolver } from "./db/file-meta-cache";
import type { SourceFileMetaCacheEntry } from "./db/file-meta-cache";
import {
  INDEX_SIDECAR_VERSION,
  dbFileSize,
  writeIndexSidecar,
  type IndexMetadata,
  type IndexSidecar,
  type IndexSidecarFileMeta,
} from "./index-sidecar";
import type { CoverageRecord, SessionSourceId } from "./types";
import { SESSION_SOURCE_IDS } from "./types";

/**
 * Snapshot metadata from an already-open write connection. Caller must close
 * the connection (and preferably checkpoint WAL) before `writeIndexSidecar`.
 */
export function snapshotIndexSidecar(db: Db): Omit<IndexSidecar, "writtenAt" | "dbIdentity"> {
  const sources: IndexSidecar["sources"] = {};
  for (const sourceId of SESSION_SOURCE_IDS) {
    const counts = tableExists(db, "sessions")
      ? tableColumnExists(db, "sessions", "source_id")
        ? getStatsCounts(db, sourceId)
        : sourceId === "codex"
          ? getLegacyCodexStatsCounts(db)
          : emptyIndexCounts()
      : emptyIndexCounts();
    sources[sourceId] = {
      ...counts,
      coverageRecords: listCoverageRecordsForStatus(db, sourceId),
      fileMeta: serializeFileMeta(loadSourceFileMetaCache(db, sourceId)),
    };
  }
  return {
    version: INDEX_SIDECAR_VERSION,
    indexVersion: INDEX_VERSION,
    sources,
  };
}

export function checkpointIndexWal(db: Db): void {
  db.pragma("wal_checkpoint(TRUNCATE)");
}

/**
 * Dual-write helper for sync: snapshot while the write connection is open,
 * checkpoint WAL, close, then atomically replace the sidecar. A sidecar
 * failure must not fail a completed sync — the next status falls back to
 * SQLite when identity does not match.
 */
export function persistIndexSidecarAfterWrite(db: Db, dbPath: string): void {
  const snapshot = snapshotIndexSidecar(db);
  try {
    checkpointIndexWal(db);
  } catch {
    // Identity includes the WAL file, so a failed checkpoint is still safe.
  }
  db.close();
  try {
    writeIndexSidecar(dbPath, snapshot);
  } catch {
    // Projection only. Status/find reopen SQLite when the sidecar is absent
    // or its db identity no longer matches.
  }
}

export function loadIndexMetadataFromSqlite(dbPath: string, sourceId: SessionSourceId): IndexMetadata {
  return withReadDb(dbPath, (db) => ({
    opened: true,
    coverageRecords: listCoverageRecordsForStatus(db, sourceId),
    metaResolver: buildSourceFileMetaResolver(loadSourceFileMetaCache(db, sourceId)),
    index: readIndexStatus(db, dbPath, sourceId),
  }));
}

function serializeFileMeta(cache: Map<string, SourceFileMetaCacheEntry>): IndexSidecarFileMeta[] {
  return [...cache.entries()].map(([filePath, entry]) => ({ filePath, ...entry }));
}

function emptyIndexCounts(): ReturnType<typeof getStatsCounts> {
  return {
    sessionCount: 0,
    messageCount: 0,
    earliestStartedAt: null,
    latestEndedAt: null,
    lastSyncAt: null,
  };
}

function readIndexStatus(db: Db, dbPath: string, sourceId: SessionSourceId): IndexMetadata["index"] {
  const counts = !tableExists(db, "sessions")
    ? emptyIndexCounts()
    : !tableColumnExists(db, "sessions", "source_id")
      ? sourceId === "codex"
        ? getLegacyCodexStatsCounts(db)
        : emptyIndexCounts()
      : getStatsCounts(db, sourceId);
  return {
    exists: true,
    sessionCount: counts.sessionCount,
    messageCount: counts.messageCount,
    earliestStartedAt: counts.earliestStartedAt,
    latestEndedAt: counts.latestEndedAt,
    dbSizeBytes: dbFileSize(dbPath),
    lastSyncAt: counts.lastSyncAt,
  };
}

function listCoverageRecordsForStatus(db: Db, sourceId: SessionSourceId): CoverageRecord[] {
  if (!tableColumnExists(db, "coverage", "source_id")) return [];
  return listCoverageRecords(db, sourceId);
}

function getLegacyCodexStatsCounts(db: Db): ReturnType<typeof getStatsCounts> {
  return db
    .prepare(`
      SELECT
        COUNT(*) AS sessionCount,
        COALESCE(SUM(message_count), 0) AS messageCount,
        MIN(started_at) AS earliestStartedAt,
        MAX(ended_at) AS latestEndedAt,
        MAX(updated_at) AS lastSyncAt
      FROM sessions
    `)
    .get() as ReturnType<typeof getStatsCounts>;
}

function tableColumnExists(db: Db, tableName: string, columnName: string): boolean {
  return db
    .prepare<[string, string], { name: string }>(`
      SELECT name
      FROM pragma_table_info(?)
      WHERE name = ?
      LIMIT 1
    `)
    .get(tableName, columnName) !== undefined;
}

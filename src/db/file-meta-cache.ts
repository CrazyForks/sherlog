import type { CachedSourceFileMeta, SessionSourceId, SourceFileMeta, SourceFileMetaResolver } from "../types";
import type { Db } from "./shared";
import { escapeLike, tableExists } from "./sql";

// Sync-time cache of content-derived source file metadata (cwd, pathDate,
// source-specific accepted fingerprints). Written only by `sync` (the single
// state writer); read-only commands consult it so coverage/freshness probes
// can skip re-reading file content for files whose mtime+size are unchanged.
//
// Correctness invariant: cached values come from the exact inventory scan
// whose fingerprints were persisted into `coverage`, so a cache hit
// reproduces the sync-time snapshot fingerprint byte-for-byte. A miss (new
// file, changed mtime/size, or absent row) falls back to the normal content
// scan, which is today's behavior.

export const SOURCE_FILE_META_CACHE_TABLE = "source_file_meta_cache";

export interface SourceFileMetaCacheEntry extends CachedSourceFileMeta {
  mtimeMs: number;
  size: number;
}

export function ensureSourceFileMetaCacheTable(db: Db): void {
  db.exec(`
    CREATE TABLE IF NOT EXISTS ${SOURCE_FILE_META_CACHE_TABLE} (
      source_id TEXT NOT NULL,
      file_path TEXT NOT NULL,
      mtime_ms REAL NOT NULL,
      size INTEGER NOT NULL,
      cwd TEXT NOT NULL DEFAULT '',
      path_date TEXT,
      extra_fingerprint TEXT NOT NULL DEFAULT '',
      updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
      PRIMARY KEY (source_id, file_path)
    )
  `);
}

/**
 * Upsert cache rows for all files of a sync snapshot. When `prune` is set
 * (canonical `all` selector syncs), rows under that root that are no longer
 * part of the snapshot are removed so the cache does not grow unboundedly
 * with deleted files. Narrower selector syncs only upsert.
 */
export function upsertSourceFileMetaCache(
  db: Db,
  sourceId: SessionSourceId,
  files: ReadonlyArray<SourceFileMeta & { acceptedFingerprint?: string }>,
  prune?: { root: string },
): void {
  ensureSourceFileMetaCacheTable(db);
  const insert = db.prepare<[string, string, number, number, string, string | null, string]>(`
    INSERT INTO ${SOURCE_FILE_META_CACHE_TABLE}
      (source_id, file_path, mtime_ms, size, cwd, path_date, extra_fingerprint, updated_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
    ON CONFLICT(source_id, file_path) DO UPDATE SET
      mtime_ms = excluded.mtime_ms,
      size = excluded.size,
      cwd = excluded.cwd,
      path_date = excluded.path_date,
      extra_fingerprint = excluded.extra_fingerprint,
      updated_at = CURRENT_TIMESTAMP
  `);

  const tx = db.transaction(() => {
    if (prune) {
      const keep = new Set(files.map((file) => file.filePath));
      const rows = db
        .prepare<[string, string, string], { file_path: string }>(`
          SELECT file_path FROM ${SOURCE_FILE_META_CACHE_TABLE}
          WHERE source_id = ? AND (file_path = ? OR file_path LIKE ? ESCAPE '\\')
        `)
        .all(sourceId, prune.root, `${escapeLike(prune.root)}/%`) as Array<{ file_path: string }>;
      const remove = db.prepare<[string, string]>(
        `DELETE FROM ${SOURCE_FILE_META_CACHE_TABLE} WHERE source_id = ? AND file_path = ?`,
      );
      for (const row of rows) {
        if (!keep.has(row.file_path)) remove.run(sourceId, row.file_path);
      }
    }
    for (const file of files) {
      insert.run(
        sourceId,
        file.filePath,
        file.mtimeMs,
        file.size,
        file.cwd,
        file.pathDate,
        file.acceptedFingerprint ?? "",
      );
    }
  });
  tx();
}

export function loadSourceFileMetaCache(db: Db, sourceId: SessionSourceId): Map<string, SourceFileMetaCacheEntry> {
  const cache = new Map<string, SourceFileMetaCacheEntry>();
  if (!tableExists(db, SOURCE_FILE_META_CACHE_TABLE)) return cache;

  const rows = db
    .prepare<[string], { file_path: string; mtime_ms: number; size: number; cwd: string; path_date: string | null; extra_fingerprint: string }>(`
      SELECT file_path, mtime_ms, size, cwd, path_date, extra_fingerprint
      FROM ${SOURCE_FILE_META_CACHE_TABLE}
      WHERE source_id = ?
    `)
    .all(sourceId) as Array<{
      file_path: string;
      mtime_ms: number;
      size: number;
      cwd: string;
      path_date: string | null;
      extra_fingerprint: string;
    }>;

  for (const row of rows) {
    cache.set(row.file_path, {
      mtimeMs: row.mtime_ms,
      size: row.size,
      cwd: row.cwd,
      pathDate: row.path_date,
      extraFingerprint: row.extra_fingerprint,
    });
  }
  return cache;
}

export function buildSourceFileMetaResolver(cache: Map<string, SourceFileMetaCacheEntry>): SourceFileMetaResolver {
  return (filePath, mtimeMs, size) => {
    const entry = cache.get(filePath);
    if (!entry) return null;
    if (entry.mtimeMs !== mtimeMs || entry.size !== size) return null;
    return { cwd: entry.cwd, pathDate: entry.pathDate, extraFingerprint: entry.extraFingerprint };
  };
}

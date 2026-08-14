import { existsSync, mkdirSync, readFileSync, renameSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname, parse, resolve } from "node:path";
import { buildSourceFileMetaResolver } from "./db/file-meta-cache";
import type { SourceFileMetaCacheEntry } from "./db/file-meta-cache";
import type {
  CoverageRecord,
  SessionSourceId,
  SourceFileMetaResolver,
  StatusSummary,
} from "./types";
import { SESSION_SOURCE_IDS } from "./types";

export const INDEX_SIDECAR_VERSION = 1;
export const INDEX_SIDECAR_FILE_SUFFIX = ".meta.json";

export interface IndexSidecarDbIdentity {
  sqliteMtimeMs: number;
  sqliteSize: number;
  walMtimeMs: number;
  walSize: number;
}

export interface IndexSidecarFileMeta extends SourceFileMetaCacheEntry {
  filePath: string;
}

export interface IndexSidecarSourceSlice {
  sessionCount: number;
  messageCount: number;
  earliestStartedAt: string | null;
  latestEndedAt: string | null;
  lastSyncAt: string | null;
  coverageRecords: CoverageRecord[];
  fileMeta: IndexSidecarFileMeta[];
}

export interface IndexSidecar {
  version: number;
  indexVersion: string;
  writtenAt: string;
  dbIdentity: IndexSidecarDbIdentity;
  sources: Partial<Record<SessionSourceId, IndexSidecarSourceSlice>>;
}

export interface IndexMetadata {
  opened: boolean;
  coverageRecords: CoverageRecord[];
  metaResolver?: SourceFileMetaResolver;
  index: StatusSummary["index"];
}

/**
 * Sidecar lives next to the body index: `index.sqlite` → `index.meta.json`.
 * Status/find coverage proofs read this file; they open SQLite only when the
 * sidecar is missing or its db identity no longer matches.
 */
export function indexSidecarPathForDb(dbPath: string): string {
  const resolved = resolve(dbPath);
  const parsed = parse(resolved);
  return resolve(parsed.dir, `${parsed.name}${INDEX_SIDECAR_FILE_SUFFIX}`);
}

export function readDbIdentity(dbPath: string): IndexSidecarDbIdentity {
  return {
    ...fileIdentity(dbPath, "sqlite" as const),
    ...fileIdentity(`${dbPath}-wal`, "wal" as const),
  };
}

export function dbIdentitiesMatch(left: IndexSidecarDbIdentity, right: IndexSidecarDbIdentity): boolean {
  return left.sqliteMtimeMs === right.sqliteMtimeMs
    && left.sqliteSize === right.sqliteSize
    && left.walMtimeMs === right.walMtimeMs
    && left.walSize === right.walSize;
}

export async function loadIndexMetadata(dbPath: string, sourceId: SessionSourceId): Promise<IndexMetadata> {
  if (!existsSync(dbPath)) {
    return {
      opened: false,
      coverageRecords: [],
      index: emptyIndexStatus(),
    };
  }

  const sidecar = readIndexSidecar(dbPath);
  if (sidecar) {
    const slice = sidecar.sources[sourceId];
    return {
      opened: false,
      coverageRecords: slice?.coverageRecords ?? [],
      metaResolver: buildSourceFileMetaResolver(fileMetaMap(slice?.fileMeta ?? [])),
      index: indexStatusFromSlice(dbPath, slice),
    };
  }

  const { loadIndexMetadataFromSqlite } = await import("./index-sidecar-sqlite");
  return loadIndexMetadataFromSqlite(dbPath, sourceId);
}

export function writeIndexSidecar(
  dbPath: string,
  snapshot: Omit<IndexSidecar, "writtenAt" | "dbIdentity">,
): void {
  const sidecarPath = indexSidecarPathForDb(dbPath);
  const payload: IndexSidecar = {
    ...snapshot,
    writtenAt: new Date().toISOString(),
    dbIdentity: readDbIdentity(dbPath),
  };
  atomicWriteJson(sidecarPath, payload);
}

export function readIndexSidecar(dbPath: string): IndexSidecar | null {
  const sidecarPath = indexSidecarPathForDb(dbPath);
  if (!existsSync(sidecarPath)) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(sidecarPath, "utf8")) as unknown;
  } catch {
    return null;
  }
  if (!isIndexSidecar(parsed)) return null;
  if (!dbIdentitiesMatch(parsed.dbIdentity, readDbIdentity(dbPath))) return null;
  return parsed;
}

export function emptyIndexStatus(): StatusSummary["index"] {
  return {
    exists: false,
    sessionCount: 0,
    messageCount: 0,
    earliestStartedAt: null,
    latestEndedAt: null,
    dbSizeBytes: 0,
    lastSyncAt: null,
  };
}

export function dbFileSize(dbPath: string): number {
  try {
    return statSync(dbPath).size;
  } catch {
    return 0;
  }
}

function fileIdentity(path: string, kind: "sqlite"): Pick<IndexSidecarDbIdentity, "sqliteMtimeMs" | "sqliteSize">;
function fileIdentity(path: string, kind: "wal"): Pick<IndexSidecarDbIdentity, "walMtimeMs" | "walSize">;
function fileIdentity(
  path: string,
  kind: "sqlite" | "wal",
): Pick<IndexSidecarDbIdentity, "sqliteMtimeMs" | "sqliteSize"> | Pick<IndexSidecarDbIdentity, "walMtimeMs" | "walSize"> {
  const missing = !existsSync(path);
  const stats = missing ? null : statSync(path);
  const size = stats ? stats.size : 0;
  // A missing WAL and a leftover 0-byte WAL are the same: no uncheckpointed
  // writes. Touching an empty WAL (readers, checkpoint leftovers) must not
  // invalidate the sidecar, or status falls back and loads the native addon.
  if (kind === "wal" && (missing || size === 0)) {
    return { walMtimeMs: 0, walSize: 0 };
  }
  const mtimeMs = stats ? Math.round(stats.mtimeMs) : 0;
  return kind === "sqlite"
    ? { sqliteMtimeMs: mtimeMs, sqliteSize: size }
    : { walMtimeMs: mtimeMs, walSize: size };
}

function fileMetaMap(entries: IndexSidecarFileMeta[]): Map<string, SourceFileMetaCacheEntry> {
  const cache = new Map<string, SourceFileMetaCacheEntry>();
  for (const entry of entries) {
    cache.set(entry.filePath, {
      mtimeMs: entry.mtimeMs,
      size: entry.size,
      cwd: entry.cwd,
      pathDate: entry.pathDate,
      extraFingerprint: entry.extraFingerprint,
    });
  }
  return cache;
}

function indexStatusFromSlice(dbPath: string, slice: IndexSidecarSourceSlice | undefined): StatusSummary["index"] {
  return {
    exists: true,
    sessionCount: slice?.sessionCount ?? 0,
    messageCount: slice?.messageCount ?? 0,
    earliestStartedAt: slice?.earliestStartedAt ?? null,
    latestEndedAt: slice?.latestEndedAt ?? null,
    dbSizeBytes: dbFileSize(dbPath),
    lastSyncAt: slice?.lastSyncAt ?? null,
  };
}

function atomicWriteJson(filePath: string, value: unknown): void {
  const dir = dirname(filePath);
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
  const tempPath = `${filePath}.${process.pid}.tmp`;
  writeFileSync(tempPath, `${JSON.stringify(value)}\n`, "utf8");
  try {
    renameSync(tempPath, filePath);
  } catch (error) {
    try {
      unlinkSync(tempPath);
    } catch {
      // Keep the original write error.
    }
    throw error;
  }
}

function isIndexSidecar(value: unknown): value is IndexSidecar {
  if (!isRecord(value)) return false;
  if (value.version !== INDEX_SIDECAR_VERSION) return false;
  if (typeof value.indexVersion !== "string") return false;
  if (typeof value.writtenAt !== "string") return false;
  if (!isDbIdentity(value.dbIdentity)) return false;
  if (!isRecord(value.sources)) return false;
  for (const [sourceId, slice] of Object.entries(value.sources)) {
    if (!SESSION_SOURCE_IDS.includes(sourceId as SessionSourceId)) return false;
    if (slice !== undefined && !isSourceSlice(slice)) return false;
  }
  return true;
}

function isDbIdentity(value: unknown): value is IndexSidecarDbIdentity {
  if (!isRecord(value)) return false;
  return Number.isFinite(value.sqliteMtimeMs)
    && Number.isFinite(value.sqliteSize)
    && Number.isFinite(value.walMtimeMs)
    && Number.isFinite(value.walSize);
}

function isSourceSlice(value: unknown): value is IndexSidecarSourceSlice {
  if (!isRecord(value)) return false;
  if (!Number.isFinite(value.sessionCount) || !Number.isFinite(value.messageCount)) return false;
  if (!isNullString(value.earliestStartedAt) || !isNullString(value.latestEndedAt) || !isNullString(value.lastSyncAt)) {
    return false;
  }
  if (!Array.isArray(value.coverageRecords) || !value.coverageRecords.every(isCoverageRecord)) return false;
  if (!Array.isArray(value.fileMeta) || !value.fileMeta.every(isFileMeta)) return false;
  return true;
}

function isCoverageRecord(value: unknown): value is CoverageRecord {
  if (!isRecord(value)) return false;
  return Number.isFinite(value.id)
    && isRecord(value.selector)
    && typeof value.sourceFingerprint === "string"
    && typeof value.sourceFileSetFingerprint === "string"
    && Number.isFinite(value.sourceFileCount)
    && Number.isFinite(value.indexedSessionCount)
    && typeof value.completedAt === "string"
    && typeof value.indexVersion === "string";
}

function isFileMeta(value: unknown): value is IndexSidecarFileMeta {
  if (!isRecord(value)) return false;
  return typeof value.filePath === "string"
    && Number.isFinite(value.mtimeMs)
    && Number.isFinite(value.size)
    && typeof value.cwd === "string"
    && (value.pathDate === null || typeof value.pathDate === "string")
    && typeof value.extraFingerprint === "string";
}

function isNullString(value: unknown): value is string | null {
  return value === null || typeof value === "string";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

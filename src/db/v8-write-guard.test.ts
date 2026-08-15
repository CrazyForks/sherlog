import { spawnSync } from "node:child_process";
import {
  lstatSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readlinkSync,
  rmSync,
  statSync,
  symlinkSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { DatabaseSync } from "node:sqlite";
import { afterEach, expect, test } from "vitest";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const v8Schema = readFileSync(join(repoRoot, "rust/src/index/v8.sql"), "utf8");
const tempDirs: string[] = [];

afterEach(() => {
  for (const directory of tempDirs.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("legacy TypeScript sync fails closed before mutating a v8 index", () => {
  const base = mkdtempSync(join(tmpdir(), "shlog-v8-ts-writer-guard-"));
  tempDirs.push(base);
  const dbPath = join(base, "state", "index.sqlite");
  const rawRoot = join(base, "raw-sessions");
  mkdirSync(dirname(dbPath), { recursive: true });
  mkdirSync(rawRoot, { recursive: true });
  createSeededV8(dbPath);
  const before = logicalState(dbPath);
  expect(legacySchemaObjectCount(dbPath)).toBe(0);

  // This deliberately invokes the checkout's legacy TypeScript CLI rather
  // than its lower-level schema helper.  It proves the whole old sync write
  // path exits non-zero when it reaches ensureSchema on a v8 database.
  const attempted = spawnSync(
    join(repoRoot, "node_modules/.bin/tsx"),
    [
      "src/cli.ts",
      "sync",
      "--source",
      "codex",
      "--root",
      rawRoot,
      "--db",
      dbPath,
      "--json",
    ],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        HOME: base,
        SHLOG_DATA_DIR: join(base, "legacy-state"),
      },
      timeout: 30_000,
    },
  );

  expect(attempted.error).toBeUndefined();
  expect(attempted.status).toBe(1);
  expect(`${attempted.stdout}\n${attempted.stderr}`).toContain(
    "cannot modify sessions because it is a view",
  );
  expect(logicalState(dbPath)).toEqual(before);
  expect(legacySchemaObjectCount(dbPath)).toBe(0);
}, 60_000);

test("legacy TypeScript cold add cannot replace the v8 config tombstone", () => {
  const base = mkdtempSync(join(tmpdir(), "shlog-v8-ts-cold-guard-"));
  tempDirs.push(base);
  const dbPath = join(base, "state", "index.sqlite");
  const coldRoot = join(base, "archived-sessions");
  const tombstone = join(dirname(dbPath), "cold-roots.json");
  const tombstoneTarget = join(dirname(dbPath), "cold-roots.json.v8-tombstone.guard");
  mkdirSync(dirname(dbPath), { recursive: true });
  mkdirSync(coldRoot, { recursive: true });
  mkdirSync(tombstoneTarget, { mode: 0o700 });
  symlinkSync(basename(tombstoneTarget), tombstone, "dir");
  createSeededV8(dbPath);
  const before = logicalState(dbPath);

  const attempted = spawnSync(
    join(repoRoot, "node_modules/.bin/tsx"),
    [
      "src/cli.ts",
      "cold",
      "add",
      "--root",
      coldRoot,
      "--source",
      "codex",
      "--db",
      dbPath,
      "--json",
    ],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        HOME: base,
        SHLOG_DATA_DIR: join(base, "legacy-state"),
      },
      timeout: 30_000,
    },
  );

  expect(attempted.error).toBeUndefined();
  expect(attempted.status).toBe(1);
  expect(`${attempted.stdout}\n${attempted.stderr}`).toMatch(
    /EISDIR|illegal operation on a directory|is a directory/i,
  );
  expect(lstatSync(tombstone).isSymbolicLink()).toBe(true);
  expect(readlinkSync(tombstone)).toBe(basename(tombstoneTarget));
  expect(statSync(tombstone).isDirectory()).toBe(true);
  expect(statSync(tombstoneTarget).mode & 0o777).toBe(0o700);
  expect(logicalState(dbPath)).toEqual(before);
}, 60_000);

function createSeededV8(dbPath: string): void {
  const db = new DatabaseSync(dbPath);
  db.prepare("PRAGMA journal_mode = WAL").get();
  db.exec("PRAGMA foreign_keys = ON");
  db.exec(v8Schema);
  db.exec(`
    INSERT INTO meta(key, value) VALUES
      ('schema_version', '8'),
      ('projection_epoch', '1'),
      ('analyzer_epoch', '1'),
      ('coverage_epoch', '1'),
      ('index_version', 'shlog-v8-unicode-word-cjk-scalar'),
      ('created_at', '2026-08-15T00:00:00.000Z');

    INSERT INTO session_rows(
      id, source_id, native_session_id, session_key, session_uuid, file_path,
      source_root, title, summary_text, compact_text, reasoning_summary_text,
      cwd, model, started_at, ended_at, path_date, message_count,
      document_count, raw_file_mtime, raw_file_size, index_version, updated_at
    ) VALUES (
      1, 'codex', 'guard-session', 'codex:guard-session', 'guard-session',
      '/seed/session.jsonl', '/seed', 'guard title', 'guard summary',
      'guard compact', 'guard reasoning', '/repo', 'gpt-test',
      '2026-08-15T00:00:00Z', '2026-08-15T00:01:00Z', '2026-08-15',
      1, 2, 1, 10, 'shlog-v8-unicode-word-cjk-scalar',
      '2026-08-15T00:02:00Z'
    );

    INSERT INTO documents(
      id, session_id, kind, seq, role, timestamp, source_kind, body_text,
      title_text, summary_text, compact_text, reasoning_text,
      raw_start, raw_end, projection_epoch
    ) VALUES
      (1, 1, 'session_profile', NULL, NULL, NULL, NULL, '', 'guard title',
       'guard summary', 'guard compact', 'guard reasoning', NULL, NULL, 1),
      (2, 1, 'message', 0, 'user', '2026-08-15T00:00:00Z', 'event_msg',
       'guard body', '', '', '', '', 0, 10, 1);

    INSERT INTO documents_fts(
      rowid, body_text, title_text, summary_text, compact_text, reasoning_text
    ) VALUES
      (1, '', 'guard title', 'guard summary', 'guard compact', 'guard reasoning'),
      (2, 'guard body', '', '', '', '');

    INSERT INTO source_files(
      source_id, file_path, session_id, source_root, source_generation,
      mtime_ms, size, indexed_bytes, head_digest, boundary_digest, next_seq,
      reducer_checkpoint, cwd, path_date, extra_fingerprint,
      projection_epoch, analyzer_epoch, coverage_epoch, updated_at
    ) VALUES (
      'codex', '/seed/session.jsonl', 1, '/seed', 'generation-1',
      1.25, 10, 10, 'head', 'boundary', 1, NULL, '/repo', '2026-08-15',
      'accepted', 1, 1, 1, '2026-08-15T00:02:00Z'
    );

    INSERT INTO coverage(
      source_id, selector_key, selector_json, selector_kind, root,
      source_fingerprint, source_file_set_fingerprint, source_file_count,
      indexed_session_count, indexed_document_count, source_generation,
      completed_at, index_version, projection_epoch, analyzer_epoch, coverage_epoch
    ) VALUES (
      'codex', '{"kind":"all","source":"codex","root":"/seed"}',
      '{"kind":"all","source":"codex","root":"/seed"}', 'all', '/seed',
      'content', 'files', 1, 1, 2, 'generation-1', '2026-08-15T00:02:00Z',
      'shlog-v8-unicode-word-cjk-scalar', 1, 1, 1
    );

    INSERT INTO cold_roots(source_id, root, added_at)
    VALUES ('codex', '/cold', '2026-08-15T00:00:00Z');
  `);
  db.close();
}

function logicalState(dbPath: string): unknown {
  const db = new DatabaseSync(dbPath, { readOnly: true });
  try {
    return {
      userVersion: (
        db.prepare("PRAGMA user_version").get() as { user_version: number }
      ).user_version,
      foreignKeyCheck: db.prepare("PRAGMA foreign_key_check").all(),
      schemaObjects: db
        .prepare(`
          SELECT type, name, tbl_name AS tableName, sql
          FROM sqlite_master
          WHERE name IN (
            'sessions', 'session_rows', 'messages', 'documents',
            'documents_fts', 'source_files', 'coverage', 'cold_roots'
          )
          ORDER BY type, name
        `)
        .all(),
      meta: db.prepare("SELECT key, value FROM meta ORDER BY key").all(),
      sessionRows: db.prepare("SELECT * FROM session_rows ORDER BY id").all(),
      publicSessions: db.prepare("SELECT * FROM sessions ORDER BY id").all(),
      documents: db.prepare("SELECT * FROM documents ORDER BY id").all(),
      ftsRows: db.prepare("SELECT rowid FROM documents_fts ORDER BY rowid").all(),
      ftsMatches: db
        .prepare("SELECT rowid FROM documents_fts WHERE documents_fts MATCH 'guard' ORDER BY rowid")
        .all(),
      sourceFiles: db
        .prepare("SELECT * FROM source_files ORDER BY source_id, file_path")
        .all(),
      coverage: db.prepare("SELECT * FROM coverage ORDER BY id").all(),
      coldRoots: db.prepare("SELECT * FROM cold_roots ORDER BY source_id, root").all(),
    };
  } finally {
    db.close();
  }
}

function legacySchemaObjectCount(dbPath: string): number {
  const db = new DatabaseSync(dbPath, { readOnly: true });
  try {
    return Number(
      db
        .prepare(`
          SELECT COUNT(*) AS count
          FROM sqlite_master
          WHERE name IN (
            'messages', 'messages_fts', 'sessions_fts', 'source_file_meta_cache'
          )
        `)
        .get()!.count,
    );
  } finally {
    db.close();
  }
}

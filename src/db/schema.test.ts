import { afterEach, describe, expect, test } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { INDEX_VERSION } from "../env";
import { findSessions } from "../query";
import { openWriteDb, replaceSession, type Db } from "../db";
import { tokenizedText } from "../tokenize";

const tempDirs: string[] = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

describe("contentless FTS schema", () => {
  test("new indexes create contentless FTS without *_content shadow tables", () => {
    const dbPath = tempDbPath("cxs-fts-contentless-new-");
    const db = openWriteDb(dbPath);
    replaceSession(
      db,
      sessionFixture(join(dbPath, ".."), "raw health check needle 健康检查"),
      1,
      100,
      INDEX_VERSION,
      "2026-04-22",
    );
    const sql = ftsSql(db, "messages_fts");
    const sessionSql = ftsSql(db, "sessions_fts");
    const shadowTables = shadowFtsTables(db);
    db.close();

    expect(sql).toMatch(/content\s*=\s*''/);
    expect(sql).toMatch(/contentless_delete\s*=\s*1/);
    expect(sessionSql).toMatch(/content\s*=\s*''/);
    expect(sessionSql).toMatch(/contentless_delete\s*=\s*1/);
    expect(shadowTables).not.toContain("messages_fts_content");
    expect(shadowTables).not.toContain("sessions_fts_content");

    const found = findSessions(dbPath, "health check", 5);
    expect(found.results[0]?.snippet).toContain("health check");
    expect(found.results[0]?.snippet).toContain("<mark>");
    expect(found.results[0]?.snippet).not.toContain(tokenizedText("raw health check needle 健康检查"));
  });

  test("migrates contentful FTS from stored rows without rewriting message bodies", () => {
    const dbPath = tempDbPath("cxs-fts-contentless-migrate-");
    const filePath = join(dbPath, "..", "legacy.jsonl");
    const body = "contentless-migrate-needle and 健康检查 survives";
    const legacy = new Database(dbPath);
    legacy.exec(`
      CREATE TABLE sessions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        source_id TEXT NOT NULL DEFAULT 'codex',
        native_session_id TEXT NOT NULL DEFAULT '',
        session_key TEXT NOT NULL UNIQUE,
        session_uuid TEXT NOT NULL,
        file_path TEXT NOT NULL,
        source_root TEXT NOT NULL DEFAULT '',
        title TEXT NOT NULL DEFAULT '',
        summary_text TEXT NOT NULL DEFAULT '',
        compact_text TEXT NOT NULL DEFAULT '',
        reasoning_summary_text TEXT NOT NULL DEFAULT '',
        cwd TEXT NOT NULL DEFAULT '',
        model TEXT NOT NULL DEFAULT '',
        started_at TEXT NOT NULL,
        ended_at TEXT NOT NULL,
        path_date TEXT NOT NULL DEFAULT '',
        message_count INTEGER NOT NULL DEFAULT 0,
        raw_file_mtime INTEGER NOT NULL DEFAULT 0,
        raw_file_size INTEGER NOT NULL DEFAULT 0,
        index_version TEXT NOT NULL DEFAULT '',
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        UNIQUE(source_id, native_session_id),
        UNIQUE(source_id, file_path)
      );
      CREATE TABLE messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        session_uuid TEXT NOT NULL,
        seq INTEGER NOT NULL,
        role TEXT NOT NULL,
        content_text TEXT NOT NULL,
        timestamp TEXT NOT NULL,
        source_kind TEXT NOT NULL,
        UNIQUE(session_id, seq)
      );
      CREATE VIRTUAL TABLE messages_fts USING fts5(
        content_text,
        session_uuid UNINDEXED,
        seq UNINDEXED,
        role UNINDEXED,
        timestamp UNINDEXED,
        tokenize='unicode61 remove_diacritics 1'
      );
      CREATE VIRTUAL TABLE sessions_fts USING fts5(
        title,
        summary_text,
        compact_text,
        reasoning_summary_text,
        session_uuid UNINDEXED,
        tokenize='unicode61 remove_diacritics 1'
      );
    `);
    legacy.prepare(`
      INSERT INTO sessions (
        source_id, native_session_id, session_key, session_uuid, file_path, source_root,
        title, summary_text, cwd, model, started_at, ended_at, path_date,
        message_count, raw_file_mtime, raw_file_size, index_version
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(
      "codex",
      "55555555-5555-4555-8555-555555555555",
      "codex:55555555-5555-4555-8555-555555555555",
      "55555555-5555-4555-8555-555555555555",
      filePath,
      join(dbPath, ".."),
      "legacy title",
      "legacy summary",
      "/tmp/legacy-fts",
      "gpt-5.4",
      "2026-04-22T00:00:00.000Z",
      "2026-04-22T00:00:00.000Z",
      "2026-04-22",
      1,
      1,
      1,
      "old-version",
    );
    legacy.prepare(`
      INSERT INTO messages (session_id, session_uuid, seq, role, content_text, timestamp, source_kind)
      VALUES (1, ?, 0, 'user', ?, '2026-04-22T00:00:00.000Z', 'event_msg')
    `).run("55555555-5555-4555-8555-555555555555", body);
    legacy.prepare(`
      INSERT INTO messages_fts(rowid, content_text, session_uuid, seq, role, timestamp)
      VALUES (1, ?, ?, 0, 'user', '2026-04-22T00:00:00.000Z')
    `).run(tokenizedText(body), "55555555-5555-4555-8555-555555555555");
    legacy.prepare(`
      INSERT INTO sessions_fts(rowid, title, summary_text, compact_text, reasoning_summary_text, session_uuid)
      VALUES (1, ?, ?, '', '', ?)
    `).run(tokenizedText("legacy title"), tokenizedText("legacy summary"), "55555555-5555-4555-8555-555555555555");
    expect(shadowFtsTables(legacy)).toContain("messages_fts_content");
    expect(shadowFtsTables(legacy)).toContain("sessions_fts_content");
    legacy.close();

    const migrated = openWriteDb(dbPath);
    const sql = ftsSql(migrated, "messages_fts");
    const shadows = shadowFtsTables(migrated);
    const storedBody = migrated.prepare("SELECT content_text AS contentText FROM messages").get() as { contentText: string };
    const ftsContent = migrated.prepare("SELECT content_text AS contentText FROM messages_fts").get() as { contentText: string | null };
    migrated.close();

    expect(storedBody.contentText).toBe(body);
    expect(sql).toMatch(/content\s*=\s*''/);
    expect(sql).toMatch(/contentless_delete\s*=\s*1/);
    expect(shadows).not.toContain("messages_fts_content");
    expect(shadows).not.toContain("sessions_fts_content");
    expect(ftsContent.contentText).toBeNull();

    const found = findSessions(dbPath, "contentless-migrate-needle", 5);
    expect(found.results).toHaveLength(1);
    expect(found.results[0]?.snippet).toContain("contentless-migrate-needle");
    expect(found.results[0]?.snippet).toContain("健康检查");

    const second = openWriteDb(dbPath);
    expect(ftsSql(second, "messages_fts")).toBe(sql);
    second.close();
  });
});

function tempDbPath(prefix: string): string {
  const base = mkdtempSync(join(tmpdir(), prefix));
  tempDirs.push(base);
  return join(base, "index.sqlite");
}

function ftsSql(db: Db, tableName: string): string {
  const row = db.prepare("SELECT sql FROM sqlite_master WHERE name = ? LIMIT 1").get(tableName) as { sql: string } | undefined;
  return row?.sql ?? "";
}

function shadowFtsTables(db: Db): string[] {
  const rows = db.prepare(`
    SELECT name FROM sqlite_master
    WHERE name IN ('messages_fts_content', 'sessions_fts_content')
  `).all() as Array<{ name: string }>;
  return rows.map((row) => row.name);
}

function sessionFixture(base: string, message: string) {
  return {
    sourceId: "codex" as const,
    nativeSessionId: "66666666-6666-4666-8666-666666666666",
    sessionKey: "codex:66666666-6666-4666-8666-666666666666",
    sessionUuid: "66666666-6666-4666-8666-666666666666",
    filePath: join(base, "rollout.jsonl"),
    title: message,
    summaryText: message,
    compactText: "",
    reasoningSummaryText: "",
    cwd: "/tmp/fts-contentless",
    model: "gpt-5.4",
    startedAt: "2026-04-22T00:00:00.000Z",
    endedAt: "2026-04-22T00:00:00.000Z",
    messages: [
      {
        role: "user" as const,
        contentText: message,
        timestamp: "2026-04-22T00:00:00.000Z",
        seq: 0,
        sourceKind: "event_msg" as const,
      },
    ],
  };
}

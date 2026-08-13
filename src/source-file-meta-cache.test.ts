import { afterEach, describe, expect, test } from "vitest";
import { mkdtempSync, mkdirSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  buildSourceFileMetaResolver,
  loadSourceFileMetaCache,
  upsertSourceFileMetaCache,
  withReadDb,
} from "./db";
import { openWriteDb } from "./db";
import { syncSessions } from "./indexer";
import { collectCodexSourceFiles } from "./sources/codex-inventory";
import { collectStatus } from "./status";

const tempDirs: string[] = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

describe("source file meta cache", () => {
  test("sync persists cache rows whose values match the snapshot scan", async () => {
    const { root, dbPath, filePath } = writeCodexFixture("cache-write");

    await syncSessions({ dbPath, selector: { kind: "all", root } });

    const cache = withReadDb(dbPath, (db) => loadSourceFileMetaCache(db, "codex"));
    const stats = statSync(filePath);
    const entry = cache.get(filePath);
    expect(entry).toBeDefined();
    expect(entry?.cwd).toBe("/tmp/cache-write");
    expect(entry?.mtimeMs).toBe(stats.mtimeMs);
    expect(entry?.size).toBe(stats.size);
  });

  test("resolver hit skips the content prefix read; miss falls back", async () => {
    const { root, filePath } = writeCodexFixture("resolver-hit");
    const stats = statSync(filePath);

    // A resolver returning a sentinel cwd proves the content scan was skipped
    // (the file's real cwd is /tmp/resolver-hit).
    const hits = await collectCodexSourceFiles(root, {
      metaResolver: (candidatePath, mtimeMs, size) =>
        candidatePath === filePath && mtimeMs === stats.mtimeMs && size === stats.size
          ? { cwd: "/tmp/from-cache", pathDate: "2026-04-22", extraFingerprint: "" }
          : null,
    });
    expect(hits).toHaveLength(1);
    expect(hits[0]?.cwd).toBe("/tmp/from-cache");

    // Wrong mtime/size (stale cache row) must fall back to the content scan.
    const misses = await collectCodexSourceFiles(root, {
      metaResolver: () => null,
    });
    expect(misses).toHaveLength(1);
    expect(misses[0]?.cwd).toBe("/tmp/resolver-hit");
  });

  test("buildSourceFileMetaResolver only resolves matching mtime and size", () => {
    const cache = new Map([
      ["/a.jsonl", { mtimeMs: 100.5, size: 42, cwd: "/tmp/a", pathDate: "2026-04-22", extraFingerprint: "fp" }],
    ]);
    const resolver = buildSourceFileMetaResolver(cache);
    expect(resolver("/a.jsonl", 100.5, 42)).toEqual({ cwd: "/tmp/a", pathDate: "2026-04-22", extraFingerprint: "fp" });
    expect(resolver("/a.jsonl", 100.5, 43)).toBeNull();
    expect(resolver("/a.jsonl", 101, 42)).toBeNull();
    expect(resolver("/b.jsonl", 100.5, 42)).toBeNull();
  });

  test("upsert prunes rows under an all-selector root that left the snapshot", () => {
    const base = mkdtempSync(join(tmpdir(), "cxs-meta-cache-prune-"));
    tempDirs.push(base);
    const dbPath = join(base, "index.sqlite");
    const db = openWriteDb(dbPath);
    try {
      const meta = (filePath: string) => ({ filePath, pathDate: null, cwd: "/tmp/x", mtimeMs: 1, size: 1 });
      upsertSourceFileMetaCache(db, "codex", [meta("/root/a.jsonl"), meta("/root/b.jsonl")]);
      upsertSourceFileMetaCache(db, "codex", [meta("/other/keep.jsonl")]);
      // All-selector sync over /root now only sees a.jsonl: b must be pruned,
      // rows outside the root must be retained.
      upsertSourceFileMetaCache(db, "codex", [meta("/root/a.jsonl")], { root: "/root" });

      const cache = loadSourceFileMetaCache(db, "codex");
      expect([...cache.keys()].sort()).toEqual(["/other/keep.jsonl", "/root/a.jsonl"]);
    } finally {
      db.close();
    }
  });

  test("status coverage stays fresh when probing through the cache", async () => {
    const { root, dbPath } = writeCodexFixture("fresh-probe");

    await syncSessions({ dbPath, selector: { kind: "all", root } });

    // The status probe now resolves file metadata from the cache; a fresh,
    // unchanged source tree must still evaluate as fresh (fingerprints are
    // byte-identical with the sync-time scan).
    const status = await collectStatus({
      rootDir: root,
      dbPath,
      selector: { kind: "all", root },
    });
    expect(status.requestedCoverage?.freshness).toBe("fresh");
  });

  test("changed files bypass the cache and surface as stale coverage", async () => {
    const { root, dbPath, filePath } = writeCodexFixture("stale-probe");

    await syncSessions({ dbPath, selector: { kind: "all", root } });

    // Rewrite the file with different content and a different mtime: the
    // cache row no longer matches, so the probe re-reads content and the
    // coverage fingerprint must diverge.
    writeFileSync(
      filePath,
      [
        line("session_meta", { id: "11111111-1111-4111-8111-111111111111", cwd: "/tmp/stale-probe-changed" }),
        line("event_msg", { type: "user_message", message: "rewritten content" }),
      ].join("\n"),
    );
    utimesSync(filePath, new Date(), new Date("2030-01-01T00:00:00Z"));

    const status = await collectStatus({
      rootDir: root,
      dbPath,
      selector: { kind: "all", root },
    });
    expect(status.requestedCoverage?.freshness).toBe("stale");
  });
});

function writeCodexFixture(name: string): { root: string; dbPath: string; filePath: string } {
  const base = mkdtempSync(join(tmpdir(), `cxs-meta-cache-${name}-`));
  tempDirs.push(base);
  const root = join(base, "sessions");
  const day = join(root, "2026", "04", "22");
  mkdirSync(day, { recursive: true });
  const filePath = join(day, "rollout-2026-04-22T10-00-00-11111111-1111-4111-8111-111111111111.jsonl");
  writeFileSync(
    filePath,
    [
      line("session_meta", { id: "11111111-1111-4111-8111-111111111111", cwd: `/tmp/${name}` }),
      line("event_msg", { type: "user_message", message: `${name} user message` }),
      line("event_msg", { type: "agent_message", message: `${name} agent reply` }),
    ].join("\n"),
  );
  return { root, dbPath: join(base, "index.sqlite"), filePath };
}

function line(type: string, payload: Record<string, unknown>): string {
  return JSON.stringify({
    timestamp: new Date("2026-04-22T00:00:00.000Z").toISOString(),
    type,
    payload,
  });
}

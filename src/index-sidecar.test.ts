import { afterEach, describe, expect, test } from "vitest";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { getLastCoverageProbeStats } from "./coverage-freshness";
import { INDEX_VERSION } from "./env";
import {
  indexSidecarPathForDb,
  loadIndexMetadata,
  readIndexSidecar,
} from "./index-sidecar";
import { syncSessions } from "./indexer";
import { collectStatus } from "./status";

const tempDirs: string[] = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

describe("index sidecar", () => {
  test("maps index.sqlite to index.meta.json beside the body database", () => {
    const base = mkdtempSync(join(tmpdir(), "cxs-sidecar-path-"));
    tempDirs.push(base);
    expect(indexSidecarPathForDb(join(base, "index.sqlite"))).toBe(join(base, "index.meta.json"));
    expect(indexSidecarPathForDb(join(base, "test.db"))).toBe(join(base, "test.meta.json"));
  });

  test("sync writes a sidecar that status uses without opening SQLite", async () => {
    const { root, dbPath } = writeCodexFixture("sidecar-hit");

    await syncSessions({ dbPath, selector: { kind: "all", root } });

    const sidecarPath = indexSidecarPathForDb(dbPath);
    expect(existsSync(sidecarPath)).toBe(true);
    const sidecar = readIndexSidecar(dbPath);
    expect(sidecar?.version).toBe(1);
    expect(sidecar?.indexVersion).toBe(INDEX_VERSION);
    expect(sidecar?.sources.codex?.coverageRecords).toHaveLength(1);
    expect(sidecar?.sources.codex?.fileMeta).toHaveLength(1);
    expect(sidecar?.sources.codex?.sessionCount).toBe(1);

    const status = await collectStatus({
      rootDir: root,
      dbPath,
      selector: { kind: "all", root },
    });
    const probe = getLastCoverageProbeStats();

    expect(status.requestedCoverage?.freshness).toBe("fresh");
    expect(status.index.exists).toBe(true);
    expect(status.index.sessionCount).toBe(1);
    expect(status.coverageCount).toBe(1);
    expect(probe?.dbOpens).toBe(0);
  });

  test("missing sidecar falls back to opening SQLite with the same proof", async () => {
    const { root, dbPath } = writeCodexFixture("sidecar-missing");
    await syncSessions({ dbPath, selector: { kind: "all", root } });
    rmSync(indexSidecarPathForDb(dbPath));

    const status = await collectStatus({
      rootDir: root,
      dbPath,
      selector: { kind: "all", root },
    });
    const probe = getLastCoverageProbeStats();

    expect(status.requestedCoverage?.freshness).toBe("fresh");
    expect(probe?.dbOpens).toBe(1);
  });

  test("stale sidecar identity falls back to SQLite", async () => {
    const { root, dbPath } = writeCodexFixture("sidecar-stale");
    await syncSessions({ dbPath, selector: { kind: "all", root } });

    const sidecarPath = indexSidecarPathForDb(dbPath);
    const parsed = JSON.parse(readFileSync(sidecarPath, "utf8")) as { dbIdentity: { sqliteSize: number } };
    parsed.dbIdentity.sqliteSize += 1;
    writeFileSync(sidecarPath, `${JSON.stringify(parsed)}\n`);

    const metadata = await loadIndexMetadata(dbPath, "codex");
    expect(metadata.opened).toBe(true);
    expect(metadata.coverageRecords).toHaveLength(1);

    const status = await collectStatus({
      rootDir: root,
      dbPath,
      selector: { kind: "all", root },
    });
    expect(status.requestedCoverage?.freshness).toBe("fresh");
    expect(getLastCoverageProbeStats()?.dbOpens).toBe(1);
  });

  test("empty WAL leftover does not invalidate a matching sidecar", async () => {
    const { root, dbPath } = writeCodexFixture("sidecar-empty-wal");
    await syncSessions({ dbPath, selector: { kind: "all", root } });
    writeFileSync(`${dbPath}-wal`, "");

    const sidecar = readIndexSidecar(dbPath);
    expect(sidecar).not.toBeNull();

    const status = await collectStatus({
      rootDir: root,
      dbPath,
      selector: { kind: "all", root },
    });
    expect(status.requestedCoverage?.freshness).toBe("fresh");
    expect(getLastCoverageProbeStats()?.dbOpens).toBe(0);
  });

  test("corrupt sidecar falls back to SQLite", async () => {
    const { root, dbPath } = writeCodexFixture("sidecar-corrupt");
    await syncSessions({ dbPath, selector: { kind: "all", root } });
    writeFileSync(indexSidecarPathForDb(dbPath), "{not-json");

    const status = await collectStatus({
      rootDir: root,
      dbPath,
      selector: { kind: "all", root },
    });
    expect(status.requestedCoverage?.freshness).toBe("fresh");
    expect(getLastCoverageProbeStats()?.dbOpens).toBe(1);
  });
});

function writeCodexFixture(name: string): { root: string; dbPath: string } {
  const base = mkdtempSync(join(tmpdir(), `cxs-sidecar-${name}-`));
  tempDirs.push(base);
  const root = join(base, "sessions");
  const day = join(root, "2026", "04", "22");
  mkdirSync(day, { recursive: true });
  writeFileSync(
    join(day, "rollout-2026-04-22T10-00-00-11111111-1111-4111-8111-111111111111.jsonl"),
    [
      line("session_meta", { id: "11111111-1111-4111-8111-111111111111", cwd: `/tmp/${name}` }),
      line("event_msg", { type: "user_message", message: `${name} user message` }),
      line("event_msg", { type: "agent_message", message: `${name} agent reply` }),
    ].join("\n"),
  );
  return { root, dbPath: join(base, "index.sqlite") };
}

function line(type: string, payload: Record<string, unknown>): string {
  return JSON.stringify({
    timestamp: new Date("2026-04-22T00:00:00.000Z").toISOString(),
    type,
    payload,
  });
}

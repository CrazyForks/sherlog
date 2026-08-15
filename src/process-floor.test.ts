import { afterEach, describe, expect, test } from "vitest";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { getLastCoverageProbeStats } from "./coverage-freshness";
import { INDEX_VERSION } from "./env";
import {
  INDEX_SIDECAR_VERSION,
  loadIndexMetadata,
  writeIndexSidecar,
} from "./index-sidecar";
import { collectStatus } from "./status";

const tempDirs: string[] = [];

afterEach(() => {
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true });
  }
});

describe("process floor: status does not open the sqlite db", () => {
  test("sidecar-backed status never opens the db until a connection is opened", async () => {
    const { root, dbPath } = writeSidecarOnlyFixture();

    const { openWriteDb, sqliteNativeModuleLoaded } = await import("./db/connection");
    expect(sqliteNativeModuleLoaded()).toBe(false);

    const metadata = await loadIndexMetadata(dbPath, "codex");
    expect(metadata.opened).toBe(false);
    expect(metadata.index.exists).toBe(true);
    expect(metadata.index.sessionCount).toBe(0);

    const status = await collectStatus({
      rootDir: root,
      dbPath,
      selector: { kind: "all", root },
    });
    expect(status.index.exists).toBe(true);
    expect(getLastCoverageProbeStats()?.dbOpens).toBe(0);
    expect(sqliteNativeModuleLoaded()).toBe(false);

    const db = openWriteDb(join(root, "..", "opened.sqlite"));
    db.close();
    expect(sqliteNativeModuleLoaded()).toBe(true);
  });
});

function writeSidecarOnlyFixture(): { root: string; dbPath: string } {
  const base = mkdtempSync(join(tmpdir(), "cxs-process-floor-"));
  tempDirs.push(base);
  const root = join(base, "sessions");
  mkdirSync(root, { recursive: true });
  const dbPath = join(base, "index.sqlite");
  writeFileSync(dbPath, "");
  writeIndexSidecar(dbPath, {
    version: INDEX_SIDECAR_VERSION,
    indexVersion: INDEX_VERSION,
    sources: {
      codex: {
        sessionCount: 0,
        messageCount: 0,
        earliestStartedAt: null,
        latestEndedAt: null,
        lastSyncAt: null,
        coverageRecords: [],
        fileMeta: [],
      },
    },
  });
  return { root, dbPath };
}

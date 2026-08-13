import { describe, expect, it, vi } from "vitest";
import {
  createCachedSnapshotter,
  emptyCoverageProbeStats,
  finishCoverageProbe,
  getLastCoverageProbeStats,
  proveRequestedCoverage,
} from "./coverage-freshness";
import { INDEX_VERSION } from "./env";
import type { CoverageRecord, Selector, SourceSnapshot } from "./types";

describe("coverage probe", () => {
  const root = "/tmp/sherlog-coverage-probe";
  const allSelector: Selector = { source: "codex", kind: "all", root };
  const cwdSelector = (cwd: string): Selector => ({ source: "codex", kind: "cwd", root, cwd });

  it("snapshots a covering all(root) once despite many historical cwd rows", async () => {
    const allSnapshot = snapshotFor(allSelector, "all-fp", "all-set", 3);
    const records = [
      coverageRecord(allSelector, allSnapshot),
      ...Array.from({ length: 12 }, (_, index) =>
        coverageRecord(cwdSelector(`/tmp/project-${index}`), snapshotFor(cwdSelector(`/tmp/project-${index}`), `cwd-${index}`, `cwd-set-${index}`, 1)),
      ),
    ];
    const stats = emptyCoverageProbeStats();
    const snapshotFromFiles = vi.fn(async (selector: Selector) => {
      if (selector.kind === "all") return allSnapshot;
      throw new Error(`unexpected snapshot of ${selector.kind}`);
    });
    const snapshotForSelector = createCachedSnapshotter(snapshotFromFiles, [], stats);

    const requested = await snapshotForSelector(allSelector);
    const proof = await proveRequestedCoverage(requested, records, snapshotForSelector);

    expect(proof.freshness).toBe("fresh");
    expect(proof.coveringSelectors).toHaveLength(1);
    expect(proof.coveringSelectors[0]?.selector.kind).toBe("all");
    expect(snapshotFromFiles).toHaveBeenCalledTimes(1);
    expect(stats.snapshotCalls).toBe(1);
  });

  it("reuses the requested snapshot when the covering selector is the same", async () => {
    const cwd = cwdSelector("/tmp/same");
    const snap = snapshotFor(cwd, "cwd-fp", "cwd-set", 1);
    const stats = emptyCoverageProbeStats();
    const snapshotFromFiles = vi.fn(async () => snap);
    const snapshotForSelector = createCachedSnapshotter(snapshotFromFiles, [], stats);

    const requested = await snapshotForSelector(cwd);
    const proof = await proveRequestedCoverage(requested, [coverageRecord(cwd, snap)], snapshotForSelector);

    expect(proof.freshness).toBe("fresh");
    expect(snapshotFromFiles).toHaveBeenCalledTimes(1);
    expect(stats.snapshotCalls).toBe(1);
  });

  it("writes probe timing to stderr only when SHLOG_DEBUG_TIMING is on", () => {
    const stats = emptyCoverageProbeStats();
    stats.snapshotCalls = 1;
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const previous = process.env.SHLOG_DEBUG_TIMING;
    try {
      delete process.env.SHLOG_DEBUG_TIMING;
      finishCoverageProbe("status", stats);
      expect(errorSpy).not.toHaveBeenCalled();
      expect(getLastCoverageProbeStats()?.snapshotCalls).toBe(1);

      process.env.SHLOG_DEBUG_TIMING = "1";
      finishCoverageProbe("status", stats);
      expect(errorSpy).toHaveBeenCalledTimes(1);
      expect(String(errorSpy.mock.calls[0]?.[0])).toContain("snapshotCalls=1");
    } finally {
      errorSpy.mockRestore();
      if (previous === undefined) delete process.env.SHLOG_DEBUG_TIMING;
      else process.env.SHLOG_DEBUG_TIMING = previous;
    }
  });
});

function snapshotFor(
  selector: Selector,
  fingerprint: string,
  fileSetFingerprint: string,
  fileCount: number,
): SourceSnapshot {
  return {
    selector,
    fingerprint,
    fileSetFingerprint,
    fileCount,
    files: [],
  };
}

function coverageRecord(selector: Selector, snapshot: SourceSnapshot): CoverageRecord {
  return {
    id: 1,
    selector,
    sourceFingerprint: snapshot.fingerprint,
    sourceFileSetFingerprint: snapshot.fileSetFingerprint,
    sourceFileCount: snapshot.fileCount,
    indexedSessionCount: snapshot.fileCount,
    completedAt: "2026-08-13T00:00:00.000Z",
    indexVersion: INDEX_VERSION,
  };
}

import { describe, expect, test } from "vitest";
import {
  DEFAULT_LEVELS,
  DEFAULT_SHAPES,
  DEFAULT_TOTAL_PER_LEVEL,
  aggregateLevelStats,
  buildConcurrencyReportMarkdown,
  parseConcurrencyArgs,
  parseLevels,
  parsePositiveInt,
  parseShapes,
  shapeCommand,
  type ConcurrencyReport,
  type OpSample,
} from "./concurrency-bench-core";

describe("concurrency shape parsing", () => {
  test("bare tokens become find shapes", () => {
    expect(parseShapes("envchain|edge tts")).toEqual(["find:envchain", "find:edge tts"]);
  });

  test("keeps explicit shapes and mixes with find", () => {
    expect(parseShapes("read-range|find:部署 health check|status")).toEqual([
      "read-range",
      "find:部署 health check",
      "status",
    ]);
  });

  test("empty input falls back to defaults", () => {
    expect(parseShapes("")).toEqual(DEFAULT_SHAPES);
  });

  test("level list is deduped and sorted; empty falls back", () => {
    expect(parseLevels("8 1 4 8")).toEqual([1, 4, 8]);
    expect(parseLevels("")).toEqual(DEFAULT_LEVELS);
  });

  test("positive int parser falls back on garbage", () => {
    expect(parsePositiveInt("42", 7)).toBe(42);
    expect(parsePositiveInt("0", 7)).toBe(7);
    expect(parsePositiveInt("abc", 7)).toBe(7);
    expect(parsePositiveInt(undefined, 7)).toBe(7);
  });
});

describe("concurrency arg parsing", () => {
  test("requires --db", () => {
    expect(() => parseConcurrencyArgs([])).toThrow(/--db is required/);
    expect(() => parseConcurrencyArgs(["--root", "/tmp/root"])).toThrow(/--db is required/);
  });

  test("parses overrides and defaults", () => {
    const args = parseConcurrencyArgs([
      "--db", "/tmp/index.sqlite",
      "--root", "/tmp/sessions",
      "--source", "claude-code",
      "--shapes", "envchain|status",
      "--levels", "1 4 16",
      "--total", "40",
      "--json-only",
    ]);
    expect(args.db).toBe("/tmp/index.sqlite");
    expect(args.root).toBe("/tmp/sessions");
    expect(args.source).toBe("claude-code");
    expect(args.shapes).toEqual(["find:envchain", "status"]);
    expect(args.levels).toEqual([1, 4, 16]);
    expect(args.totalPerLevel).toBe(40);
    expect(args.jsonOnly).toBe(true);
    // No executable override: resolves to the TypeScript reference by default.
    expect(args.commandUnderTest.source).toBe("typescript-reference");
  });

  test("accepts explicit executable override", () => {
    const args = parseConcurrencyArgs(["--db", "/tmp/index.sqlite", "--bin", "/tmp/shlog"]);
    expect(args.commandUnderTest.source).not.toBe("typescript-reference");
  });
});

describe("shape command construction", () => {
  const ctx = { source: "codex", root: "/tmp/sessions", db: "/tmp/index.sqlite", sessionRef: "session-1" };

  test("find shape carries query and limit", () => {
    const cmd = shapeCommand("find:edge tts", ctx);
    expect(cmd).toEqual([
      "find", "edge tts", "--source", "codex", "--root", "/tmp/sessions",
      "--db", "/tmp/index.sqlite", "--limit", "10", "--json",
    ]);
  });

  test("status shape carries the all(root) selector", () => {
    const cmd = shapeCommand("status", ctx);
    expect(cmd?.[0]).toBe("status");
    expect(cmd).toContain("--selector");
    expect(JSON.parse(cmd![cmd!.indexOf("--selector") + 1]!)).toEqual({
      source: "codex", kind: "all", root: "/tmp/sessions",
    });
  });

  test("read shapes require a resolvable session ref", () => {
    expect(shapeCommand("read-range", ctx)).not.toBeNull();
    expect(shapeCommand("read-page", ctx)).not.toBeNull();
    expect(shapeCommand("read-range", { ...ctx, sessionRef: null })).toBeNull();
    expect(shapeCommand("read-page", { ...ctx, sessionRef: null })).toBeNull();
  });

  test("unknown shape throws", () => {
    expect(() => shapeCommand("list", ctx)).toThrow(/unknown shape/);
  });
});

describe("level aggregation", () => {
  test("computes percentiles, throughput and error count", () => {
    const samples: OpSample[] = [
      sample(10, 8, true), sample(20, 15, true), sample(30, 22, true),
      sample(40, 30, true), sample(50, 38, true), sample(999, null, false),
    ];
    const stats = aggregateLevelStats(4, 6, 300, samples);
    expect(stats.level).toBe(4);
    expect(stats.total).toBe(6);
    expect(stats.errors).toBe(1);
    expect(stats.throughputPerSec).toBe(20); // 6 ops / 0.3s
    // E2E p50/p95/p99/max over [10,20,30,40,50,999] (R-7 linear interpolation)
    expect(stats.p50E2E).toBe(35);
    expect(stats.p95E2E).toBeCloseTo(761.75, 1);
    expect(stats.p99E2E).toBeCloseTo(951.55, 1);
    expect(stats.maxE2E).toBe(999);
    // op samples exclude the failed op (no elapsedMs): [8,15,22,30,38]
    expect(stats.opSampleCount).toBe(5);
    expect(stats.p50Op).toBe(22);
    expect(stats.p95Op).toBeCloseTo(36.4, 1);
  });

  test("handles empty sample set without NaN", () => {
    const stats = aggregateLevelStats(1, 0, 1, []);
    expect(stats.p50E2E).toBe(0);
    expect(stats.p95E2E).toBe(0);
    expect(stats.maxE2E).toBe(0);
    expect(stats.opSampleCount).toBe(0);
    expect(stats.throughputPerSec).toBe(0);
  });
});

describe("markdown report", () => {
  test("renders shape tables without NaN", () => {
    const report: ConcurrencyReport = {
      generatedAt: "2026-08-17T00:00:00.000Z",
      commandUnderTest: {
        executable: "shlog",
        prefixArgv: [],
        source: "argv-json",
        resolvedExecutablePath: "/tmp/shlog",
        executableSizeBytes: 100,
        artifactPath: null,
        artifactSizeBytes: null,
      },
      sourceId: "codex",
      dbPath: "/tmp/index.sqlite",
      rootDir: "/tmp/sessions",
      sessionCount: 10,
      messageCount: 100,
      totalPerLevel: 2,
      shapes: [
        {
          shape: "find:envchain",
          command: ["find", "envchain", "--json"],
          levels: [
            {
              level: 1, total: 2, wallMs: 20, throughputPerSec: 100, errors: 0,
              p50E2E: 10, p95E2E: 15, p99E2E: 18, maxE2E: 20,
              p50Op: 8, p95Op: 12, p99Op: 14, maxOp: 16, opSampleCount: 2,
            },
          ],
        },
      ],
    };
    const md = buildConcurrencyReportMarkdown(report);
    expect(md).toContain("## find:envchain");
    expect(md).toContain("| 1 |");
    expect(md).not.toContain("NaN");
    expect(md).toContain("shlog 并发性能基准报告");
  });
});

function sample(e2eMs: number, opMs: number | null, ok: boolean): OpSample {
  return { ok, exitCode: ok ? 0 : 1, e2eMs, opMs, stdoutLen: 0, stderr: "" };
}

import { basename, dirname, resolve } from "node:path";
import { describe, expect, test } from "vitest";
import {
  DEFAULT_TOTAL_RUNS,
  commandArgv,
  findCommandArgv,
  parsePeakRssBytes,
  percentile,
  resolveCommandUnderTest,
  resourceSamplerCommand,
  timingStats,
} from "./perf-bench-core";

const ROOT = resolve(import.meta.dirname, "..");
const CLI_ENTRY = resolve(ROOT, "src", "cli.ts");

describe("perf benchmark command target", () => {
  test("keeps the current TSX source command as the default", () => {
    const target = resolveCommandUnderTest({ root: ROOT, cliEntry: CLI_ENTRY, env: {} });

    expect(target.executable).toBe(process.execPath);
    expect(target.prefixArgv).toEqual([
      "--disable-warning=ExperimentalWarning",
      "--import",
      "tsx",
      CLI_ENTRY,
    ]);
    expect(target.source).toBe("typescript-reference");
    expect(target.artifactPath).toBe(CLI_ENTRY);
    expect(target.artifactSizeBytes).toBeGreaterThan(0);
  });

  test("accepts a full JSON command prefix from the shared eval environment", () => {
    const target = resolveCommandUnderTest({
      root: ROOT,
      cliEntry: CLI_ENTRY,
      env: {
        PATH: dirname(process.execPath),
        SHLOG_BIN_UNDER_TEST: "/tmp/lower-precedence-shlog",
        SHLOG_CLI_ARGV_JSON: JSON.stringify([basename(process.execPath), "dist/cli.js", "--trace-warnings"]),
        SHLOG_ARTIFACT_UNDER_TEST: "package.json",
      },
    });

    expect(target.source).toBe("argv-json");
    expect(target.resolvedExecutablePath).toBe(process.execPath);
    expect(target.prefixArgv).toEqual(["dist/cli.js", "--trace-warnings"]);
    expect(target.artifactPath).toBe(resolve(ROOT, "package.json"));
    expect(target.artifactSizeBytes).toBeGreaterThan(0);
  });

  test("rejects shell-like argv strings instead of guessing how to split them", () => {
    expect(() => resolveCommandUnderTest({
      root: ROOT,
      cliEntry: CLI_ENTRY,
      env: {
        SHLOG_CLI_ARGV_JSON: "--flag value",
      },
    })).toThrow(/non-empty string array/);
  });

  test("builds commands without a shell and propagates root to find", () => {
    const target = resolveCommandUnderTest({
      root: ROOT,
      cliEntry: CLI_ENTRY,
      argvJson: JSON.stringify(["/tmp/custom-shlog", "--launcher-mode"]),
      env: {},
    });

    expect(commandArgv(target, "status", "--json")).toEqual([
      "/tmp/custom-shlog",
      "--launcher-mode",
      "status",
      "--json",
    ]);
    expect(findCommandArgv(target, "needle", "codex", "/tmp/index.sqlite", "/tmp/sessions")).toEqual([
      "/tmp/custom-shlog",
      "--launcher-mode",
      "find",
      "needle",
      "--source",
      "codex",
      "--root",
      "/tmp/sessions",
      "--db",
      "/tmp/index.sqlite",
      "--limit",
      "10",
      "--json",
    ]);
  });
});

describe("perf benchmark statistics", () => {
  test("defaults to one warmup plus twenty measured samples", () => {
    expect(DEFAULT_TOTAL_RUNS).toBe(21);
  });

  test("drops the warmup and calculates an interpolated p95", () => {
    const stats = timingStats([999, ...Array.from({ length: 20 }, (_, index) => index + 1)]);

    expect(stats.runs).toBe(20);
    expect(stats.warmupMs).toBe(999);
    expect(stats.samplesMs).toEqual(Array.from({ length: 20 }, (_, index) => index + 1));
    expect(stats.p50Ms).toBe(10.5);
    expect(stats.p95Ms).toBe(19.05);
    expect(stats.p95Ms).toBeLessThan(20);
  });

  test("uses the single sample when no separate warmup exists", () => {
    expect(timingStats([12.345])).toEqual({
      runs: 1,
      samplesMs: [12.35],
      warmupMs: null,
      p50Ms: 12.35,
      p95Ms: 12.35,
    });
    expect(percentile([], 0.95)).toBe(0);
  });
});

describe("perf benchmark RSS probes", () => {
  test("parses Darwin and GNU time output into bytes", () => {
    expect(parsePeakRssBytes("  70909952  maximum resident set size\n", "time-darwin")).toBe(70_909_952);
    expect(parsePeakRssBytes("Maximum resident set size (kbytes): 12345\n", "time-gnu")).toBe(12_641_280);
    expect(parsePeakRssBytes("no resource data", "time-darwin")).toBeNull();
  });

  test("only enables known /usr/bin/time formats", () => {
    expect(resourceSamplerCommand(["shlog", "status"], "darwin")?.command).toEqual([
      "/usr/bin/time",
      "-l",
      "shlog",
      "status",
    ]);
    expect(resourceSamplerCommand(["shlog", "status"], "win32")).toBeNull();
  });
});

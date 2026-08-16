import { beforeAll, describe, expect, test } from "vitest";
import {
  normalizeContractValue,
  resolveContractExecutables,
  runContractGate,
  type ContractGateResult,
} from "./contract-gate";

describe("executable-neutral contract gate", { timeout: 120_000 }, () => {
  let result: ContractGateResult;

  beforeAll(async () => {
    const env = { ...process.env };
    delete env.SHLOG_BIN_UNDER_TEST;
    delete env.SHLOG_CLI_ARGV_JSON;
    result = await runContractGate({ env });
  }, 120_000);

  test("deep-compares the complete public CLI surface against the TypeScript reference", () => {
    expect(result.referenceCli.source).toBe("typescript-reference");
    expect(result.candidateCli.source).toBe("typescript-reference");
    expect(result).toMatchObject({ total: 27, passed: 27, failed: 0 });
    expect(result.cases.map((entry) => entry.id)).toEqual([
      "version",
      "help",
      "error-missing-find-query",
      "status-empty-index",
      "sync-strict-codex",
      "sync-strict-claude-code",
      "sync-strict-pi",
      "status-indexed-selector",
      "cold-add",
      "cold-list",
      "cold-remove",
      "find-single-source",
      "find-all-sources",
      "find-unscoped-default-root",
      "read-range",
      "error-anchor-not-found",
      "read-page",
      "list",
      "stats",
      "error-unknown-source",
      "error-invalid-selector",
      "error-index-unavailable",
      "error-session-not-found",
      "sync-strict-error",
      "sync-best-effort",
      "sync-prune",
      "status-destructive-change",
    ]);
    expect(result.cases.every((entry) => entry.mismatchPaths.length === 0)).toBe(true);
  });

  test("keeps runtime normalization narrowly allowlisted", () => {
    const value = {
      elapsedMs: 19,
      completedAt: "2026-08-15T10:00:00.000Z",
      lastSyncAt: "2026-08-15T10:00:01.000Z",
      addedAt: "2026-08-15T10:00:02.000Z",
      dbSizeBytes: 4096,
      startedAt: "2026-08-15T01:00:00.000Z",
      sourceFingerprint: "must-remain-exact",
      id: 7,
      path: "/private/tmp/reference/main.sqlite",
    };

    expect(normalizeContractValue(value, [{ from: "/private/tmp/reference", to: "<STATE>" }])).toEqual({
      elapsedMs: "<ELAPSED_MS>",
      completedAt: "<RUNTIME_TIMESTAMP>",
      lastSyncAt: "<RUNTIME_TIMESTAMP>",
      addedAt: "<RUNTIME_TIMESTAMP>",
      dbSizeBytes: "<SQLITE_FILE_SIZE>",
      startedAt: "2026-08-15T01:00:00.000Z",
      sourceFingerprint: "must-remain-exact",
      id: 7,
      path: "<STATE>/main.sqlite",
    });
  });

  test("keeps the reference immune to ambient candidate overrides", () => {
    const commands = resolveContractExecutables({
      candidateArgvJson: JSON.stringify(["cargo", "run", "--quiet", "--"]),
      env: { SHLOG_BIN_UNDER_TEST: "/tmp/ignored-candidate" },
    });

    expect(commands.reference.source).toBe("typescript-reference");
    expect(commands.candidate).toEqual({
      source: "argv-json",
      argv: ["cargo", "run", "--quiet", "--"],
    });
  });

  test("can require an explicit candidate instead of silently self-comparing", async () => {
    await expect(runContractGate({ requireCandidateOverride: true, env: {} })).rejects.toThrow(
      "requires an explicit candidate executable override",
    );
  });
});

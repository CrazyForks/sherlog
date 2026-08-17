import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { beforeAll, describe, expect, test } from "vitest";
import { runAcceptanceGate, type AcceptanceGateResult } from "./acceptance-gate";

const ROOT = resolve(import.meta.dirname, "..");
const hasCheckoutShlog = ["target/release/shlog", "target/debug/shlog"]
  .some((rel) => existsSync(resolve(ROOT, rel)));

describe.skipIf(!hasCheckoutShlog)("acceptance gate", () => {
  let result: AcceptanceGateResult;

  beforeAll(async () => {
    result = await runAcceptanceGate();
  }, 60_000);

  test("passes synthetic evidence-level retrieval fixtures through the CLI executable seam", () => {
    expect(result.cliUnderTest.argv.length).toBeGreaterThan(0);

    expect(result.sync.added).toBe(11);
    expect(result.sourceSyncs["claude-code"].added).toBe(1);
    expect(result.sourceSyncs.pi.added).toBe(1);
    expect(result.scoreboard).toMatchObject({
      total: 8,
      pass: 8,
      fail: 0,
      hardFail: 0,
      candidateFail: 0,
      assertionPass: 8,
      assertionFail: 0,
      facetPass: 1,
      facetFail: 0,
    });
    expect(result.rows.map((row) => row.id)).toEqual([
      "message-hit-context",
      "session-only-compact-context",
      "cjk-message-hit",
      "duplicate-family-diversity",
      "claude-code-message-range-context",
      "pi-session-page-context",
      "command-restatement-loses-to-execution",
      "query-window-keeps-table-rows",
    ]);
    expect(result.rows.every((row) => row.predicates.length > 0)).toBe(true);
    expect(result.rows.find((row) => row.id === "message-hit-context")?.facetMark).toBe("pass");
    expect(result.returnedContext.reads).toBe(6);
    expect(result.returnedContext.charsP50).toBeGreaterThan(0);
    expect(result.rows.find((row) => row.id === "query-window-keeps-table-rows")?.returnedContext.read).toBe(true);
    expect(result.rows.find((row) => row.id === "command-restatement-loses-to-execution")?.returnedContext.read).toBe(false);
  });

  test("reports top-result diversity metrics for the duplicate-family case", () => {
    const diversityRow = result.rows.find((row) => row.id === "duplicate-family-diversity");

    expect(diversityRow).toBeDefined();
    expect(diversityRow?.blocking).toBe(false);
    // One row per session is structural; the family shows up as collapsed
    // title/cwd diversity, which is exactly what a future ranking change
    // would have to improve with a failing-before/passing-after eval.
    expect(diversityRow?.diversity).toEqual({
      topK: 4,
      resultCount: 4,
      distinctSessions: 4,
      distinctTitles: 2,
      distinctCwds: 2,
    });
    // Dense within-session follow-up stays available through result metadata.
    expect(result.rows.every((row) => row.diversity.topK > 0)).toBe(true);
  });
});

describe("acceptance gate helpers", () => {
  test("can require an explicit candidate instead of silently testing the checkout binary", async () => {
    await expect(runAcceptanceGate({ requireCandidateOverride: true })).rejects.toThrow(
      "requires an explicit candidate executable override",
    );
  });
});

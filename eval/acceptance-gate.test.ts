import { describe, expect, test } from "vitest";
import { runAcceptanceGate } from "./acceptance-gate";

describe("acceptance gate", () => {
  test("passes synthetic evidence-level retrieval fixtures", async () => {
    const result = await runAcceptanceGate();

    expect(result.sync.added).toBe(8);
    expect(result.sourceSyncs["claude-code"].added).toBe(1);
    expect(result.sourceSyncs.pi.added).toBe(1);
    expect(result.scoreboard).toMatchObject({
      total: 6,
      pass: 6,
      fail: 0,
      hardFail: 0,
      candidateFail: 0,
      assertionPass: 6,
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
    ]);
    expect(result.rows.every((row) => row.predicates.length > 0)).toBe(true);
    expect(result.rows.find((row) => row.id === "message-hit-context")?.facetMark).toBe("pass");
  });

  test("reports top-result diversity metrics for the duplicate-family case", async () => {
    const result = await runAcceptanceGate();
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

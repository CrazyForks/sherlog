import { describe, expect, test } from "vitest";
import { buildRelaxedRecallQueries, buildZeroResultRefinement } from "./relaxed-recall";

describe("buildRelaxedRecallQueries", () => {
  test("keeps technical English terms from mixed-language questions", () => {
    expect(buildRelaxedRecallQueries("最近一个星期有没有触发过 multi agent")).toEqual([
      "multi agent",
      "multi agents",
    ]);
  });

  test("expands simple dash and underscore variants", () => {
    expect(buildRelaxedRecallQueries("有没有用过 multi_agents")).toContain("multi agents");
    expect(buildRelaxedRecallQueries("有没有用过 multi-agents")).toContain("multi agent");
  });

  test("does not relax already concise English searches", () => {
    expect(buildRelaxedRecallQueries("health check")).toEqual([]);
  });
});

describe("buildZeroResultRefinement", () => {
  test("flags long AND-combined queries and suggests distinctive single terms", () => {
    const refinement = buildZeroResultRefinement("kubernetes ingress controller timeout retry");
    expect(refinement.overConstrained).toBe(true);
    expect(refinement.suggestedQueries.length).toBeGreaterThan(0);
    expect(refinement.suggestedQueries).toContain("kubernetes");
    expect(refinement.hints.some((hint) => hint.includes("AND-combines"))).toBe(true);
  });

  test("flags mixed Chinese/English queries and suggests both scripts separately", () => {
    const refinement = buildZeroResultRefinement("部署 healthcheck 失败");
    expect(refinement.overConstrained).toBe(true);
    expect(refinement.suggestedQueries).toContain("healthcheck");
    expect(refinement.suggestedQueries).toContain("部署");
    expect(refinement.hints.some((hint) => hint.includes("Mixed Chinese/English"))).toBe(true);
  });

  test("keeps concise single-term queries unflagged with no redundant suggestions", () => {
    const refinement = buildZeroResultRefinement("tailscale");
    expect(refinement.overConstrained).toBe(false);
    expect(refinement.suggestedQueries).toEqual([]);
  });
});

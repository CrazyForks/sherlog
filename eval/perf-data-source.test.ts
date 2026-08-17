import { describe, expect, test } from "vitest";
import { PerfDataSourceError, resolvePerfWorkload } from "./perf-data-source";

describe("resolvePerfWorkload", () => {
  test("omitting both paths selects synthetic smoke", () => {
    expect(resolvePerfWorkload({ explicitRoot: false, explicitDb: false })).toEqual({
      kind: "synthetic_smoke",
    });
  });

  test("omitting both paths still allows --fixture-mb", () => {
    expect(resolvePerfWorkload({
      explicitRoot: false,
      explicitDb: false,
      fixtureMbExplicit: true,
    })).toEqual({ kind: "synthetic_smoke" });
  });

  test("both paths select private calibration", () => {
    expect(resolvePerfWorkload({ explicitRoot: true, explicitDb: true })).toEqual({
      kind: "private_calibration",
    });
  });

  test("only --root is rejected", () => {
    expect(() => resolvePerfWorkload({ explicitRoot: true, explicitDb: false }))
      .toThrow(PerfDataSourceError);
    expect(() => resolvePerfWorkload({ explicitRoot: true, explicitDb: false }))
      .toThrow(/both --root and --db/);
  });

  test("only --db is rejected", () => {
    expect(() => resolvePerfWorkload({ explicitRoot: false, explicitDb: true }))
      .toThrow(/both --root and --db/);
  });

  test("mixing --fixture-mb with both real paths is rejected", () => {
    expect(() => resolvePerfWorkload({
      explicitRoot: true,
      explicitDb: true,
      fixtureMbExplicit: true,
    })).toThrow(/do not mix --fixture-mb/);
  });
});

import { describe, expect, test } from "vitest";
import { measureReturnedContext, summarizeReturnedContext } from "./returned-context";

describe("returned context metric", () => {
  test("measures utf8 bytes separately from chars", () => {
    const metric = measureReturnedContext({ kind: "read-range", text: "回滚预案" });
    expect(metric).toEqual({
      read: true,
      kind: "read-range",
      chars: 4,
      bytes: 12,
    });
  });

  test("treats a skipped or failed read as unread", () => {
    expect(measureReturnedContext({ unavailableReason: "no selected hit for context read" })).toEqual({
      read: false,
      kind: null,
      chars: 0,
      bytes: 0,
    });
    expect(measureReturnedContext({})).toEqual({
      read: false,
      kind: null,
      chars: 0,
      bytes: 0,
    });
  });

  test("summarizes only successful reads so scorecards can spot context-budget drift", () => {
    const summary = summarizeReturnedContext([
      measureReturnedContext({}),
      measureReturnedContext({ kind: "read-range", text: "a".repeat(100) }),
      measureReturnedContext({ kind: "read-range", text: "b".repeat(200) }),
      measureReturnedContext({ kind: "read-page", text: "c".repeat(400) }),
    ]);
    expect(summary).toEqual({
      reads: 3,
      charsP50: 200,
      charsP95: 400,
      charsMax: 400,
      bytesMax: 400,
    });
  });
});

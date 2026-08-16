import { describe, expect, test } from "vitest";
import { buildEvidenceReadAction, type EvidenceReadContext } from "./evidence-read";

const context: EvidenceReadContext = { dbPath: "/state/index.sqlite", json: true };

describe("buildEvidenceReadAction", () => {
  test("message hits resolve to a bounded read-range command closed over db/json/source", () => {
    expect(buildEvidenceReadAction({
      sourceId: "codex",
      sessionRef: "11111111-1111-4111-8111-111111111111",
      matchSeq: 7,
      query: "ranking weights",
    }, context)).toEqual({
      kind: "read-range",
      reason: "message_match",
      sourceId: "codex",
      sessionRef: "11111111-1111-4111-8111-111111111111",
      seq: 7,
      query: "ranking weights",
      before: 2,
      after: 2,
      command: {
        executable: "inherit",
        args: [
          "read-range",
          "11111111-1111-4111-8111-111111111111",
          "--seq",
          "7",
          "--before",
          "2",
          "--after",
          "2",
          "--query",
          "ranking weights",
          "--source",
          "codex",
          "--db",
          "/state/index.sqlite",
          "--json",
        ],
        sideEffect: "read_index",
      },
    });
  });

  test("session-only hits with query resolve to read-range --query (finds real anchor, not offset=0)", () => {
    expect(buildEvidenceReadAction({
      sourceId: "claude-code",
      sessionRef: "claude-code:session-abc",
      matchSeq: null,
      query: "durable output queue",
    }, context)).toEqual({
      kind: "read-range",
      reason: "session_level_match",
      sourceId: "claude-code",
      sessionRef: "claude-code:session-abc",
      query: "durable output queue",
      before: 2,
      after: 2,
      command: {
        executable: "inherit",
        args: [
          "read-range",
          "claude-code:session-abc",
          "--query",
          "durable output queue",
          "--before",
          "2",
          "--after",
          "2",
          "--source",
          "claude-code",
          "--db",
          "/state/index.sqlite",
          "--json",
        ],
        sideEffect: "read_index",
      },
    });
  });

  test("session-only hits without query fall back to read-page offset=0", () => {
    expect(buildEvidenceReadAction({
      sourceId: "claude-code",
      sessionRef: "claude-code:session-abc",
      matchSeq: null,
    }, context)).toEqual({
      kind: "read-page",
      reason: "session_level_match",
      sourceId: "claude-code",
      sessionRef: "claude-code:session-abc",
      offset: 0,
      limit: 40,
      command: {
        executable: "inherit",
        args: [
          "read-page",
          "claude-code:session-abc",
          "--offset",
          "0",
          "--limit",
          "40",
          "--source",
          "claude-code",
          "--db",
          "/state/index.sqlite",
          "--json",
        ],
        sideEffect: "read_index",
      },
    });
  });
});

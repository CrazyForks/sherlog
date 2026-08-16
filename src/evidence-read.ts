import type { FindResult, SessionSourceId } from "./types";

const DEFAULT_READ_RANGE_BEFORE = 2;
const DEFAULT_READ_RANGE_AFTER = 2;
const DEFAULT_SESSION_PAGE_OFFSET = 0;
const DEFAULT_SESSION_PAGE_LIMIT = 40;

export interface EvidenceReadCommand {
  executable: "inherit";
  args: string[];
  sideEffect: "read_index";
}

export type EvidenceReadAction =
  | {
      kind: "read-range";
      reason: "message_match" | "session_level_match";
      sourceId: SessionSourceId;
      sessionRef: string;
      seq: number;
      query?: string;
      before: number;
      after: number;
      command: EvidenceReadCommand;
    }
  | {
      kind: "read-range";
      reason: "session_level_match";
      sourceId: SessionSourceId;
      sessionRef: string;
      query: string;
      before: number;
      after: number;
      command: EvidenceReadCommand;
    }
  | {
      kind: "read-page";
      reason: "session_level_match";
      sourceId: SessionSourceId;
      sessionRef: string;
      offset: number;
      limit: number;
      command: EvidenceReadCommand;
    };

export interface EvidenceReadContext {
  dbPath: string;
  json: boolean;
}

function command(args: string[]): EvidenceReadCommand {
  return { executable: "inherit", args, sideEffect: "read_index" };
}

export function buildEvidenceReadAction(
  result: Pick<FindResult, "sourceId" | "sessionRef" | "matchSeq"> & { query?: string },
  context: EvidenceReadContext,
): EvidenceReadAction {
  // Every generated command closes over the exact DB path, source qualifier,
  // and output mode that produced the candidate so a verbatim execution reads
  // the same projection regardless of PATH or default-state drift.
  const scope = [
    "--source",
    result.sourceId,
    "--db",
    context.dbPath,
    ...(context.json ? ["--json"] : []),
  ];
  if (result.matchSeq === null) {
    // Session-level hit: the match came from session metadata/compact, not a
    // specific message. When we have the query, point read-range at it so
    // resolveAnchorSeq can locate the real evidence anchor inside the session
    // transcript. Without a query we fall back to read-page from the start.
    if (result.query) {
      return {
        kind: "read-range",
        reason: "session_level_match",
        sourceId: result.sourceId,
        sessionRef: result.sessionRef,
        query: result.query,
        before: DEFAULT_READ_RANGE_BEFORE,
        after: DEFAULT_READ_RANGE_AFTER,
        command: command([
          "read-range",
          result.sessionRef,
          "--query",
          result.query,
          "--before",
          String(DEFAULT_READ_RANGE_BEFORE),
          "--after",
          String(DEFAULT_READ_RANGE_AFTER),
          ...scope,
        ]),
      };
    }

    return {
      kind: "read-page",
      reason: "session_level_match",
      sourceId: result.sourceId,
      sessionRef: result.sessionRef,
      offset: DEFAULT_SESSION_PAGE_OFFSET,
      limit: DEFAULT_SESSION_PAGE_LIMIT,
      command: command([
        "read-page",
        result.sessionRef,
        "--offset",
        String(DEFAULT_SESSION_PAGE_OFFSET),
        "--limit",
        String(DEFAULT_SESSION_PAGE_LIMIT),
        ...scope,
      ]),
    };
  }

  const argv = [
    "read-range",
    result.sessionRef,
    "--seq",
    String(result.matchSeq),
    "--before",
    String(DEFAULT_READ_RANGE_BEFORE),
    "--after",
    String(DEFAULT_READ_RANGE_AFTER),
  ];
  if (result.query) argv.push("--query", result.query);

  return {
    kind: "read-range",
    reason: "message_match",
    sourceId: result.sourceId,
    sessionRef: result.sessionRef,
    seq: result.matchSeq,
    ...(result.query ? { query: result.query } : {}),
    before: DEFAULT_READ_RANGE_BEFORE,
    after: DEFAULT_READ_RANGE_AFTER,
    command: command([...argv, ...scope]),
  };
}

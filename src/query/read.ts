import { coverageEntriesForSession, getMessagesForPage, getMessagesForRange, getSessionRecord, withSourceAwareReadDb } from "../db";
import { rerankHits } from "../ranking";
import type { FindMatchedField, FindResult, SessionRecord } from "../types";
import type { Db } from "../db";
import { queryTerms } from "../tokenize";
import { elideMessages } from "./message-elision";
import { searchMessageHits } from "./search";
import { SessionNotFoundError } from "./session-not-found";

export { SessionNotFoundError } from "./session-not-found";

export class AnchorNotFoundError extends Error {
  sessionRef: string;
  sourceId: string;
  nativeSessionId: string;
  query: string;
  matchedProfileFields: FindMatchedField[];

  constructor(session: SessionRecord, sessionRef: string, query: string, matchedProfileFields: FindMatchedField[]) {
    const detail = matchedProfileFields.length === 0
      ? `query "${query}" matched no message in session ${sessionRef}`
      : `query "${query}" matched only session-level fields (${matchedProfileFields.join(", ")}) in session ${sessionRef}; there is no message anchor`;
    super(detail);
    this.name = "AnchorNotFoundError";
    this.sessionRef = sessionRef;
    this.sourceId = session.sourceId;
    this.nativeSessionId = session.nativeSessionId;
    this.query = query;
    this.matchedProfileFields = matchedProfileFields;
  }
}

export function getMessageRange(
  dbPath: string,
  sessionUuid: string,
  options: { seq?: number; query?: string; before: number; after: number; maxMessageChars?: number },
): {
  session: SessionRecord;
  anchorSeq: number;
  rangeStartSeq: number;
  rangeEndSeq: number;
  messages: ReturnType<typeof getMessagesForRange>;
  coverage: { entries: ReturnType<typeof coverageEntriesForSession> };
} {
  return withSourceAwareReadDb(dbPath, (db) => {
    const session = getSessionRecord(db, sessionUuid);
    if (!session) throw new SessionNotFoundError(sessionUuid);
    const anchorSeq = resolveAnchorSeq(db, session, sessionUuid, options.seq, options.query);

    const rangeStartSeq = Math.max(0, anchorSeq - options.before);
    const rangeEndSeq = anchorSeq + options.after;
    const messages = elideMessages(getMessagesForRange(db, session.id, rangeStartSeq, rangeEndSeq), {
      anchorSeq,
      query: options.query,
      maxMessageChars: options.maxMessageChars,
    });
    return {
      session,
      anchorSeq,
      rangeStartSeq,
      rangeEndSeq,
      messages,
      coverage: { entries: coverageEntriesForSession(db, session) },
    };
  });
}

export function getMessagePage(
  dbPath: string,
  sessionUuid: string,
  offset: number,
  limit: number,
  options: { maxMessageChars?: number } = {},
): {
  session: SessionRecord;
  offset: number;
  limit: number;
  totalCount: number;
  hasMore: boolean;
  messages: ReturnType<typeof getMessagesForPage>;
  coverage: { entries: ReturnType<typeof coverageEntriesForSession> };
} {
  return withSourceAwareReadDb(dbPath, (db) => {
    const session = getSessionRecord(db, sessionUuid);
    if (!session) throw new SessionNotFoundError(sessionUuid);
    const messages = elideMessages(getMessagesForPage(db, session.id, offset, limit), {
      maxMessageChars: options.maxMessageChars,
    });
    const totalCount = session.messageCount;
    const hasMore = offset + messages.length < totalCount;
    return {
      session,
      offset,
      limit,
      totalCount,
      hasMore,
      messages,
      coverage: { entries: coverageEntriesForSession(db, session) },
    };
  });
}

function resolveAnchorSeq(
  db: Db,
  session: SessionRecord,
  sessionUuid: string,
  seq?: number,
  query?: string,
): number {
  if (typeof seq === "number") {
    return seq;
  }

  if (query) {
    const best = searchTopHitInSession(db, session, query);
    if (best && typeof best.matchSeq === "number") return best.matchSeq;
    // A session-level/profile-only match has no message anchor. Falling back
    // to seq=0 would present unrelated messages as matched evidence, so fail
    // closed with a typed error that names the matched profile fields.
    throw new AnchorNotFoundError(session, sessionUuid, query, matchedSessionFields(session, query));
  }

  throw new Error("read-range requires an explicit sessionRef plus either --seq or --query");
}

function searchTopHitInSession(db: Db, session: SessionRecord, query: string): FindResult | null {
  const rows = searchMessageHits(db, query, 20, session.id, null, { sourceId: session.sourceId });
  const result = rerankHits(rows, query, 1)[0];
  return result ?? null;
}

function matchedSessionFields(session: SessionRecord, query: string): FindMatchedField[] {
  const normalizedQuery = query.trim().toLowerCase();
  const terms = queryTerms(normalizedQuery);
  const candidates: Array<[FindMatchedField, string]> = [
    ["title", session.title],
    ["summary", session.summaryText],
    ["compact", session.compactText],
    ["reasoningSummary", session.reasoningSummaryText],
  ];
  const matched: FindMatchedField[] = [];
  for (const [name, text] of candidates) {
    if (!text) continue;
    const lower = text.toLowerCase();
    const termHit = terms.some((term) => lower.includes(term));
    if (termHit || (normalizedQuery.length > 0 && lower.includes(normalizedQuery))) {
      matched.push(name);
    }
  }
  return matched;
}

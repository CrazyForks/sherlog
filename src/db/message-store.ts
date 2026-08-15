import type { MessageRecord } from "../types";
import type { Db } from "./shared";

export function getMessagesForRange(
  db: Db,
  sessionId: number,
  startSeq: number,
  endSeq: number,
): MessageRecord[] {
  return db
    .prepare(`
      SELECT
        session_uuid AS sessionUuid,
        seq,
        role,
        content_text AS contentText,
        timestamp,
        source_kind AS sourceKind
      FROM messages
      WHERE session_id = ? AND seq BETWEEN ? AND ?
      ORDER BY seq
    `)
    .all(sessionId, startSeq, endSeq) as unknown as MessageRecord[];
}

export function getMessagesForPage(
  db: Db,
  sessionId: number,
  offset: number,
  limit: number,
): MessageRecord[] {
  return db
    .prepare(`
      SELECT
        session_uuid AS sessionUuid,
        seq,
        role,
        content_text AS contentText,
        timestamp,
        source_kind AS sourceKind
      FROM messages
      WHERE session_id = ?
      ORDER BY seq
      LIMIT ? OFFSET ?
    `)
    .all(sessionId, limit, offset) as unknown as MessageRecord[];
}

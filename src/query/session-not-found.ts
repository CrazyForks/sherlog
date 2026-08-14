import { DEFAULT_SESSION_SOURCE_ID, isSessionSourceId, type SessionSourceId } from "../types";

export class SessionNotFoundError extends Error {
  sessionRef: string;
  sourceId: SessionSourceId;
  nativeSessionId: string;

  constructor(sessionRef: string) {
    const identity = parseSessionRef(sessionRef);
    super(`session not found in Sherlog index: ${sessionRef}`);
    this.name = "SessionNotFoundError";
    this.sessionRef = sessionRef;
    this.sourceId = identity.sourceId;
    this.nativeSessionId = identity.nativeSessionId;
  }
}

export function parseSessionRef(sessionRef: string): { sourceId: SessionSourceId; nativeSessionId: string } {
  const separator = sessionRef.indexOf(":");
  if (separator > 0) {
    const sourceId = sessionRef.slice(0, separator);
    const nativeSessionId = sessionRef.slice(separator + 1);
    if (isSessionSourceId(sourceId)) return { sourceId, nativeSessionId };
  }
  return { sourceId: DEFAULT_SESSION_SOURCE_ID, nativeSessionId: sessionRef };
}

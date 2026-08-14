import type { MessageElision, MessageRecord } from "../types";
import { queryTerms } from "../tokenize";

export const DEFAULT_MAX_MESSAGE_CHARS = 800;

export function elideMessages(
  messages: MessageRecord[],
  options: { maxMessageChars?: number; anchorSeq?: number; query?: string } = {},
): MessageRecord[] {
  const maxMessageChars = options.maxMessageChars ?? DEFAULT_MAX_MESSAGE_CHARS;
  if (maxMessageChars <= 0) return messages;

  return messages.map((message) => {
    const query = message.seq === options.anchorSeq ? options.query : undefined;
    return elideMessage(message, maxMessageChars, query);
  });
}

function elideMessage(message: MessageRecord, maxMessageChars: number, query?: string): MessageRecord {
  if (message.contentText.length <= maxMessageChars) return message;

  const anchor = query ? findQueryAnchor(message.contentText, query) : null;
  const strategy: MessageElision["strategy"] = anchor ? "around_query" : "head_tail";
  const preserved = strategy === "around_query"
    ? preserveAroundQuery(message.contentText, maxMessageChars, anchor!.index, anchor!.length)
    : preserveHeadTail(message.contentText, maxMessageChars);
  const omittedCharCount = message.contentText.length - preserved.visibleCharCount;
  const hint = `Rerun this read with --max-message-chars ${message.contentText.length} to inspect the full message.`;
  const elision: MessageElision = {
    originalCharCount: message.contentText.length,
    displayedCharCount: preserved.text.length,
    omittedCharCount,
    strategy,
    ...(strategy === "around_query" && query ? { query } : {}),
    hint,
  };
  return {
    ...message,
    contentText: preserved.text,
    elision,
  };
}

function findQueryAnchor(text: string, query: string): { index: number; length: number } | null {
  const textLower = text.toLowerCase();
  const queryLower = query.toLowerCase();
  const phraseIndex = textLower.indexOf(queryLower);
  if (phraseIndex >= 0) return { index: phraseIndex, length: query.length };

  const terms = queryTerms(query).sort((left, right) => right.length - left.length);
  for (const term of terms) {
    const termIndex = textLower.indexOf(term.toLowerCase());
    if (termIndex >= 0) return { index: termIndex, length: term.length };
  }

  return null;
}

function preserveAroundQuery(
  text: string,
  maxMessageChars: number,
  queryIndex: number,
  queryLength: number,
): { text: string; visibleCharCount: number } {
  const budget = Math.max(maxMessageChars, queryLength);
  const queryWindow = windowAround(text.length, queryIndex, queryLength, budget);
  if (queryWindow.start === 0) return markElision(text, queryWindow.start, queryWindow.end);

  // Keep the full query window plus a short leading slice so `cd /path &&`
  // and table headers survive. Do not shrink the query window: that drops
  // nearby evidence that already fit in the original around_query span.
  const headBudget = Math.min(Math.floor(budget / 4), queryWindow.start);
  if (headBudget < 32) return markElision(text, queryWindow.start, queryWindow.end);
  if (queryWindow.start <= headBudget) return markElision(text, 0, queryWindow.end);
  return markElision(text, 0, headBudget, queryWindow.start, queryWindow.end);
}

function windowAround(
  textLength: number,
  queryIndex: number,
  queryLength: number,
  budget: number,
): { start: number; end: number } {
  const start = Math.max(0, queryIndex - Math.floor((budget - queryLength) / 2));
  const end = Math.min(textLength, start + budget);
  return { start: Math.max(0, end - budget), end };
}

function preserveHeadTail(text: string, maxMessageChars: number): { text: string; visibleCharCount: number } {
  const headCount = Math.min(Math.ceil(maxMessageChars / 2), text.length);
  const tailCount = Math.min(Math.floor(maxMessageChars / 2), Math.max(0, text.length - headCount));
  return markElision(text, 0, headCount, text.length - tailCount, text.length);
}

function markElision(
  text: string,
  firstStart: number,
  firstEnd: number,
  secondStart?: number,
  secondEnd?: number,
): { text: string; visibleCharCount: number } {
  const visible = secondStart === undefined
    ? text.slice(firstStart, firstEnd)
    : `${text.slice(firstStart, firstEnd)}\n[... shlog elided middle ...]\n${text.slice(secondStart, secondEnd)}`;
  const prefix = firstStart > 0 ? "[... shlog elided prefix ...]\n" : "";
  const suffix = secondEnd === undefined && firstEnd < text.length ? "\n[... shlog elided suffix ...]" : "";
  const visibleCharCount = secondStart === undefined
    ? firstEnd - firstStart
    : (firstEnd - firstStart) + (secondEnd! - secondStart);
  return { text: `${prefix}${visible}${suffix}`, visibleCharCount };
}

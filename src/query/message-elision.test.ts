import { describe, expect, test } from "vitest";
import { DEFAULT_MAX_MESSAGE_CHARS, elideMessages } from "./message-elision";
import type { MessageRecord } from "../types";

function message(contentText: string, seq = 0): MessageRecord {
  return {
    sessionUuid: "sess",
    seq,
    role: "user",
    timestamp: "2026-06-05T08:08:53.465Z",
    sourceKind: "event_msg",
    contentText,
  };
}

describe("elideMessages", () => {
  test("around_query keeps a comparison-table row that head_tail would drop", () => {
    const header = "两个格式的关键差异：\n";
    const storageRow = "存储位置 │ ~/.codex/sessions/（扁平） │ ~/.claude/projects/<hash>/<uuid>.jsonl（嵌套）\n";
    const padding = `${"x".repeat(500)}\n`;
    const structureRow = "对话结构 │ 线性 │ 树形（parentUuid 分支）\n";
    const tail = "y".repeat(400);
    const contentText = `${header}${storageRow}${padding}${structureRow}${tail}`;
    expect(contentText.length).toBeGreaterThan(DEFAULT_MAX_MESSAGE_CHARS);

    const headTail = elideMessages([message(contentText)], { maxMessageChars: DEFAULT_MAX_MESSAGE_CHARS, anchorSeq: 0 });
    expect(headTail[0]?.elision?.strategy).toBe("head_tail");
    expect(headTail[0]?.contentText).not.toContain("对话结构 │ 线性 │ 树形（parentUuid 分支）");

    const aroundQuery = elideMessages([message(contentText)], {
      maxMessageChars: DEFAULT_MAX_MESSAGE_CHARS,
      anchorSeq: 0,
      query: "两个格式的关键差异",
    });
    expect(aroundQuery[0]?.elision?.strategy).toBe("around_query");
    expect(aroundQuery[0]?.contentText).toContain("存储位置 │ ~/.codex/sessions/（扁平）");
    expect(aroundQuery[0]?.contentText).toContain("对话结构 │ 线性 │ 树形（parentUuid 分支）");
  });

  test("around_query also keeps a leading cd/path when the command match is later in the message", () => {
    const head = "cd /tmp/what7 && ";
    const padding = "n".repeat(900);
    const command = "node dist/cli.js publish fixtures/sample.jsonl --json";
    const contentText = `${head}${padding}${command}`;
    expect(contentText.length).toBeGreaterThan(DEFAULT_MAX_MESSAGE_CHARS);

    const aroundQuery = elideMessages([message(contentText)], {
      maxMessageChars: DEFAULT_MAX_MESSAGE_CHARS,
      anchorSeq: 0,
      query: "node dist/cli.js publish",
    });
    expect(aroundQuery[0]?.elision?.strategy).toBe("around_query");
    expect(aroundQuery[0]?.contentText).toContain("cd /tmp/what7");
    expect(aroundQuery[0]?.contentText).toContain("node dist/cli.js publish fixtures/sample.jsonl --json");
  });
});

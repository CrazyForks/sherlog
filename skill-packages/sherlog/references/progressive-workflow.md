# Progressive Workflow

场景、命令与完成标准。不变量、绝对 `$SHLOG` 解析与错误恢复以 `SKILL.md` 和 `failure-cookbook.md` 为准；本文件不重新解析 executable。

## 1. Metadata projection

用户问：`最早的有意义的对话是哪个`

```bash
DB_PATH="$("$SHLOG" stats --json | jq -r '.dbPath')"
sqlite3 -readonly "$DB_PATH" \
  "SELECT session_key, started_at, message_count, cwd, title
   FROM sessions
   WHERE message_count > 0
   ORDER BY started_at ASC
   LIMIT 10;"
"$SHLOG" read-page <session_key> --offset 0 --limit 20 --json
```

完成：candidate 来自 index metadata；“有意义”已用 content projection 验证。`sessions` 是 v8 read-only compatibility view。

## 2. Semantic recall

用户问：`上次我配 cf tunnel 是怎么弄的`

```bash
"$SHLOG" find "cf tunnel" --json -n 5
```

短产品名/通用文件名先加 `--cwd` 或换成更独特 phrase。

完成：回答中的每个事实都能指到某条 `read-*` 返回的 message/session 内容；未验证的部分已说明不确定。

## 3. Current project discussion

用户问：`最近这个项目讨论了什么`

```bash
"$SHLOG" list --cwd <repo-cwd-fragment> --sort ended -n 8 --json
"$SHLOG" read-page <sessionRef-or-codex-uuid> --offset 0 --limit 20 --json
```

注意 `list --cwd` 是 metadata substring filter，不是 exact cwd selector。需要 coverage proof 时另用：

```bash
"$SHLOG" status --cwd <absolute-repo-cwd> --json
```

完成：逐个读取回答实际引用的 session；若声称覆盖这 8 个候选，就全部检查，若只抽样就说明选取范围。任何 `read-page.hasMore=true` 且该 session 的目标上下文仍未解决时继续翻页。

## 4. Latest keyword, excluding self-hit

```bash
"$SHLOG" find "X" --cwd <repo-cwd> --sort ended \
  --exclude-session <current-sessionRef> -n 5 --json
```

按 `endedAt` 顺序执行 candidate 的 `evidenceRead.command`，直到首个内容确认 phrase X 的结果；更靠前但尚未取证的 candidate 仍存在时不能声称“最新”。完成：phrase X 已从 `read-*` 输出确认，且所有更晚 candidate 已取证并排除。

## 5. Coverage diagnosis

用户问：`为什么这个 repo 的历史查不到`

```bash
"$SHLOG" status --cwd <repo-cwd> --json
```

- `fresh + complete`：refine query；
- `missing/stale + sync`：同 source/root/cwd sync 后 retry；
- `source_content_changed + query`：现有 index 可先查，只有 latest tail 重要时再 sync。

完成：必要操作已重试；只在 coverage 可证明时下完整 miss 结论。

## 6. Known session decision

```bash
"$SHLOG" read-range <sessionRef> --query "决定" --before 6 --after 10 --json
"$SHLOG" read-page <sessionRef> --offset 0 --limit 60 --json
```

有 elision 且关键句不可见：

```bash
"$SHLOG" read-range <sessionRef> --query "决定" --before 6 --after 10 \
  --max-message-chars 0 --json
```

完成：目标结论已被 `read-range`/`read-page` 输出中的具体语句覆盖；若 `read-page` 报 `hasMore=true` 且还没看到目标上下文，继续翻页，不得提前结束。

## 7a. Cold inspection

```bash
"$SHLOG" cold list --source codex --json
```

完成：知道当前 registered roots；不执行任何删除。

## 7b. Authorized prune

先与用户确认删除意图（删除 hot 与 registered cold 都不存在的 projection），确认后才运行：

```bash
"$SHLOG" sync --source codex --prune --json
```

完成：检查 `removed` 与 `retainedCold`，并说明删除了什么、保留了什么。

## 8. Raw full-text fallback

只在 `read-*` projection 明确不足以回答完整 tool call、patch、长代码或原始 event 时进入：

1. 用 Sherlog 定位 `sessionRef`、source、time、cwd；
2. cold path 来自 `cold list --json` 或用户明确路径；
3. 只定位对应 raw session；
4. hot plain JSONL 可 `rg`，cold per-file zstd 先解压临时副本；
5. 回答区分 index projection 与 raw transcript evidence。

```bash
"$SHLOG" cold list --json
rg "exact clue" <hot-root> --glob '*.jsonl'
zstd -d <cold-session-file>.jsonl.zst -o /tmp/sherlog-session.jsonl
rg "exact clue" /tmp/sherlog-session.jsonl
```

raw fallback 是 agent-side 取证，不是 `shlog` subcommand；不能用它绕过 privacy projection 做常规召回。

## 9. Source-aware read

```bash
"$SHLOG" find "failure phrase" --source claude-code --json
"$SHLOG" read-range claude-code:<native-id> --query "failure phrase" --json
```

不要从 bare UUID 猜 source。完成：`sessionRef` 来自 find output，read source 与它一致。

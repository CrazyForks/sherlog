# Advanced Queries

## Query/tokenizer 语义

Sherlog 不把用户输入原样透传给 FTS5：

- 非 CJK text 依照 UAX #29 word segmentation 切分并 lowercase；
- 连续 Han/Hiragana/Katakana/Hangul 生成重叠 Unicode-scalar bigram；
- query terms 去重但保持 first-seen order；
- 每个 term 双引号转义，terms 用 `AND` 连接；
- 单个 CJK scalar 等非空 query 若没有 FTS token，可走有界 literal substring LIKE；
- zero candidate 后可能尝试确定性的 relaxed query，但没有全正文无界 fuzzy。

因此用户输入的 `OR`、`NEAR`、`*` 或 quotes 不应被当成 native FTS operator。先用自然关键词；过窄时删掉冗余 term，过宽时增加稳定 identifier/error phrase。

## Candidate 与 evidence

v8 `documents_fts` 同时索引：

- message body；
- session title；
- summary；
- compact；
- reasoning summary。

BM25 column weights：`body=1.0`、`title=8.0`、`summary=3.0`、`compact=4.0`、`reasoning=1.2`。

session-profile candidate 只是 recall signal，不是 message evidence。`matchSource="session"`、`matchSeq=null` 时仍执行完整 `evidenceRead.command`；`read-range --query` 无 message anchor 会返回 typed `anchor_not_found`，按 nextAction 回退 `read-page`。

source/root/cwd/date/exact-session/exclude 条件尽量在 SQL candidate generation 下推。不要先拉全库候选再自行过滤，也不要把 session score 当成 message anchor。

## CJK 实务

- 至少两个连续 CJK scalar 才产生 bigram；
- supplementary Han（如 `𠮷`）按 Unicode scalar，不按 UTF-16 code unit；
- 中英混合 query 可以用中文短词 + English identifier；
- 单字 CJK fallback 是 literal LIKE，不代表可扩展的 fuzzy search；
- 零结果按 `zeroResults` / coverage policy 处理，不要盲目重复。

## `list` vs `find`

已知 project/time、关键词弱时先 `list`：

```bash
"${SHLOG_BIN:-${CXS_BIN:-shlog}}" list --cwd <cwd-fragment> --since <iso> --json
```

需要内容主题时 `find`：

```bash
"${SHLOG_BIN:-${CXS_BIN:-shlog}}" find "specific phrase" --cwd <absolute-cwd> --json
```

`list --cwd` 是 case-insensitive substring；`find --cwd` 构造 exact cwd selector。两者 coverage 语义不同。

## Read-only SQLite metadata projection

v8 公共 metadata surface 是 `sessions` read-only compatibility view；物理 writer table `session_rows` 不应被 agent 直接修改。

先拿 DB path：

```bash
DB_PATH="$("${SHLOG_BIN:-${CXS_BIN:-shlog}}" stats --json | jq -r '.dbPath')"
```

最早 session：

```bash
sqlite3 -readonly "$DB_PATH" \
  "SELECT session_key, started_at, message_count, cwd, title
   FROM sessions
   WHERE message_count > 0
   ORDER BY started_at ASC
   LIMIT 20;"
```

某 cwd 最近 session（应用层参数化不可信输入；这里只展示 SQL shape）：

```sql
SELECT session_key, ended_at, message_count, title
FROM sessions
WHERE cwd = ?
ORDER BY ended_at DESC
LIMIT 20;
```

按 cwd 聚合：

```bash
sqlite3 -readonly "$DB_PATH" \
  "SELECT cwd, COUNT(*) AS sessions, SUM(message_count) AS messages,
          MAX(ended_at) AS latest
   FROM sessions
   GROUP BY cwd
   ORDER BY sessions DESC
   LIMIT 20;"
```

稳定 metadata columns：

- `id`
- `source_id`
- `native_session_id`
- `session_key`
- `session_uuid`
- `file_path`
- `source_root`
- `title`
- `summary_text`
- `compact_text`
- `reasoning_summary_text`
- `cwd`
- `model`
- `started_at`
- `ended_at`
- `path_date`
- `message_count`
- `document_count`

metadata projection 不是内容证据。拿到 candidate 后用 `read-range` / `read-page`。

## FTS internal rules

- `documents_fts` 是 contentless；不要 select 它的 text columns。
- snippet 从 `documents` JOIN 后的原文构造，带 `<mark>...</mark>`。
- session profile 与 message 都在 `documents`，用 `kind` 区分。
- 不要手写 FTS row、`meta` epoch、coverage 或 source cursor。

## Same-title variants

Codex resume/fork 可能出现 title 相似但 identity 不同的 session。当前不会按 title family collapse：

- 不要先按 title 去重；
- 看 `sourceId/sessionRef`、cwd、time、matchCount；
- 用 evidence read 决定是否同一决策链。

## Future optimization constraints

如果未来增加 fast prefilter：

- 必须返回 conservative candidate superset；只能排除可证明不匹配的 item；
- tokenizer/delta/source state 不确定时纳入 candidate；
- 必须由 exact/evidence stage 验证；
- incremental candidate state 必须与 full replay 等价。

当前没有 public `weakMatch`/`matchMode` 或 per-stage candidate count，也没有默认 typo fallback/frecency。不要在 agent workflow 中假设这些字段存在。

## Source of truth

- `rust/src/tokenizer.rs`
- `rust/src/index/reader.rs`
- `rust/src/retrieval/query.rs`
- `rust/src/retrieval/ranking.rs`
- `rust/src/index/v8.sql`

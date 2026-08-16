---
name: sherlog
description: "Use proactively for local agent-session history and prior setup archaeology. Trigger when the user asks what was discussed/done/configured before, asks which local servers/services/accounts/domains/providers were configured, or says 之前/上次/配过/装过/历史对话/翻旧 session. Not for current-repo code search, live-only state, web docs, daily summaries, or session wrap-up."
---

# Sherlog

用 standalone `shlog` 在本机 Sherlog SQLite index 中检索 agent 历史。Node.js 不是 CLI runtime dependency。

默认命令形式：

```bash
"${SHLOG_BIN:-${CXS_BIN:-shlog}}" <subcommand> ...
```

## 先选 retrieval primitive

| 用户需要 | 起手 primitive | 完成标准 |
| --- | --- | --- |
| metadata projection：最早/最新、数量、cwd/session 清单、大 session | `list` 或只读 SQLite `sessions` view | 候选完整；涉及内容时已用 `read-*` 验证 |
| semantic recall：主题、关键词、历史配置考古 | `find <query> --json`，按需加 scope | 已执行候选的 `evidenceRead.command`（`executable` + `args` 原样拼接），不只看 title/snippet |
| context read：已知 `sessionRef` / seq | `read-range` 或 `read-page` | 已读足够 projection 证据 |
| coverage/freshness/index availability | `status --cwd/--selector --json` | 已按 `requestedCoverage.recommendedAction` 决定 query 或同范围 sync |
| mutation：建/更新 index | bare/scoped `sync` | 无未解决 error；coverage 语义已检查 |
| cold retention | `cold add/list/remove` | v8 registration 与后续 prune 意图明确 |

## Canonical policy

| Policy | Rule |
| --- | --- |
| Read/write | `find/read/list/stats/cold list` 只读 SQLite，不扫描 raw、不隐式 sync/migrate。`status` 不返回/检索正文、不写 index；inventory cache miss 可流式读 raw，但仅以 privacy-allowlisted projection 派生 proof，exact cache hit 不重 parse。`sync` 写 content/index/coverage；`cold add/remove` 只写 retention state。 |
| Evidence | `find/list` 只定位 candidate；内容执行 `evidenceRead.command`（`executable:"inherit"` + `args` 原样执行，`sideEffect:"read_index"`）/ `read-*`。只有 projection 明确缺少完整 tool call、patch、长代码或原始事件时，定位 session 后才走 agent-side raw fallback。 |
| Provenance | 先看 `matchSource`、`matchedFields`、`sessionMessageCount`。message anchor 走 `read-range`；session-level hit 原样执行 `evidenceRead`，`read-range --query` 无 message anchor 时返回 typed `anchor_not_found`（含 `matchedProfileFields` 与 read-page nextAction），此时回退 `read-page` 或改用消息中真实出现的 term，严禁伪造 seq。关键句因 elision 不可见时加 `--max-message-chars 0`。 |
| Sort | `find` 默认 relevance；最新/最近用 `--sort ended`，必要时 `--exclude-session` 防 self-hit。日期自然语言只是 query term，不是 date filter。 |
| Query refine | 多 term 是 quoted-term AND。零结果先读 `zeroResults.reason`；不要原样重复过长 query。当前没有全正文 typo/fuzzy search。 |
| Coverage | `find/list` 只报告 stored index proof，`freshness` 为 `not_checked`；需要 live proof 才用同 scope 的 `status --cwd/--selector`。`recommendedAction=query` 不 sync，`recommendedAction=sync` 才同步同范围。 |
| Legacy index | `status` 的 `index.layout` 为 `legacy_v7` 时：内容命令会返回 typed `index_schema_upgrade_required`（nextAction 为 `shlog sync --db <db> --json`）。跑一次该 sync 即完成迁移（保留 `*.v7.bak.*` 备份、coverage 重建、旧 0.4.4 writer 不再可写），之后所有命令回到 v8。不要绕开这条路径去手工 patch 旧库。 |
| Cold | v8 truth 是 SQLite `cold_roots`。`cold add` 只登记 presence；`sync` 不从 `.jsonl.zst` 重建。JSON `configPath` 是 legacy tombstone 兼容字段，不是 v8 truth。 |
| Prune | 默认不用 `--prune`。只有用户明确要删除 hot 与 registered cold 都不存在的 projection 时才执行。registered cold root 不可达（missing/unmounted/permission）时 prune fail-closed，不会把不可读当作不存在。当前 cold-presence destructive prune 只支持 Codex。 |

## Source 与 identity

- public source：`codex`、experimental `claude-code`、experimental `pi`；
- `find` 默认跨 public source；其他 source-scoped command 省略时默认 Codex；
- 跨 source 读取使用 `find` 返回的 `sessionRef`，不要从 UUID 猜 source；
- `matchSource = "session"` 时 `matchSeq = null`，但 `evidenceRead` 可能给出 `read-range --query`；执行后若返回 `anchor_not_found`，按 error 的 `nextAction` 回退 `read-page`；
- 无 `--root/--cwd/--selector` 的 `find` 只搜各 source 的 canonical default `all(root)`；历史只同步在非默认 root 时，必须显式给同 root scope。

## Coverage workflow

不要固定执行 `status -> sync -> find`，也不要每次检索前无条件 sync。先 query；只有结果/零结果的 coverage 不足以支持 latest/completeness 结论时：

```bash
"${SHLOG_BIN:-${CXS_BIN:-shlog}}" status --cwd <repo-cwd> --json
"${SHLOG_BIN:-${CXS_BIN:-shlog}}" sync --cwd <repo-cwd> --json
```

保持 status、必要的 sync、retry 的 source/root/selector 一致。status 为 `fresh` 或 `source_content_changed` + `recommendedAction: "query"`（proven append）时直接 query/refine；只有 `recommendedAction: "sync"`（含 truncate/prefix rewrite/same-size rewrite 等未证明 append 的破坏性变化），或答案依赖最新 active tail 时才 sync。

## 不适用

当前 repo 代码搜索、已知路径阅读、外部网页/docs、无历史语义的 live state、日报、当前会话收尾。

## 每次使用后的轻量自评

内部归类为：`reliable` / `needs-refine` / `coverage-issue` / `skill-guidance-issue` / `dogfood-candidate`。

- 前四类按需 refine、status/sync 或说明边界；
- 可复现 recall/ranking/context 问题只建议用户显式说 `$sherlog-dogfood 记录这个 case`；不要自动写或 promote 私有 golden；
- 有真实 scan/count/time 才报告性能数字。

## 参考

按需加载：

- `references/progressive-workflow.md` — 场景、完成标准、raw fallback
- `references/failure-cookbook.md` — coverage/error/recovery
- `references/cli-surface.md` — command/options/defaults
- `references/advanced-queries.md` — tokenizer/FTS/metadata SQL
- `references/json-schema.md` — public JSON 与 error shape

# skill-sync: native v8 canonical retrieval policy

# Index coverage contract

本文记录 native v8 的当前 coverage 不变量。架构总览见 [ARCHITECTURE.md](ARCHITECTURE.md)，命令示例见 [USAGE.md](USAGE.md)。

## 范围与真相源

当前 public source：`codex`、experimental `claude-code`、experimental `pi`。Coverage 是 SQLite v8 对 canonical selector 的 stored sync proof，不是内容副本，也不是 live filesystem 状态。

- 内容真相来自 privacy-filtered `documents` projection。
- 在 `shlog` CLI 内，raw transcript 只由 `sync` 做 bounded projection，或由 `status` 在 inventory cache miss 时流式解析 accepted projection；query/read 不读取 raw。
- `sync` 是唯一 content/index/coverage writer。
- `cold add/remove` 只写 retention registration；`cold list` 只读。
- `find`、`read-range`、`read-page`、`list`、`stats` 只读 SQLite，不隐式 sync/migrate。

## Selector

Canonical selector 总是带 source 与 root：

```text
all(source, root)
date_range(source, root, fromDate, toDate)
cwd(source, root, cwd)
cwd_date_range(source, root, cwd, fromDate, toDate)
```

CLI 可在入口把 `--root`、`--cwd` 或缺省字段规范化为 selector。进入 coverage/index 后，不再存在无 source 的 selector。

Coverage implication 仅在相同 source/root 内成立：

- `all` 覆盖任意更窄 selector；
- `date_range` 覆盖其日期区间内的 date/cwd-date selector；
- `cwd` 覆盖相同 cwd 及其更窄日期 selector；
- `cwd_date_range` 只覆盖相同 cwd 且日期被包含的 selector。

不同 source/root 永不互相覆盖。

## Stored proof 与 live freshness 分离

成功 sync 写入的 `coverage` record 包含 selector、source/file-set fingerprints、file/session counts、完成时间与 index version。

Query-only command 只能判断是否存在兼容的 covering record：

- `complete=true` 表示 SQLite 中有 covering sync proof；
- `freshness="not_checked"` 表示本次 command 没有扫描 raw；
- stored `complete` 不能单独证明 raw 当前仍未变化。

只有 `status --cwd/--selector` 会建立 live source snapshot，并把 stored record 与当前 snapshot 比较。Cache miss 会流式读取 raw records/body，但只让 privacy allowlist 接受的 metadata/message projection 影响 inventory/fingerprint；rejected/private record 不影响 proof。exact `mtime_ns`/checkpoint cache hit 不重 parse。它返回：

- `fresh` / `recommendedAction: "query"`；
- `missing` / `recommendedAction: "sync"`；
- `stale`，通常建议 sync；Codex 的 `source_content_changed` 可作为 advisory soft stale 返回 `recommendedAction: "query"`。

`status --root` 只改变 inventory/default root；要得到 requested selector proof，必须传 `--cwd` 或 `--selector`。`--inventory` 才展开完整 coverage inventory 与 cwd groups。Status 不返回或检索正文、不写 index；cache miss 成本可能是 O(raw bytes)，不是固定 metadata-only 操作。

## Command 权限

| command | raw access | coverage behavior | write |
|---|---|---|---|
| `status` | cache-aware allowlisted inventory；miss 可流式读 raw | live requested freshness | 否 |
| `sync` | bounded transcript projection | 完整时写 coverage record；否则明确 not-written | content/index/coverage |
| `cold add/remove` | root validation | 不写 coverage | retention only |
| `cold list` | 无 | 无 | 否 |
| `find` | 无 | stored proof，freshness `not_checked` | 否 |
| `list` | 无 | selector 存在时读 stored proof；否则 incomplete/not_checked | 否 |
| `read-*` | 无 | 返回包含该 session 的 stored coverage entries | 否 |
| `stats` | 无 | 返回 stored coverage rows | 否 |

## Sync proof

- Bare `sync` 是默认 Codex root 的 canonical `all` bootstrap，不是 query 的隐式前置步骤。
- Strict sync 只有在选中 snapshot 可证明完整时才写 complete record；失败不发布部分 complete coverage。
- `--best-effort` 可提交成功 projection，但 errors 存在时不能伪造 complete coverage。
- Codex 可对已证明安全的 bounded append 提交 prefix，并用 `source_content_changed` / `recommendedAction: "query"` 标记尾部 advisory；truncate、prefix rewrite 与不安全 identity change 走 full replay 或失败。
- 新 active file 若无法证明安全 prefix，可返回 `active_source_deferred`，不写 complete coverage。
- 默认 sync 保留 raw 已消失的历史；只有显式 `--prune` 才删除 hot 与 registered/ephemeral cold 都不存在的同-source projection。
- cold-only history 可使 `indexedSessionCount > sourceFileCount`，这是 retention 语义，不是 coverage corruption。

## Agent 决策流程

不要固定执行 `status -> sync -> find`，也不要每次 query 前无条件 sync：

```text
find/list from SQLite
  -> useful candidate: evidenceRead/read-*; only check freshness if the answer needs latest/completeness
  -> zero result or completeness required: status for the same selector
       -> recommendedAction=query: refine/retry without sync
       -> recommendedAction=sync: sync the same selector, then retry
```

因此：

- 非空结果可以先做 evidence read；coverage 未确认只限制完整性结论，不使 evidence 失效。
- 当前 native `find` 不做 live scan，正常返回的 coverage freshness 是 `not_checked`；把紧邻的同-selector `status` proof 与 find 结果组合使用。
- 只有 status 显示 missing/stale 且建议 sync，或答案明确依赖尚未索引的最新 tail 时，才做同范围 sync。
- 对全历史未找到的结论，需要同 source/root 的 live fresh `all` proof；任意 narrow coverage 不能冒充全局完整。

## 非目标

- watcher / daemon / realtime sync；
- query command 扫 raw 或隐式 sync；
- Codex state DB 作为内容候选源；
- raw grep 取代 privacy-filtered projection；
- 跨 source/root 的 coverage implication。

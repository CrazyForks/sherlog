# Failure Cookbook

唯一的错误恢复入口。先判断失败层并读取 error payload/hint，再按表中条件执行；表格与下方细节共同定义动作，不要把所有问题都用无条件 full sync 处理。typed-error `nextAction.commands[].argv[0] == "shlog"` 是 portable placeholder：用本轮记录的绝对 `$SHLOG` 替换它，其余 argv 逐项不变。

| 错误 | 立即动作 |
| --- | --- |
| `shlog` 不存在 / 版本过旧 | 安装或升级 CLI；显式 override 同步更新或取消后重新解析绝对 executable，再重试原命令。仅在后续 index/coverage payload 要求时 sync |
| `index_unavailable` | 执行 nextAction 里的 bootstrap sync；只有问题明确局限当前 repo 时才 `--cwd` |
| `index_schema_upgrade_required` | supported `legacy_v7` 才执行闭包 `--db` 的 sync；future/incompatible v8 改用兼容 binary 或可信 backup，不重复 sync |
| `session_not_found` | 从原 find/user scope 恢复 `--cwd/--selector` 后跑 status；有 `requestedCoverage.recommendedAction=sync` 才同范围 sync。无法恢复 scope 就说明无法证明，不盲目 sync |
| `anchor_not_found` | 按 nextAction 回退 `read-page`，或改用消息中真实出现的 term |
| zero results | 同 selector `status`；`query` 则 refine，`sync` 则同范围 sync 后重试 |
| strict sync failure | 看 `errorDetails[]` 修 source 后同范围重试；不用 `--best-effort` 冒充 complete |
| `invalid_cold_root` / cold prune failure | 修正 path 或恢复可读性；destructive prune fail-closed，不绕过 |

## `shlog` 不存在或版本过旧

```bash
curl -fsSL https://github.com/catoncat/sherlog/releases/latest/download/install.sh | sh
# 或：brew tap catoncat/sherlog && brew install sherlog
# 已安装 tap 时升级：brew update && brew upgrade sherlog
```

CLI 与 skill 分开安装；CLI 不依赖 Node.js。若 `SHLOG_BIN` / `CXS_BIN` 显式指向缺失或旧 binary，安装 PATH 版本不会覆盖该优先级：更新或取消 override，按 `SKILL.md` 重新解析绝对 executable，再重试原命令。只有随后出现 supported v7 migration 或 coverage `recommendedAction=sync` 时才运行对应 scope 的 sync。

## `index_unavailable`

含义：query/read/list/stats/cold-list 请求的 SQLite index 不存在。只读 command 不会创建它。

默认 bootstrap：

```bash
"$SHLOG" sync --json
```

只有用户问题明确局限当前 repo 时才用：

```bash
"$SHLOG" sync --cwd <repo-cwd> --json
```

有 `nextAction.commands[].argv` 时优先执行：argv[0] 的 portable `shlog` 替换为绝对 `$SHLOG`，其余 argv 逐项不变。

## `unsupported_source`

当前 public source 只有 `codex`、`claude-code`、`pi`；只有 `find --source all` 接受 `all`。检查拼写与 selector.source，不要自动换 source。

## `invalid_selector` / `selector_required`

- `--selector` 与 `--cwd` 不能同时使用；
- selector source 必须与 `--source` 一致；
- root/cwd 必须可规范化为有效 UTF-8 path；
- date selector 使用 `fromDate` / `toDate`。

修正参数后重试；不要降级成不带 scope 的全局 destructive sync/prune。

## `index_schema_upgrade_required`

含义：reader/writer 发现不支持的 schema/version/epoch。该 code 同时覆盖 supported v7 与 future/incompatible v8；read-only command 不会改 DB，writer 也不会静默混写。

处理：

1. 确认 `dbPath` 与执行的 binary 版本；
2. 如果是受支持 v7，运行同 source/root 的显式 `sync` 触发 copy/verify migration；
3. 如果是 future/unknown v8 epoch，不要手改 `meta`，保留 DB 并使用匹配版本或从受信 backup 恢复；
4. migration error 时保留 `.backup` / quarantined evidence，不删除后重来。

## `session_not_found`

可能原因：

- bare UUID 被按 Codex 解释，但 session 来自其他 source；
- raw session 存在但尚未进入 index；
- source/root selector 未覆盖；
- session 已被显式 prune。

先确认 `sessionRef`，再从原始 find invocation、candidate cwd 或用户问题恢复最窄相关 `--cwd/--selector`，运行同 source/root 的 status。error 自带的 source-only status argv 不含 selector，因此本身不会产生 `requestedCoverage.recommendedAction`；补齐 scope 后只有 `recommendedAction=sync` 才同步同范围，最后重试原 read argv。无法恢复 scope 时说明无法证明 coverage，不退化成无 scope sync 或全量 raw inventory。

## Zero results

当前 native `find` 是 query-only：它不扫 raw，所以 stored coverage 的 `freshness` 正常为 `not_checked`，zero-result diagnosis 通常是 `coverage_not_confirmed`。即使 `coverage.complete=true`，也只说明 SQLite 有 compatible covering record，不说明 raw 此刻未变化。

选择问题的实际 scope，运行同 source/root/selector 的 `status --cwd/--selector --json`：

- `recommendedAction: "query"`：不要 sync；用当前 index refine/retry。若 live status 是 fresh，可把这份 proof 与紧邻的 find miss 组合起来做范围内结论。
- `recommendedAction: "sync"`：同范围 sync，成功后重试；不要扩大为无关全量 destructive sync。
- Codex `source_content_changed` + `query`：proven append 的 soft stale，只有答案依赖最新 active tail 时才 sync。truncate/prefix rewrite/same-size rewrite 无法证明 append 时，status 会给出 `recommendedAction: "sync"`，不要把它当 soft stale 继续 query。

Refine 时：

- 去掉冗余自然语言；
- 使用稳定 identifier/error phrase；
- 单字 CJK 改为至少两字词；
- 不使用用户自造 FTS `OR`/`NEAR`/`*`。

Schema 仍接受 `fresh_miss` / `stale_or_missing_coverage` 作为兼容 reason；不要从 stored coverage 自行伪造它们，实际动作仍以同 selector status 的 live proof 为准。

## Non-empty results + unconfirmed coverage

已有 candidate 可以继续 `evidenceRead`，但不要声称完整。答案是否依赖最新 tail/全量结果，决定是否需要 status/sync。

## `source_content_changed`

表示 bounded prefix 已安全提交，但 active file 在同步窗口继续 append。`recommendedAction: query` 时先查现有 index；只有答案依赖最新尾部或 strict completeness 时再 sync。

这不等于 truncate/prefix rewrite。后两者不能被 soft-stale 掩盖。

## `active_source_deferred`

新/未索引 active file 在建立 bounded read 前已变化，Sherlog 无法证明旧 prefix 安全。其他稳定 file 可能已提交，但 complete coverage 未写。稍后执行同范围 sync。

## Legacy v7 index (升级后首次使用)

`status` 的 `index.layout` 为 `legacy_v7` 时，内容命令（find/read/list）返回 typed `index_schema_upgrade_required`，`nextAction.commands[0]` 是闭包 `--db` 的 `shlog sync`。执行一次即完成迁移：

- 迁移保留 `*.v7.bak.*` 一致备份，可用备份回滚；
- 迁移后 coverage 清空，需要重新 sync 各常用 root/selector；
- 迁移是单向的：旧 0.4.4 writer 对 v8 fail-closed；
- 不要为绕过迁移手工 patch 旧库，或反复重试读命令。

## Strict sync failure

JSON report 的 `errors` 与 `errorDetails[]` 是 per-file/source evidence。strict failure 不发布部分 complete coverage；先修 filesystem/permission/malformed source，再同范围重试。

不要：

- 改成 `--best-effort` 后把结果称为 complete；
- 手工删 DB、lock、backup；
- 用 raw grep 代替 privacy projection。

## `--best-effort`

用于允许成功 file 先进入 projection。带 errors 的 report 仍需记录，coverage 不应是 complete。回答时说明可能漏掉哪些 scope；必要时修复后 strict sync。

## Writer lock

sync、migration、cold add/remove 共用 single-writer lock。活跃 writer 时等待/失败；dead owner 可由实现安全清理。不要手动并发多个 writer，也不要删除无法确认 owner 的 lock directory。

## `invalid_cold_root`

`cold add` 的 root 必须存在且是 directory。修正 path 后重试。`cold remove` 不要求 archive 仍存在。

v8 registration truth 是 SQLite `cold_roots`；不要根据 JSON `configPath` 直接编辑 `cold-roots.json`。

## Cold prune failure

- cold root unreadable/walk error：destructive prune fail closed；先恢复可读性。
- non-Codex source + cold roots：identity mapping 未实现，拒绝 destructive prune；不要绕过。
- `.jsonl.zst` 存在但 filename 无 UUID：不会保护 indexed session；先修 archive naming/registration，验证后再 prune。

## Legacy cutover/tombstone error

成功 v8 cutover 后 `cold-roots.json` 是目录 tombstone。旧 writer 写这个路径失败是预期 fence，不要把 tombstone 换回文件。需要查看导入前配置时读 `cold-roots.json.v7-imported.*` backup，只读，不作为 active truth。

## Raw evidence required

如果 `read-*` 已定位 session，但 projection 确实没有完整 tool call/patch/long code：

1. `cold list --json` 找 registered roots；
2. 只定位对应 session raw；
3. plain JSONL 用 `rg`，zstd 单文件先解压到临时路径；
4. 回答时区分 index projection 与 raw transcript evidence。

这是 agent-side fallback，不是 Sherlog query error recovery 的默认路径。

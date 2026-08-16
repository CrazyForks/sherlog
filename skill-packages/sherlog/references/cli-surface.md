# CLI Surface

Production CLI 是 standalone Rust binary。下面只描述固定 public command；不要使用旧 `window/session` alias、sidecar 命令或 npm CLI。

统一写法：

```bash
SHLOG="${SHLOG_BIN:-${CXS_BIN:-shlog}}"
```

所有 command 支持 `--db <path>`；结构化输出使用 `--json`。

## `status`

```bash
"$SHLOG" status \
  [--source codex|claude-code|pi] \
  [--root <dir>] [--cwd <path> | --selector <json>] \
  [--inventory] [--db <path>] [--json]
```

- 默认 source：Codex。
- 不返回/检索正文、不写/migrate index。inventory cache miss 可流式读取 raw records/body，但仅让 privacy-allowlisted accepted projection 影响 inventory/fingerprint；rejected/private record 不影响 proof。exact `mtime_ns`/checkpoint cache hit 不重 parse。
- `--cwd` / `--selector` 才产生 `requestedCoverage`；`--root` 只改变 inventory/default selector root。
- 默认省略完整 `coverage[]` 与 cwd groups；审计时加 `--inventory`。

## `sync`

```bash
"$SHLOG" sync \
  [--source codex|claude-code|pi] \
  [--root <dir>] [--cwd <path> | --selector <json>] \
  [--best-effort] [--prune] [--cold-root <dir>]... \
  [--db <path>] [--json]
```

- 默认 source：Codex；bare sync 等价于默认 Codex root 的 `all` selector。
- 唯一 content/index/coverage writer；可创建 v8 或显式 migrate v7。
- strict 默认：任何不可接受输入错误都使 command 非零，不发布部分 complete coverage。
- `--best-effort` 可提交成功 projection，但保留 errorDetails 且不伪造 complete coverage。
- `--prune` 才删除 hot/cold 都不存在的当前-source projection。
- `--cold-root` 只参与本次 prune，不持久注册。

## `cold`

```bash
"$SHLOG" cold add --root <dir> [--source codex|claude-code|pi] [--db <path>] [--json]
"$SHLOG" cold list [--source codex|claude-code|pi] [--db <path>] [--json]
"$SHLOG" cold remove --root <dir> [--source codex|claude-code|pi] [--db <path>] [--json]
```

- add/remove 默认 Codex，是 retention-state writer；list 只读。
- v8 truth：SQLite `cold_roots`。
- add 需要现有 directory；remove 不删除 directory 或 index row。
- registration 只保护显式 prune，不摄取 zstd body。
- 当前 destructive cold presence mapping 只支持 Codex。
- JSON 的 `configPath` 是 legacy tombstone compatibility，不是 v8 truth。

## `find`

```bash
"$SHLOG" find <query> \
  [--source all|codex|claude-code|pi] \
  [-n|--limit <n>] [--sort relevance|ended|started] \
  [--root <dir>] [--cwd <path> | --selector <json>] \
  [--exclude-session <uuid-or-sessionRef>]... \
  [--db <path>] [--json]
```

- 默认跨所有 public source；默认 sort `relevance`，limit 10。
- 用户问“最新/最近 + 关键词”用 `--sort ended`。
- self-hit 用 `--exclude-session`，可重复。
- query 只读 SQLite；不扫描 raw、不检查 live freshness、不隐式 sync。coverage freshness 因此为 `not_checked`；需要 live proof 时另跑同 scope `status`。
- JSON candidate 包含 `sessionRef`、`matchSource`、`matchSeq`、`matchedFields`、`sessionMessageCount` 与 `evidenceRead.command`（`executable:"inherit"`、`args`、`sideEffect`）。

## `read-range`

```bash
"$SHLOG" read-range <sessionRef> \
  [--source <source>] [--seq <n>] [--query <text>] \
  [--before <n>] [--after <n>] [--max-message-chars <n>] \
  [--db <path>] [--json]
```

- bare id 默认 Codex；source-qualified `sessionRef` 优先。
- 至少需要有效 `--seq` 或可在该 session 找到 anchor 的 `--query`。
- before/after 默认各 2。
- `--max-message-chars` 默认 800；0 禁用 elision。
- exact session scope 在 SQL candidate generation 下推，避免全局 limit 吞掉目标 anchor。

## `read-page`

```bash
"$SHLOG" read-page <sessionRef> \
  [--source <source>] [--offset <n>] [--limit <n>] \
  [--max-message-chars <n>] [--db <path>] [--json]
```

- offset 默认 0，limit 默认 20。
- `--max-message-chars` 默认 800；0 禁用 elision。

## `list`

```bash
"$SHLOG" list \
  [--source codex|claude-code|pi] [--cwd <needle>] [--since <iso>] \
  [--root <dir>] [--selector <json>] \
  [--sort ended|started|messages] [-n|--limit <n>] \
  [--db <path>] [--json]
```

- 默认 source Codex、sort ended、limit 20。
- `--cwd` 是 case-insensitive metadata substring filter，不构造 cwd selector；需要 exact selector coverage 时传 `--selector`。
- 无 selector 的 list coverage 是 `not_checked`/incomplete；不会把任意窄 coverage 当全局 complete。

## `stats`

```bash
"$SHLOG" stats [--source codex|claude-code|pi] [--db <path>] [--json]
```

默认 Codex。只读 source-scoped counts、time range、top cwd、DB size、index version 与 stored coverage。

## Selector JSON

```json
{"source":"codex","kind":"all","root":"/abs/sessions"}
{"source":"codex","kind":"cwd","root":"/abs/sessions","cwd":"/abs/repo"}
{"source":"codex","kind":"date_range","root":"/abs/sessions","fromDate":"2026-08-01","toDate":"2026-08-15"}
{"source":"codex","kind":"cwd_date_range","root":"/abs/sessions","cwd":"/abs/repo","fromDate":"2026-08-01","toDate":"2026-08-15"}
```

CLI 可补默认 root/source。`--selector` 与 `--cwd` 互斥；显式 `--source` 必须与 selector source 一致。

## Exit/output

- success：0。
- business/index/sync/parse failure：non-zero。
- typed business errors + `--json`：通常输出 `{"error":{...}}`；按 contract 可能在 stdout。
- strict sync JSON failure report：stderr。
- best-effort sync report：stdout，即使带 per-file errors也不代表 complete coverage。
- CLI parser error（如漏 `find` query）：plain stderr，不包装 JSON。

## Source of truth

- `rust/src/cli.rs`
- `rust/src/app/selectors.rs`
- `rust/src/runner.rs`
- `rust/src/app/output.rs`

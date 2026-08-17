# Sherlog performance benchmark

`npm run eval:perf` 是 candidate-aware 的 Node 开发 harness。它从父进程观测公开 CLI contract，可以测 Rust release binary、已安装 `shlog` 或 TypeScript reference。Harness 本身不是 production CLI。

## 先选对被测 executable

未传任何 override 时，harness 默认运行：

```text
node --import tsx <repo>/src/cli.ts
```

这是 differential oracle，不是 production candidate，也不能作为 native acceptance 证据。验证当前 Rust checkout 时应显式选择 release binary：

```bash
cargo build --release --locked --bin shlog

npm run eval:perf -- \
  --bin ./target/release/shlog \
  --artifact ./target/release/shlog \
  --root /absolute/path/to/fixture/sessions \
  --db /absolute/path/to/fixture/index.sqlite
```

选择优先级为：

1. `--cli-argv-json` 提供的非空 JSON argv array；
2. `--bin`，内部等价为单元素 argv array；
3. 环境变量 `SHLOG_CLI_ARGV_JSON`；
4. 环境变量 `SHLOG_BIN_UNDER_TEST`；
5. TypeScript reference fallback。

`--artifact` 或 `SHLOG_ARTIFACT_UNDER_TEST` 指定要记录体积的 artifact。需要 launcher 与固定参数时使用 JSON argv，不做 shell split：

```bash
SHLOG_CLI_ARGV_JSON='["cargo","run","--quiet","--release","--bin","shlog","--"]' \
npm run eval:perf -- --root <root> --db <db> --skip-sync --json-only
```

命令行 executable override 高于对应环境变量。

## Safety 与输入范围

无参数的兼容默认值是本机默认 Codex root 与默认 state DB，并在读测试前执行 strict `sync`。这会修改所选 SQLite，适合明确的本机 dogfood，不适合作为隔离基准。

推荐显式传入 sanitized fixture root 和独立 DB。若 DB 已预建，使用 `--skip-sync`；此时 DB 必须存在，harness 不执行任何 sync：

```bash
npm run eval:perf -- \
  --bin ./target/release/shlog \
  --artifact ./target/release/shlog \
  --root /absolute/path/to/fixture/sessions \
  --db /absolute/path/to/fixture/index.sqlite \
  --skip-sync \
  --json-only
```

`status` 会按公开 contract 建立 live privacy-filtered inventory 并计算 requested selector coverage；它不返回/检索正文、不写 index，但 cache miss 可流式读取 raw accepted records/body，成本可能为 O(raw bytes)，exact `mtime_ns`/checkpoint cache hit 则不重 parse。`find`、`read-range`、`read-page`、`stats` 只读 index。Harness 会把显式 root 传给 status/find，但 find 不自行扫描 raw transcript freshness。

## 被测 command shapes

一次完整 run 包括：

- 可选的一次 `sync`；
- 带 canonical `all(root)` selector 的 `status`；
- 七种固定 query shape：英文单 token、短 token、多 token、CJK 和中英混合；
- 每个 query 的 `find --limit 10`；
- 对 top hit 分别运行 `read-range` 和 `read-page`；session-profile hit 用 query 重新锚定 range；
- 一次 `stats`，加 SQLite storage accounting。

当前固定 query 是 `hammerspoon`、`envchain`、`sb`、`fly deploy`、`edge tts`、`豆包输入法`、`部署 health check`。它们用于比较 tokenization 与 command shapes，不代表正式 relevance goldens。

## Timing contract

默认每个 status/find/read shape 调用 21 次：首轮 warmup，之后 20 个 measured samples。P50/P95 对 measured samples 使用线性插值。可用 `--runs`、`--read-runs`、`--status-runs` 缩短 smoke，但小样本不能作为 acceptance gate。

每条 latency record 包含：

- `processE2E`：父进程观测的完整 wall time，包括 executable/launcher 启动、命令工作、JSON 序列化与退出；
- `operation`：被测 JSON payload 的 `elapsedMs`（若提供），不是纯 SQLite timer；
- `processOverhead`：配对样本的 `max(0, processE2E - operation)`；
- legacy `runs`、`samplesMs`、`p50Ms`、`p95Ms`：继续镜像 `processE2E` 供旧 report consumer 使用。

`sync` 是有状态操作，只测一次，`syncMode` 标记 `run` 或 `skip`。

## RSS、artifact 与 DB storage

`--collect-rss` 为每种 command shape 的 warmup 采一个 peak RSS：

- macOS 使用 `/usr/bin/time -l`，单位 bytes；
- Linux 使用 `/usr/bin/time -v`，从 KiB 转 bytes；
- 不支持的平台或无法解析时为 `null`。

sync 没有可丢弃 warmup，因此启用 RSS 时其 wall time 包含少量 wrapper overhead。Harness 还会记录 executable/artifact size、DB file size、page/freelist 信息，并通过开发期 Node `node:sqlite` oracle + `dbstat` 估算各 SQLite object 体积。Node 只属于仓库 eval harness，不进入 standalone Rust CLI。

报告记录 count、latency、size、source/session reference 与 argv，不写 transcript message content。

## Dogfood integration

`--dogfood <goldens.jsonl>` 会附带执行 `eval/run-dogfood-eval.ts`。Dogfood runner 已复用同一 executable selector，支持 `--cli-argv-json`、`SHLOG_CLI_ARGV_JSON` 与 `SHLOG_BIN_UNDER_TEST`，并在 report 中记录 `cli_under_test`。

perf wrapper 会把自己解析后的 executable prefix 以 `--cli-argv-json` 转发给 dogfood 子进程，因此 `--bin`、`--cli-argv-json` 或环境变量选中的 candidate 都会保持一致：

```bash
npm run eval:perf -- \
  --bin ./target/release/shlog \
  --root <root> --db <db> --skip-sync \
  --dogfood data/cxs-dogfood/goldens.local.jsonl
```

无 executable override 时，dogfood 与其他 eval runner 一样默认使用 TypeScript oracle。

## 并发基准

`npm run eval:perf:concurrency` 是并发读路径的补充 harness。与串行 harness 不同，它测的是**同时多个独立 `shlog` 进程**访问同一个只读 SQLite index 时的吞吐与 tail latency。它不执行 `sync`，要求 `--db` 已存在。

```bash
npm run eval:perf:concurrency -- \
  --bin ./target/release/shlog \
  --root /absolute/path/to/fixture/sessions \
  --db /absolute/path/to/fixture/index.sqlite \
  --shapes "find:hammerspoon|find:edge tts|read-range|read-page|status" \
  --levels "1 2 4 8 16 32" \
  --total 80   # 每级并发总共跑多少 op
```

- executable selector 与串行 harness 完全一致（`--bin` / `--cli-argv-json` / 环境变量 / TS reference fallback）。
- command shapes：`find:<query>` 构造 `find` 命令；`read-range` / `read-page` / `status` 是字面命令。read shapes 会先用 `list --limit 1` 解析一个真实 session ref 作为 anchor；解析失败时只跳过该 shape 并警告，不使整个 run 失败。
- 方法：worker 池 + 共享任务队列，每个 op 独立 spawn 一个被测进程；并发度 = worker 数。每个 level 的记录包含：
  - `throughputPerSec`（完成 ops / wall time）
  - per-op `processE2E` 的 p50/p95/p99/max（毫秒）
  - payload `elapsedMs` 的 p50/p95/p99/max（若被测命令提供；`status` 不提供时为 `null`）
  - `errors`（非零退出计数）
- 默认写入 `data/shlog-perf/concurrency/<timestamp>/report.json` 与 `report.md`；`--json-only` 只向 stdout 输出。

### 本机基线（2026-08-17，Apple M4 / 10 核 / 16GB）

被测 `target/release/shlog` 0.5.1（native），真实 Codex index：6217 sessions / 318k messages / 420MB SQLite，热缓存。数字来自 `npm run eval:perf:concurrency`（`total=40`、`levels 1 2 4 8 16 32`），为 per-op E2E p50/p95（毫秒）与峰值吞吐（ops/s）：

| shape | 1 并发 p50/p95 | 4 并发 p50/p95 | 16 并发 p50/p95 | 峰值吞吐（@并发） |
|---|---|---|---|---|
| find:hammerspoon | 12.4 / 13.3 | 18.7 / 24.3 | 82.2 / 114.0 | 204.7 @4 |
| find:edge tts | 33.9 / 41.4 | 49.8 / 64.8 | 176.1 / 250.3 | 91.4 @16 |
| find:豆包输入法 | 13.1 / 13.9 | 17.8 / 19.2 | 74.7 / 105.8 | 222.2 @4 |
| read-range | 4.3 / 5.3 | 4.7 / 6.4 | 9.5 / 16.9 | 1257.6 @16 |
| read-page | 4.2 / 4.8 | 4.7 / 6.0 | 11.0 / 22.7 | 1226.9 @32 |
| status | 84.1 / 123.7 | 106.1 / 121.5 | 359.9 / 558.4 | 45.9 @8 |

> 该基线是热缓存稳态；冷启动首击明显更慢（`find` 首次可达 ~0.6s、`status` 首次 ~2.5s）。并发读路径没有写锁，实测 0 error；超过 ~8 并发时 find 类 latency 劣化明显，超过 ~16 后吞吐不再增长，建议作为限流参考而不是无限开并发。

## 输出

默认写入：

```text
data/shlog-perf/<timestamp>/report.json
data/shlog-perf/<timestamp>/report.md
```

`--json-only` 只向 stdout 输出完整 report，不创建上述目录。比较结果时至少保留 commandUnderTest、source/root/DB、syncMode、run counts 与 artifact identity；缺少这些上下文的数字不可作为回归结论。

# Sherlog Context

项目领域术语表。不含实现细节，仅定义领域概念及其边界。

<!--
  Maintainers: this is a glossary, not a spec. Terms are added lazily —
  only when they are resolved and need to be shared across agents.
  See domain-modeling skill for the definition format.
-->

## 架构分层

Sherlog 仓库有两条明确的技术栈边界：

| 层 | 语言 | 角色 | 路径 |
|---|---|---|---|
| **Production CLI** | Rust | 用户安装的 `shlog` binary：SQLite FTS5、tokenizer、sync、find/read/stats | `rust/src/`，产物 `target/release/shlog` |
| **Eval harness** | TypeScript | 开发期裁判：fork 被测 CLI 当子进程，观测延迟/吞吐/正确性/契约。不实现检索逻辑，不是 product runtime | `eval/`，`src/`（legacy TS oracle） |

eval harness 是"裁判"，Rust binary 是"选手"。`eval/perf-bench.ts` 做的事情是 `spawn(<candidate>, ["find", "豆包输入法", ...])` 然后掐表——它自身不执行 tokenization、不参与检索。contract-gate、acceptance-gate、dogfood runner、concurrency-bench 都遵循同一模式。Harness 可以用开发期 Node `node:sqlite` 读已生成 index 的 `dbstat` 做体积记账；这不是检索路径，也不进入发布态 CLI。

未传 `--bin` / `--cli-argv-json` 时，harness 默认测 TypeScript oracle，不是 Rust production candidate。

## 性能

- **合成烟雾**（synthetic smoke）：对确定性小 fixture 跑 `eval:perf` / `eval:perf:concurrency` 得到的延迟/吞吐数字。用来隔离回归、不碰开发者真实数据。不代表真实 Codex/Pi/Claude 的文件体积或命中基数。
  _Avoid_: 性能基线（在尚未有进 git 的对照 JSON 时）、可复现真实负载

- **本机校准**（private calibration）：开发者对自己已有 index 做的只读测量（必须同时显式 `--root` 与 `--db`，建议 `--skip-sync`）。回答「我这份库、这台机器上容量如何」。数字只对标定它的那份库和那台机器有意义，不进 git 当回归门。
  _Avoid_: 默认负载、CI 基线

- **性能回归门**（perf regression gate）：发布流程里对**合成烟雾**数字的一次性检查。当前尚未落地；计划只在 release workflow 跑，不在 PR CI 跑，也不用本机校准数字当门槛。

- **业务性能**（end-to-end CLI performance）：用户可感知的 CLI 命令进程级 wall-clock 延迟、内存与 DB 体积。由 `eval/perf-bench.ts`（串行）和 `eval/concurrency-bench.ts`（并发）测量。默认测合成烟雾；本机校准须显式 opt-in。

- **并发性能**（concurrency performance）：多个独立 `shlog` 只读进程同时访问同一 SQLite index 时的吞吐与 tail latency。用于容量观察，不进入回归门。

- **组件性能**（component-level micro-benchmark）：单个内部模块（如 tokenizer）的吞吐、分配次数、长文本缩放。当前尚不存在。

## 数据与负载

- **合成 fixture**（synthetic fixture）：由 `eval/perf-fixture.ts` 确定性生成的临时 Codex JSONL。按**消息条**抽签约 60% CJK / 25% Latin / 15% 路径，体积由 `--fixture-mb` 控制（默认 16MB）。相同参数跨机器可复现。这是合成烟雾的默认负载，不是真实会话的形状模型，也不用于正确性或相关性测试。
  _Avoid_: 真实语料、dogfood 数据、形状拟合语料

- **dogfood 数据**（dogfood data）：开发者本机的真实 agent session 历史。只用于质量线（dogfood eval、acceptance 的人工对照）和性能线的本机校准。perf harness 默认不触碰它。

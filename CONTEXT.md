# Sherlog Context

项目领域术语表。不含实现细节，仅定义领域概念及其边界。

<!--
  Maintainers: this is a glossary, not a spec. Terms are added lazily —
  only when they are resolved and need to be shared across agents.
  See domain-modeling skill for the definition format.
-->

## 性能

- **性能基线**（perf baseline）：在某指定机器类上，对固定工作负载（合成 fixture 或真实 dogfood 数据）跑 `eval:perf` / `eval:perf:concurrency` 产出的 p50/p95 延迟、吞吐、RSS、DB 体积等数字。基线 JSON 随代码一起进 git，作为回归门判据的参照点。基线只在标定它的那台机器类上有比较意义。

- **性能回归门**（perf regression gate）：发布流程中对性能基线的一次性检查——当某指标漂移超过容差（推荐默认 `max(baseline × 2.0, baseline + 150ms)`）时阻止发布，但提供显式 escape hatch 供人工复核后放行。当前只计划在 release workflow 跑，不在 PR CI 跑。

- **业务性能**（end-to-end CLI performance）：用户可感知的 CLI 命令（`find`/`read-range`/`read-page`/`status`/`sync`）的进程级 wall-clock 延迟、内存与 DB 体积。由 `eval/perf-bench.ts`（串行）和 `eval/concurrency-bench.ts`（并发）测量。

- **并发性能**（concurrency performance）：多个独立 `shlog` 只读进程同时访问同一 SQLite index 时的吞吐（ops/s）与 tail latency（p95/p99）。由 `eval/concurrency-bench.ts` 测量。用于容量分析，不进入回归门。

- **组件性能**（component-level micro-benchmark）：单个内部模块（如 tokenizer）的吞吐、分配次数、长文本缩放。当前尚不存在；计划以零依赖 `examples/` bench bin 实现，用 e2e 中文 fixture 的 sync/find 延迟作为间接回归保护。

## 数据与负载

- **合成 fixture**（synthetic fixture）：由 `eval/perf-fixture.ts` 确定性生成的临时 session 语料（Codex JSONL 格式）。内容 ~60% CJK + 25% Latin + 15% 路径/命令，体积由 `--fixture-mb` 控制（默认 16MB）。相同 seed ≡ 相同输出，跨机器可复现。用于性能基准的默认负载，不用于正确性测试。

- **dogfood 数据**（dogfood data）：开发者本机的真实 agent session 历史。只用于质量线（dogfood eval、acceptance gate）和性能线的显式 opt-in 模式（`--root`/`--db` 显式传参）。perf harness 默认不触碰真实数据。

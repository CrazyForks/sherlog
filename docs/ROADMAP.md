# Sherlog Roadmap

## 当前判断

Production runtime 已切到 standalone Rust：固定 command surface、v8 SQLite、source adapters、manual sync、coverage、progressive read 与 native distribution pipeline 都已进入当前 checkout。下一阶段的重点不是继续堆功能，而是把 native acceptance、incremental equivalence 与诊断可观测性做成可信 gate。

当前 retrieval：

```text
documents_fts candidate recall
  -> deterministic session aggregation/ranking
  -> evidenceRead
  -> read-range/read-page
```

message 与 session-profile 共用 `documents`/`documents_fts`，但 evidence provenance 分离。当前不是 resource-level reranker，也没有 duplicate collapse、vector retrieval、全文 fuzzy、watcher/daemon。

## P0: 持续提高 acceptance gate 的可信度

Native release 已存在，固定 CLI、跨 source 检索与发布资产验证已进入 CI。每版仍需独立回读 release、tap、官网与本机安装态，不能用本地 build 代替发布证据。

已有基础：

- `eval/` 只调用指定的 native binary；contract 与 evidence-level acceptance 会验证归档内的 Linux executable；
- Rust unit/integration tests 覆盖 sources、index、sync、migration、retrieval、app 与真实 CLI；
- Codex 新旧消息表示、隐私过滤、解释版本升级、空投影追加、分页连续追加与逐条增量/full 对比已进入聚焦测试；
- private dogfood runner 可验证已有本机验收集；新增与 promote 仍需用户显式触发 dev-only skill；
- synthetic perf 与用户指定 root/DB 的本机校准分开，尚无真实体积分布的持续回归基线。

继续补强 gold set / rubric / error taxonomy 与失败归因，不以单次测试全绿替代产品验收。

## P1: `incremental == full replay` acceptance

目标：证明 append cursor 是性能优化，不是第二套语义。

把现有 focused tests 扩成 property/state-machine matrix：

- append；
- truncate；
- same-size/prefix rewrite；
- rename/path identity change；
- unterminated tail；
- hot -> cold；
- cold remove + explicit prune；
- per-file scan failure strict vs best-effort；
- migration crash at each publication phase；
- pre-existing `.next`/WAL/SHM；
- old TypeScript DB writer 与 cold-config writer competition。

每条 state trace 都比较最终 sessions/documents/FTS/source_files/coverage/cold_roots invariant 与 full replay/reference state。不能只比较 command exit code。

## P2: Retrieval 可观测性

增加能区分问题层级的计数，而不是先改 ranking：

```text
indexed scope
  -> FTS/LIKE candidate count
  -> SQL scope-filtered count
  -> relaxed-recall count
  -> grouped session count
  -> returned count
  -> evidence-read outcome
```

目标 public/debug fields：

- `matchMode`（exact FTS / literal fallback / relaxed）；
- `weakMatch` 与明确原因；
- candidate -> filter -> rank stage counts；
- per-source counts；
- zero-result reason 与 coverage proof 的关联。

要求：不开启 debug 时不显著扩大 payload；不暴露 private raw；计数必须能判定是 coverage、candidate、filter、ranking 还是 evidence read 问题。

## P3: Eval 与 source hardening

- 扩充真实 query/golden，但 private dogfood 仍由用户显式触发 dev-only skill 采集；
- 对 title/summary/compact/reasoning-only hit 加稳定断言；
- 对 `matchSource=session`、`matchSeq=null`、`evidenceRead` 可执行性加 contract test；
- 持续验证 default cross-source find、selector isolation、coverage 不跨 source、sessionRef round-trip；
- 增加 source-specific negative privacy fixtures；
- 观察 Claude Code/Pi upstream format drift，再决定是否维持 raw transcript reader 或切换稳定 upstream interface。

## P4: Eval-driven recall/ranking experiments

只有 P0-P3 能给出稳定归因后才探索：

### Controlled typo fallback

- 只考虑 title/cwd/model/identifier；
- 只在 exact/relaxed zero result 后启用；
- 必须标注 `matchMode`/weak reason；
- 不对全部正文做无界 fuzzy。

### Evidence-read frecency

- 只能作为有上限、短半衰期的 tie-breaker；
- 防 self-hit、反馈循环与“越读越高”的锁定效应；
- 必须在关闭该信号时仍保持内容召回完整。

### Ranking weight changes

- 先写分类 eval，再改常量；
- 同时观察 relevance 与 `--sort ended/started`；
- 不能为单个 dogfood query hardcode；
- message evidence anchor 与 session-profile score 继续分离。

## Deferred

- independent stage-2/resource-level reranker；
- richer event projection/range cache；
- duplicate family collapse/diversity control；
- embeddings/vector store/heavier model；
- watcher/daemon/realtime sync；
- LMDB 或其他第二持久化真相；
- Linux arm64/musl/Windows release target。

这些不是 Rust cutover 的完成条件。FFF 类常驻内存 file picker 与 Sherlog 的短命、显式 sync、SQLite projection 模型互补，不合并存储或生命周期。

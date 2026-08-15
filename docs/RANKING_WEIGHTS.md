# Sherlog ranking 当前规则

本文描述 production Rust retrieval 的确定性规则。实现入口是
[ranking.rs](../rust/src/retrieval/ranking.rs)、
[reader.rs](../rust/src/index/reader.rs) 与
[global.rs](../rust/src/retrieval/global.rs)。`src/**` 中的 TypeScript 排序实现只用于开发期 differential oracle，不是 production 真相。

## Pipeline 与候选上限

```text
documents_fts / literal LIKE
  -> SQL scope filters
  -> bounded document candidates
  -> session aggregation and heuristic ranking
  -> message-preferred display evidence
```

- query token 以 `AND` 组成 FTS5 expression；极少数无法产生 token 的 CJK query 使用 literal `LIKE` fallback。
- root、cwd、date、session、exclude 等可表达约束应在 SQL candidate generation 阶段下推。
- relevance 排序读取 `max(limit * 12, 50)` 个 document candidates，并完整 rerank。
- `--sort ended` / `--sort started` 读取 `max(limit * 4, 50)` 个按时间预排的 candidates；先按 session 去重，只对最多 `limit * 2` 个最新 session 做 heuristic rerank，最后恢复时间顺序。

对任何未来快速预过滤层的硬约束是 conservative superset：只能排除可证明不匹配的项，不能制造 false negative。tokenizer 无法确定或索引状态不明时应保守纳入，再由精确层验证。当前 bounded FTS/LIKE recall 仍需 acceptance/eval 证明候选上限没有造成不可接受的 recall loss；不能仅凭架构意图宣称零漏召回。

## FTS5 列权重

v8 的 `documents_fts` 与 `documents` 一一对应，列顺序和 `bm25` 权重如下：

| projection field | weight | 说明 |
|---|---:|---|
| `body_text` | 1.0 | message 正文；session-profile document 中为空 |
| `title_text` | 8.0 | 高密度 session 标题信号 |
| `summary_text` | 3.0 | session summary |
| `compact_text` | 4.0 | compact handoff，通常保留更多具体术语 |
| `reasoning_text` | 1.2 | reasoning summary，权重最低以限制泛匹配噪音 |

SQLite FTS5 `bm25()` 越小越好。Rust row score 使用 `-fts_score` 转成越大越好。v7 compatibility reader 仍可读取旧 `messages_fts` / `sessions_fts`，但它们不是 v8 storage truth。

## Document row score

普通 row 的分数是：

```text
-fts_score
  + exact content phrase       8
  + path-like command sequence bonus
  + matched query term count * 2
  + message document           4
  + user role                  2
```

这些 signal 的职责不同：FTS 提供基础相关性，完整短语和 term coverage 补足 tokenizer 后丢失的邻接信息，message/user bonus 轻量偏好可回读原始消息与用户表述。

### Path-like command 特例

含空白且包含 `\\ / . _ : -` 之一的 query 进入 path-like command 模式：

- 有 token boundary 的完整 phrase：额外 `+36`；若同一行后续 80 个 UTF-16 code units 内出现 path、文件扩展样式或 flag 参数，再 `+24`。
- 无完整 phrase 但存在有序 token span：零 gap `+8`；1-3 gap 为 `24 - 2 * gap`；gap 不超过 term 数时为 `10 - gap`；更松散时不加分。
- 完整 phrase 仍同时获得普通 content phrase 的 `+8`。

这是为了把真实命令执行上下文与自然语言里的松散复述分开，不是全文 fuzzy。

## Session score

每个 session 先保留最高 row score，再叠加：

```text
best row score
  + exact title phrase             30
  + title term hits *              10
  + cwd term hits *                18
  + min(user hit count, 3) *        4
  + min(session-profile hits, 2) *  2
  + min(all hit count, 6) *         1.5
  + max(0, 18 - age_days * 0.15)
  - command-restatement title       20
```

recency bonus 在 120 天后归零。它是 relevance 的有限补强，不代表“最新优先”；显式时间意图应使用 `--sort ended` 或 `--sort started`。

path-like command query 若完整出现在 title，但删除该 phrase 后 title 仍有至少两个 token，则 title 被视为 command restatement：不再获得 title phrase / term bonus，并扣 20 分。这防止自动标题复述查询压过正文中的真实命令证据。

## Ranking row 与 evidence anchor 分离

一个 session 同时保留：

- `best_row`：决定相关性分数；
- `best_display_row`：决定 `snippet`、`matchSource`、`matchSeq` 与 progressive read anchor。

只要存在 message candidate，display row 就硬优先 message；只有 source kind 相同时才比较 row score。session-profile 命中可以帮助召回和排序，但不能伪装成 message 证据。若只有 profile hit，`matchSource=session`、`matchSeq=null`，`evidenceRead` 应使用 query 重新锚定 `read-range` 或回退 `read-page`。

## Cross-source merge

未限定 source 的 relevance find 先在每个 source 内独立排序，再以 reciprocal-rank score 合并：

```text
1 / (60 + source-local rank)
```

这避免不同 source 的原始 heuristic score scale 直接互相压制。时间排序则直接按 started/ended timestamp 合并，并用 score、source、sessionRef 做稳定 tie-break。

## 修改规则

任何调权都应先补分类 eval，再改常量：

1. 说明变化位于 candidate、row、session、time sort 还是 cross-source merge。
2. 同时检查 recall completeness 与 returned evidence，不只看 top-1。
3. 保持 candidate conservative superset、SQL filter pushdown、ranking/evidence anchor 分离三项不变量。
4. 用 native candidate 跑 acceptance/contract/perf；TypeScript 结果只用于 differential diagnosis。
5. 不为单个 private dogfood query hardcode，也不引入无界正文 fuzzy 或未受控 frecency。

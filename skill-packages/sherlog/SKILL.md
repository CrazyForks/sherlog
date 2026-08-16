---
name: sherlog
description: "Search local agent-session history to recover prior decisions, commands, configurations, and context from earlier sessions. Use for cross-session recall when the current task depends on what happened in a previous agent conversation, not for inspecting the current repository or summarizing the current session."
---

# Sherlog

用 standalone `shlog` 在本机 Sherlog SQLite index 中检索 agent 历史。Node.js 不是 CLI runtime dependency。

## Executable

每轮先解析一次并记录绝对 executable：

```sh
if [ "${SHLOG_BIN+x}" = x ]; then
  SHLOG_CANDIDATE=$SHLOG_BIN
elif [ "${CXS_BIN+x}" = x ]; then
  SHLOG_CANDIDATE=$CXS_BIN
else
  SHLOG_CANDIDATE=shlog
fi

case $SHLOG_CANDIDATE in
  */*) SHLOG=$SHLOG_CANDIDATE ;;
  *) SHLOG=$(command -v "$SHLOG_CANDIDATE" 2>/dev/null) || {
    echo "shlog not found: $SHLOG_CANDIDATE" >&2
    exit 1
  } ;;
esac

case $SHLOG in
  /*) ;;
  */*)
    SHLOG_DIR=$(CDPATH= cd -P "$(dirname "$SHLOG")" 2>/dev/null && pwd -P) || {
      echo "shlog not executable: $SHLOG_CANDIDATE" >&2
      exit 1
    }
    SHLOG=$SHLOG_DIR/$(basename "$SHLOG")
    ;;
  *)
    echo "shlog is not an executable file: $SHLOG_CANDIDATE" >&2
    exit 1
    ;;
esac

[ -f "$SHLOG" ] && [ -x "$SHLOG" ] || {
  echo "shlog not executable: $SHLOG_CANDIDATE" >&2
  exit 1
}
printf '%s\n' "$SHLOG"
```

`SHLOG_BIN` / `CXS_BIN` 只要存在（包括空值）就拥有优先级；候选不可执行时直接报错，不回退 PATH。不要依赖 shell 变量跨 tool call 持久化：把输出的绝对路径保存在本轮 agent 状态中，后续每次调用都使用该字面路径或在同一 shell 设为 `SHLOG`。`evidenceRead.command.executable == "inherit"` 时用它替换 sentinel；typed-error `nextAction.commands[].argv[0] == "shlog"` 时也只替换 argv[0]，其余 argv 逐项不变。

## 主循环

1. **定位**：按问题选 `list` / `find` / metadata SQL。完成：有 candidate identity（source + sessionRef）和明确 scope。
2. **取证**：执行 candidate 的 `evidenceRead.command`，或 `read-range` / `read-page`。完成：每个历史事实都有 `read-*` 内容支持；`hasMore=true` 且目标上下文或完整性仍未解决时继续翻页，抽样回答则明确样本边界。
3. **证明范围**：只在回答依赖 latest / completeness / miss 结论时，对相同 selector 跑 `status --json`。完成：按 `recommendedAction` 决定 query、同范围 sync，或说明边界。
4. **自评**：结论是否有内容证据、范围结论是否有 live coverage proof、不确定处是否已说明。

完成 executable 解析后直接 query，不预跑版本检查、status 或 sync。错误发生时按 `references/failure-cookbook.md` 的恢复表执行，不要用无条件全量 sync 处理所有错误。

## 不变量

- `find/read/list/stats/cold list` 只读；`sync` 是唯一 content writer；`cold add/remove` 只写 retention state。只读命令不隐式 sync/migrate。
- 内容事实只能来自 `read-*`；title/snippet/profile 命中只是 candidate。
- message anchor 走 `read-range`；profile-only 命中不伪造 seq；`anchor_not_found` 时按 nextAction 回退。
- 无 `--root/--cwd/--selector` 的 find 只搜 canonical default `all(root)`；latest/completeness/miss 结论必须匹配同 selector 的 coverage proof。
- 破坏性操作（`sync --prune`、`cold remove`）只在用户明确表达删除意图后执行。

## References（按触发条件加载）

- 遇到 typed error / zero results / 迁移升级：`references/failure-cookbook.md`
- 需要参数、默认值、selector JSON：`references/cli-surface.md`
- 需要字段和 error shape 定义：`references/json-schema.md`
- 需要具体场景示例与完成标准：`references/progressive-workflow.md`
- 需要 tokenizer/FTS/read-only SQL 细节：`references/advanced-queries.md`

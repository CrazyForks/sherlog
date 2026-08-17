/**
 * Deterministic Codex-format smoke fixture for Sherlog performance harnesses.
 *
 * Generates short session transcripts into a temporary directory. Message
 * draws are ~60% CJK / ~25% Latin / ~15% paths (per message, not per byte,
 * and not a real size histogram). This is isolated regression smoke — it is
 * not a shape-faithful model of developer Codex/Pi/Claude corpora.
 *
 * Volume is controlled by `--fixture-mb` (default 16 MB of body text).
 * All output is deterministic given the same megabyte parameter — same seed
 * produces identical sessions on any machine.
 *
 * Usage:
 *   import { generateFixture, cleanupFixture } from "./perf-fixture";
 *   const f = generateFixture(16);
 *   // ... run benchmarks against f.root + f.db ...
 *   cleanupFixture(f);
 */

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

// ── types ───────────────────────────────────────────────────────────────────

export interface FixturePaths {
  /** Temp directory containing generated session transcripts. */
  root: string;
  /** Recommended temp db path (not created by the generator — harness owns sync). */
  db: string;
  /** Number of sessions written. */
  sessionCount: number;
  /** Approximate total body bytes (message text only). */
  bodyBytes: number;
}

// ── CJK seed pool (~60% of content) ────────────────────────────────────────

const CJK_SEEDS = [
  "健康检查服务在部署后恢复正常，所有节点通过验证。",
  "回滚预案需要在发布前经过两次以上的演练确认。",
  "数据库迁移脚本已通过预发布环境的完整性校验。",
  "日志聚合器在峰值流量下出现了短暂的磁盘写入延迟。",
  "配置文件中的环境变量引用在构建时未能正确展开。",
  "安全审计发现三个低危漏洞，建议在下个迭代修复。",
  "预发布环境与生产环境的网络策略不一致导致连接超时。",
  "负载均衡器的健康检查端点返回了非预期的状态码。",
  "容器镜像的构建缓存未命中，全量构建耗时超出预算。",
  "服务网格的边车代理在滚动更新时丢失了短暂连接。",
  "分布式追踪采样率从百分之一调整为千分之一以节省存储。",
  "密钥轮换脚本忽略了命名空间级别的资源引用。",
  "弹性伸缩策略的冷却时间设置过短导致频繁扩缩。",
  "监控告警规则在周末误报了两次，阈值得重新校准。",
  "灰度发布的新版本在十分钟后被自动回滚到上一个稳定版。",
  "缓存穿透防护对高频热点路径仍然依赖单节点互斥锁。",
  "查询优化器选择的索引未能覆盖 WHERE 子句的全部谓词。",
  "消息队列的消费者组发生了分区重平衡，导致短暂消费中断。",
  "证书将在四十八小时内过期，自动续期任务需要人工确认。",
  "跨可用区的同步复制延迟在高峰时段超过阈值，触发了主从切换。",
  "数据校验发现昨天的增量备份缺失了两千条记录。",
  "API 网关的限流规则对内部服务之间的调用误加了全局限流。",
  "构建流水线的缓存层在本次提交命中率仅为百分之十二。",
  "协程泄漏导致事件循环在第七个小时后响应显著变慢。",
  "覆盖率报告丢失了集成测试的命中行统计。",
  "分词器在处理补充平面汉字时产生了重复的标量二元组。",
  "索引重建过程中数据库文件大小峰值达到了正常值的四倍。",
  "查询计划显示 FTS5 跳过了内容表直接扫描了影子表。",
  "会话摘要的紧凑文本中遗漏了推理链的关键结论。",
  "指纹计算在解析超大单行 JSON 时超时，需要流式分块处理。",
  // Terms matching every BENCH_QUERIES shape so the harness find+read path doesn't crash:
  "豆包输入法在 SwiftUI 上的体验比原生键盘好很多，尤其符号布局。",
  "部署健康检查脚本失败，原因是目标主机的 SSH 密钥已过期。",
  "今天把 hammerspoon 的窗口管理配置从 0.9 迁移到了 1.0 语法。",
  "新的 envchain 集成支持 namespace 级别的密钥隔离。",
  "重构 sb 模块的查询构造器，把动态 ORDER BY 改成固定索引扫描。",
  "fly deploy 超时是因为 Docker 构建缓存层在 CI 上不可用。",
  "edge tts 服务的 gRPC 端点需要在网关层增加连接池配置。",
  "部署 health check 发现两个节点没有拉取最新的配置中心变更。",
  "Hammerspoon 需要检测到外接显示器变化后自动重新布局所有窗口。",
  "Envchain 支持通过环境变量注入通配符匹配的密钥集合。",
  "sb 的日志输出格式在 debug 模式下打印了完整的 AST 节点。",
  "Fly deploy 前应该先验证 Turbosrc 仓库有没有未推送的 commit。",
  "Edge TTS 的语音合成出现了偶发的音节丢失，采样率可能不匹配。",
];

// ── Latin seed pool (~25% of content) ───────────────────────────────────────

const LATIN_SEEDS = [
  "Deploy health-check endpoint returned 200 after rollback.",
  "The CI pipeline failed at the integration-test stage due to a missing secret.",
  "Refactor the parser to handle edge cases in Unicode normalization.",
  "Performance regression detected in the tokenizer bigram generation path.",
  "The session index is missing coverage for the newly added source adapter.",
  "FTS5 contentless table requires explicit column rebuild after schema change.",
  "Lock contention on the WAL checkpoint is visible above 8 concurrent readers.",
  "Incremental sync produced a different document count than full replay.",
  "The evidence read plan must preserve message-level provenance for CJK hits.",
  "Query analysis falls back to literal LIKE when zero FTS tokens are produced.",
  "The ranking heuristic over-weights session-profile hits on single-word queries.",
  "Database page size affects B-tree fanout for the per-source coverage table.",
  "The cold retention registration file was tombstoned after v7-to-v8 migration.",
  "Sanitized fixture generation should be deterministic across platform locales.",
  "The write-ahead log grows unboundedly when no checkpoint occurs between syncs.",
  "Compact text summarisation dropped the domain-specific terminology in the conclusion.",
  "A dangling junction symlink prevented the plugin loader from resolving the entry.",
  "The acceptance gate uses synthetic UUIDs and deterministic timestamps for repeatability.",
  "Coverage proof compares source file digests without re-reading historical rows.",
  "Strict sync fails closed on malformed JSONL records and reports the byte offset.",
  // Terms matching single/dual-token queries so harness find+read doesn't crash:
  "Hammerspoon window manager layout is configured via Lua scripting.",
  "Envchain stores secrets per-project and injects them into shell sessions.",
  "The sb tool is a search backend that accelerates text queries.",
  "Fly deploy failed because the Dockerfile referenced a stale base image tag.",
  "Edge tts latency improved after switching to the premium neural voice tier.",
];

// ── path / command templates (~15% of content) ──────────────────────────────

const PATH_TEMPLATES = [
  (i: number) => `cd /tmp/shlog-perf-fixture/project-${i % 20} && node dist/cli.js publish fixtures/sample.jsonl --json`,
  (i: number) => `cd /Users/dev/work/repos/project-${i % 15} && cargo build --release --bin service-${i % 5}`,
  (i: number) => `cd /opt/deploy/env-${i % 3}/config && kubectl apply -f deployment-${i % 10}.yaml`,
  (i: number) => `grep -rn "tokenize" /src/rust/tokenizer-${i % 8}.rs | wc -l`,
  (i: number) => `find /var/log/service-${i % 7} -name "*.log" -mtime -${1 + (i % 7)} | head -20`,
  (i: number) => `curl -s http://localhost:${8000 + i}/health | jq .status`,
  (i: number) => `cat /etc/config-${i % 4}/defaults.json | python3 -m json.tool > /dev/null`,
  (i: number) => `systemctl restart daemon-${i % 6} && journalctl -u daemon-${i % 6} -n 5 --no-pager`,
];

// ── deterministic pRNG (mulberry32) ─────────────────────────────────────────

function mulberry32(seed: number): () => number {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function pick<T>(arr: readonly T[], rand: () => number): T {
  return arr[Math.floor(rand() * arr.length)];
}

// ── message generation ──────────────────────────────────────────────────────

interface CodexLine {
  timestamp: string;
  type: string;
  payload: Record<string, unknown>;
}

function sessionMeta(id: string, cwd: string, timestamp: string): CodexLine {
  return { timestamp, type: "session_meta", payload: { id, cwd } };
}

function turnContext(timestamp: string): CodexLine {
  return { timestamp, type: "turn_context", payload: { model: "gpt-5.4" } };
}

function eventMessage(
  type: "user_message" | "agent_message",
  message: string,
  timestamp: string,
): CodexLine {
  return { timestamp, type: "event_msg", payload: { type, message } };
}

function sequentialTimestamp(base: Date, offsetSec: number): string {
  return new Date(base.getTime() + offsetSec * 1000).toISOString();
}

// ── public API ──────────────────────────────────────────────────────────────

/**
 * Generate a deterministic CJK-heavy fixture for performance benchmarking.
 *
 * @param megabytes Approximate body text volume in MB. Default 16.
 * @param source "codex" (default) — only codex-format sessions for now.
 */
export function generateFixture(megabytes = 16, source = "codex"): FixturePaths {
  if (source !== "codex") throw new Error("Only codex source fixtures are supported");

  const root = mkdtempSync(join(tmpdir(), "shlog-perf-fixture-"));
  const db = join(root, "index.sqlite");
  const sessionsDir = join(root, "2026", "08", "17");
  mkdirSync(sessionsDir, { recursive: true });

  const targetBytes = megabytes * 1024 * 1024;
  const baseDate = new Date("2026-08-17T00:00:00.000Z");
  const rand = mulberry32(megabytes); // deterministic given --mb

  let bodyBytes = 0;
  let sessionCount = 0;

  while (bodyBytes < targetBytes) {
    const id = `50000000-0000-4000-8000-${String(sessionCount).padStart(12, "0")}`;
    const cwd = `/tmp/shlog-perf-fixture/project-${sessionCount % 20}`;
    const lines: string[] = [];

    // Session meta + turn_context
    const t0 = sessionCount * 120; // 120s spacing between sessions
    lines.push(JSON.stringify(sessionMeta(id, cwd, sequentialTimestamp(baseDate, t0))));
    lines.push(JSON.stringify(turnContext(sequentialTimestamp(baseDate, t0 + 0.5))));

    // 3–10 messages per session
    const msgCount = 3 + (sessionCount % 8);
    for (let m = 0; m < msgCount; m++) {
      const type = m % 2 === 0 ? "user_message" : "agent_message";
      const bucket = rand();
      let message: string;

      if (bucket < 0.60) {
        // CJK
        message = pick(CJK_SEEDS, rand);
      } else if (bucket < 0.85) {
        // Latin
        message = pick(LATIN_SEEDS, rand);
      } else {
        // Path / command
        message = pick(PATH_TEMPLATES, rand)(sessionCount + m);
      }

      lines.push(
        JSON.stringify(
          eventMessage(type as "user_message" | "agent_message", message, sequentialTimestamp(baseDate, t0 + 1 + m)),
        ),
      );
      bodyBytes += Buffer.byteLength(message);
    }

    writeFileSync(join(sessionsDir, `rollout-${id}.jsonl`), lines.join("\n") + "\n");
    sessionCount++;
  }

  return { root, db, sessionCount, bodyBytes };
}

/** Remove the temp fixture directory. */
export function cleanupFixture(f: FixturePaths): void {
  rmSync(f.root, { recursive: true, force: true });
}

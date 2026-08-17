import { resolve } from "node:path";
import { resolveCommandUnderTest, type CommandUnderTest } from "./perf-bench-core";
import { resolvePerfWorkload, type PerfWorkloadKind } from "./perf-data-source";

/**
 * Pure helpers for the concurrency benchmark harness (`concurrency-bench.ts`).
 *
 * Design: the runner (`concurrency-bench.ts`) stays thin and async; everything
 * that can be reasoned about deterministically lives here and is unit-tested:
 * argument parsing, per-level latency aggregation, command-shape construction
 * and the markdown report builder.
 */

export const DEFAULT_LEVELS = [1, 2, 4, 8, 16, 32];
export const DEFAULT_TOTAL_PER_LEVEL = 80;
export const DEFAULT_SHAPES = [
  "find:hammerspoon",
  "find:edge tts",
  "find:豆包输入法",
  "read-range",
  "read-page",
  "status",
];

export interface ConcurrencyArgs {
  workload: PerfWorkloadKind;
  root: string;
  db: string;
  source: string;
  /** Command shapes. A `find:<query>` shape maps to a `find` invocation; the
   *  literal shapes `read-range`, `read-page` and `status` map to their
   *  commands. */
  shapes: string[];
  levels: number[];
  totalPerLevel: number;
  jsonOnly: boolean;
  commandUnderTest: CommandUnderTest;
  /** When set, generate fixture of this many MB before running. */
  fixtureMb: number;
  keepFixture: boolean;
}

export interface OpSample {
  ok: boolean;
  exitCode: number | null;
  e2eMs: number;
  opMs: number | null;
  stdoutLen: number;
  stderr: string;
}

export interface LevelStats {
  level: number;
  total: number;
  wallMs: number;
  throughputPerSec: number;
  errors: number;
  p50E2E: number;
  p95E2E: number;
  p99E2E: number;
  maxE2E: number;
  p50Op: number | null;
  p95Op: number | null;
  p99Op: number | null;
  maxOp: number | null;
  opSampleCount: number;
}

export interface ShapeLevelResult {
  shape: string;
  command: string[];
  levels: LevelStats[];
}

export interface ConcurrencyReport {
  generatedAt: string;
  commandUnderTest: CommandUnderTest;
  sourceId: string;
  dbPath: string;
  rootDir: string;
  sessionCount: number;
  messageCount: number;
  totalPerLevel: number;
  shapes: ShapeLevelResult[];
}

export function parseConcurrencyArgs(argv: string[]): ConcurrencyArgs {
  let root = "";
  let db = "";
  let source = "codex";
  let jsonOnly = false;
  let shapes = [...DEFAULT_SHAPES];
  let levels = [...DEFAULT_LEVELS];
  let totalPerLevel = DEFAULT_TOTAL_PER_LEVEL;
  let executable: string | undefined;
  let cliArgvJson: string | undefined;
  let artifactPath: string | undefined;
  let fixtureMb = 16;
  let fixtureMbExplicit = false;
  let explicitRoot = false;
  let explicitDb = false;
  let keepFixture = false;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const next = () => argv[++i];
    if (a === "--root") {
      root = resolve(next() ?? root);
      explicitRoot = true;
    } else if (a === "--db") {
      db = resolve(next() ?? "");
      explicitDb = true;
    } else if (a === "--source") source = next() ?? source;
    else if (a === "--shapes") shapes = parseShapes(next() ?? "");
    else if (a === "--levels") levels = parseLevels(next() ?? "");
    else if (a === "--total") totalPerLevel = parsePositiveInt(next(), DEFAULT_TOTAL_PER_LEVEL);
    else if (a === "--fixture-mb") {
      fixtureMb = parsePositiveInt(next(), 16);
      fixtureMbExplicit = true;
    } else if (a === "--fixture") { /* no-op alias: synthetic smoke is the default when both paths are omitted */ }
    else if (a === "--keep-fixture") keepFixture = true;
    else if (a === "--bin") executable = next();
    else if (a === "--cli-argv-json") cliArgvJson = next();
    else if (a === "--artifact") artifactPath = next();
    else if (a === "--json-only") jsonOnly = true;
    else if (a === "--help" || a === "-h") {
      throw new HelpRequested();
    }
  }
  const workload = resolvePerfWorkload({ explicitRoot, explicitDb, fixtureMbExplicit });
  const commandUnderTest = resolveCommandUnderTest({
    root: ROOT,
    cliEntry: CLI_ENTRY,
    executable,
    argvJson: cliArgvJson,
    artifactPath,
  });
  return {
    workload: workload.kind,
    root,
    db,
    source,
    shapes,
    levels,
    totalPerLevel,
    jsonOnly,
    commandUnderTest,
    fixtureMb,
    keepFixture,
  };
}

export class HelpRequested extends Error {
  constructor() {
    super("help requested");
    this.name = "HelpRequested";
  }
}

export const USAGE = `Usage: npm run eval:perf:concurrency -- \\
  [--fixture-mb <n>] [--keep-fixture] | --root <sessions> --db <index.sqlite> \\
  [--source <id>] [--shapes "find:hammerspoon|read-range|read-page|status"] \\
  [--levels "1 2 4 8 16 32"] [--total 80] \\
  [--bin <executable> | --cli-argv-json <json>] [--artifact <path>] [--json-only]`;

/** Literal command shapes that must not be reinterpreted as find queries. */
const RESERVED_SHAPES = new Set(["read-range", "read-page", "status"]);

/** Parse `a|b|c` shape list; bare tokens become `find:<token>` unless they are
 *  reserved literal shapes (`read-range`, `read-page`, `status`). */
export function parseShapes(raw: string): string[] {
  const parts = raw.split("|").map((s) => s.trim()).filter(Boolean);
  if (parts.length === 0) return [...DEFAULT_SHAPES];
  return parts.map((part) => (part.includes(":") || RESERVED_SHAPES.has(part) ? part : `find:${part}`));
}

export function parseLevels(raw: string): number[] {
  const values = raw.split(/\s+/).map(Number).filter((n) => Number.isFinite(n) && n > 0);
  return values.length > 0 ? [...new Set(values)].sort((a, b) => a - b) : [...DEFAULT_LEVELS];
}

export function parsePositiveInt(value: string | undefined, fallback: number): number {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

/**
 * Build the CLI argv for a shape. Read shapes require a resolved session ref;
 * when it is missing they return `null` so the runner can skip them with a
 * clear message instead of running a failing command.
 */
export function shapeCommand(
  shape: string,
  ctx: { source: string; root: string; db: string; sessionRef: string | null },
): string[] | null {
  if (shape === "read-range" || shape === "read-page") {
    if (!ctx.sessionRef) return null;
    if (shape === "read-range") {
      return ["read-range", ctx.sessionRef, "--source", ctx.source, "--seq", "0", "--before", "2", "--after", "2", "--db", ctx.db, "--json"];
    }
    return ["read-page", ctx.sessionRef, "--source", ctx.source, "--offset", "0", "--limit", "40", "--db", ctx.db, "--json"];
  }
  if (shape === "status") {
    return [
      "status",
      "--source",
      ctx.source,
      "--root",
      ctx.root,
      "--selector",
      JSON.stringify({ source: ctx.source, kind: "all", root: ctx.root }),
      "--db",
      ctx.db,
      "--json",
    ];
  }
  if (shape.startsWith("find:")) {
    const query = shape.slice("find:".length);
    return [
      "find",
      query,
      "--source",
      ctx.source,
      "--root",
      ctx.root,
      "--db",
      ctx.db,
      "--limit",
      "10",
      "--json",
    ];
  }
  throw new Error(`unknown shape: ${shape}`);
}

/** Aggregate per-op samples into per-level latency/throughput stats. */
export function aggregateLevelStats(
  level: number,
  total: number,
  wallMs: number,
  samples: OpSample[],
): LevelStats {
  const e2e = samples.map((s) => s.e2eMs).sort((a, b) => a - b);
  const opSamples = samples.filter((s) => s.opMs !== null).map((s) => s.opMs as number).sort((a, b) => a - b);
  return {
    level,
    total,
    wallMs: round2(wallMs),
    throughputPerSec: round2((samples.length / wallMs) * 1000),
    errors: samples.filter((s) => !s.ok).length,
    p50E2E: round2(percentileFrom(e2e, 0.5)),
    p95E2E: round2(percentileFrom(e2e, 0.95)),
    p99E2E: round2(percentileFrom(e2e, 0.99)),
    maxE2E: round2(e2e.length ? e2e[e2e.length - 1]! : 0),
    p50Op: opSamples.length ? round2(percentileFrom(opSamples, 0.5)) : null,
    p95Op: opSamples.length ? round2(percentileFrom(opSamples, 0.95)) : null,
    p99Op: opSamples.length ? round2(percentileFrom(opSamples, 0.99)) : null,
    maxOp: opSamples.length ? round2(opSamples[opSamples.length - 1]!) : null,
    opSampleCount: opSamples.length,
  };
}

function percentileFrom(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  if (sorted.length === 1) return sorted[0]!;
  const pos = (sorted.length - 1) * Math.min(1, Math.max(0, p));
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  const l = sorted[lo]!;
  const h = sorted[hi]!;
  return l + (h - l) * (pos - lo);
}

function round2(value: number): number {
  return Number(value.toFixed(2));
}

export function fmtMs(n: number): string {
  return n.toFixed(1).padStart(8);
}

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function buildConcurrencyReportMarkdown(r: ConcurrencyReport): string {
  const lines: string[] = [];
  lines.push("# shlog 并发性能基准报告");
  lines.push("");
  lines.push(`- generated_at: ${r.generatedAt}`);
  lines.push(`- command: \`${[r.commandUnderTest.executable, ...r.commandUnderTest.prefixArgv].join(" ")}\``);
  lines.push(`- command_source: ${r.commandUnderTest.source}`);
  lines.push(`- resolved_executable: \`${r.commandUnderTest.resolvedExecutablePath ?? "unresolved"}\``);
  lines.push(`- executable_size: ${r.commandUnderTest.executableSizeBytes === null ? "-" : fmtBytes(r.commandUnderTest.executableSizeBytes)}`);
  lines.push(`- artifact: \`${r.commandUnderTest.artifactPath ?? "unresolved"}\``);
  lines.push(`- source: \`${r.sourceId}\``);
  lines.push(`- root: \`${r.rootDir}\``);
  lines.push(`- db: \`${r.dbPath}\``);
  lines.push(`- session_count: ${r.sessionCount}`);
  lines.push(`- message_count: ${r.messageCount}`);
  lines.push(`- total_ops_per_level: ${r.totalPerLevel}`);
  lines.push("");
  for (const shape of r.shapes) {
    lines.push(`## ${shape.shape}`);
    lines.push("");
    lines.push(`command: \`shlog ${shape.command.join(" ")}\``);
    lines.push("");
    lines.push("| level | ops/sec | errors | p50 E2E | p95 E2E | p99 E2E | max E2E | p50 op | p95 op | p99 op | max op |");
    lines.push("|------:|--------:|-------:|--------:|--------:|--------:|--------:|-------:|-------:|-------:|-------:|");
    for (const lvl of shape.levels) {
      lines.push([
        `| ${lvl.level}`,
        lvl.throughputPerSec.toFixed(1),
        lvl.errors.toString(),
        fmtMs(lvl.p50E2E),
        fmtMs(lvl.p95E2E),
        fmtMs(lvl.p99E2E),
        fmtMs(lvl.maxE2E),
        lvl.p50Op === null ? "-" : fmtMs(lvl.p50Op),
        lvl.p95Op === null ? "-" : fmtMs(lvl.p95Op),
        lvl.p99Op === null ? "-" : fmtMs(lvl.p99Op),
        `${lvl.maxOp === null ? "-" : fmtMs(lvl.maxOp)} |`,
      ].join(" | "));
    }
    lines.push("");
  }
  lines.push("> 方法：worker 池 + 共享任务队列，每个 op 独立进程；并发度=worker 数。E2E 为父进程观测的完整进程 wall time，op 来自被测 JSON 的 elapsedMs。所有延迟单位为毫秒。报告不包含 transcript 内容。");
  lines.push("");
  return lines.join("\n");
}

// Executable resolution context mirrors the serial perf harness.
const ROOT = resolve(import.meta.dirname, "..");
const CLI_ENTRY = "";

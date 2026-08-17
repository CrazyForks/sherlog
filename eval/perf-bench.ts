#!/usr/bin/env -S node --import tsx

import { existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import { spawn as childSpawn } from "node:child_process";
import { join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { cleanupFixture, generateFixture, type FixturePaths } from "./perf-fixture";
import { PerfDataSourceError, resolvePerfWorkload } from "./perf-data-source";
import {
  DEFAULT_TOTAL_RUNS,
  commandArgv,
  findCommandArgv,
  parsePeakRssBytes,
  resolveCommandUnderTest,
  resourceSamplerCommand,
  timingStats,
  type CommandUnderTest,
  type TimingStats,
} from "./perf-bench-core";

interface LatencyStats {
  runs: number;
  samplesMs: number[];
  p50Ms: number;
  p95Ms: number;
  outputBytes: number;
  outputChars: number;
  /** Explicit process wall-clock latency; legacy fields above mirror this. */
  processE2E: TimingStats;
  /** Executable-reported `elapsedMs`, when the JSON contract provides it. */
  operation: (TimingStats & { source: "payload.elapsedMs" }) | null;
  /** Paired processE2E - operation samples; mostly wrapper/launcher overhead. */
  processOverhead: TimingStats | null;
  peakRssBytes: number | null;
  rssSampler: string | null;
}

interface TopHitRecord {
  sourceId: string;
  sessionRef: string;
  matchSource: string;
  matchSeq: number | null;
}

interface ReadProbeRecord extends LatencyStats {
  kind: "read-range" | "read-page";
  sourceId: string;
  sessionRef: string;
  argv: string[];
  messagesReturned: number;
  messageContentChars: number;
  anchorSeq?: number;
  totalCount?: number;
}

interface PerQueryRecord extends LatencyStats {
  query: string;
  resultCount: number;
  scannedMessageCount: number;
  topHit: TopHitRecord | null;
  readRange: ReadProbeRecord | null;
  readPage: ReadProbeRecord | null;
}

interface CoverageCostSummary {
  statusMs: number;
  statusProcessE2E: TimingStats;
  peakRssBytes: number | null;
  rssSampler: string | null;
  coverageCount: number;
  freshness: Record<string, number>;
  staleReasons: Record<string, number>;
  requestedCoverage: {
    freshness: string;
    staleReason: string;
    sourceFileCount: number;
    recommendedAction: string;
  } | null;
}

interface DbTableSizeRecord {
  name: string;
  bytes: number;
}

interface DbStorageSummary {
  dbSizeBytes: number;
  pageSize: number;
  pageCount: number;
  freelistCount: number;
  tableSizes: DbTableSizeRecord[];
}

interface DogfoodScoreboard {
  total: number;
  pass: number;
  fail: number;
  skip: number;
  hardFail: number;
  candidateFail: number;
  assertionPass: number;
  assertionFail: number;
  facetPass: number;
  facetFail: number;
}

interface DogfoodScorecardSummary {
  path: string;
  exitCode: number;
  stdoutBytes: number;
  stdoutChars: number;
  outDir: string | null;
  scorecard: string | null;
  scoreboard: DogfoodScoreboard | null;
  error: string | null;
}

interface Report {
  generatedAt: string;
  commandUnderTest: CommandUnderTest;
  collectRss: boolean;
  sourceId: string;
  dbPath: string;
  rootDir: string;
  sessionCount: number;
  messageCount: number;
  syncMs: number;
  syncMode: "run" | "skip";
  syncPeakRssBytes: number | null;
  syncRssSampler: string | null;
  dbSizeBytes: number;
  storage: DbStorageSummary;
  runsPerQuery: number;
  readRunsPerProbe: number;
  statusRuns: number;
  coverage: CoverageCostSummary;
  perQuery: PerQueryRecord[];
  dogfood: DogfoodScorecardSummary | null;
}

interface FindJsonPayload {
  scannedMessageCount?: number;
  results?: Array<{
    sourceId?: string;
    sessionRef?: string;
    matchSource?: string;
    matchSeq?: number | null;
  }>;
}

interface ReadJsonPayload {
  anchorSeq?: number;
  totalCount?: number;
  messages?: Array<{ contentText?: unknown }>;
}

interface StatsJsonPayload {
  sessionCount?: number;
  messageCount?: number;
  dbSizeBytes?: number;
}

interface StatusJsonPayload {
  coverage?: Array<{
    freshness?: string;
    sourceFileSetFingerprint?: string;
    currentSourceFileSetFingerprint?: string;
  }>;
  requestedCoverage?: {
    freshness?: string;
    staleReason?: string;
    sourceFileCount?: number;
    recommendedAction?: string;
  };
}

// Bench query 选取原则:
//  - 单 token 高频(hammerspoon/envchain): 检验最常见广义 fts 命中
//  - 短 token(sb): 检验 trigram fallback 路径
//  - 多 token 英文(fly deploy / edge tts): 检验多 term AND 路径
//  - CJK 短语(豆包输入法): 检验 CJK bigram 路径
//  - 中英混合(部署 health check): 检验 mixed match
const BENCH_QUERIES: string[] = [
  "hammerspoon",
  "envchain",
  "sb",
  "fly deploy",
  "edge tts",
  "豆包输入法",
  "部署 health check",
];

// 21 total invocations => one warmup + 20 measured samples. That is the
// minimum useful default for a real interpolated p95 instead of max-of-4.
const DEFAULT_RUNS_PER_QUERY = DEFAULT_TOTAL_RUNS;
const DEFAULT_READ_RUNS_PER_PROBE = DEFAULT_TOTAL_RUNS;
const DEFAULT_STATUS_RUNS = DEFAULT_TOTAL_RUNS;
const ROOT = resolve(import.meta.dirname, "..");
const CLI_ENTRY = "";
const OUT_BASE = resolve(ROOT, "data", "shlog-perf");

interface CliArgs {
  root: string;
  db: string;
  source: string;
  jsonOnly: boolean;
  runsPerQuery: number;
  readRunsPerProbe: number;
  statusRuns: number;
  dogfoodPath: string | null;
  bestEffortSync: boolean;
  skipSync: boolean;
  collectRss: boolean;
  commandUnderTest: CommandUnderTest;
  fixture: FixturePaths | null;
  keepFixture: boolean;
}

function parseArgs(argv: string[]): CliArgs {
  let root = "";
  let db = "";
  let source = "codex";
  let jsonOnly = false;
  let runsPerQuery = DEFAULT_RUNS_PER_QUERY;
  let readRunsPerProbe = DEFAULT_READ_RUNS_PER_PROBE;
  let statusRuns = DEFAULT_STATUS_RUNS;
  let dogfoodPath: string | null = null;
  let bestEffortSync = false;
  let skipSync = false;
  let collectRss = false;
  let executable: string | undefined;
  let cliArgvJson: string | undefined;
  let artifactPath: string | undefined;
  let explicitRoot = false;
  let explicitDb = false;
  let fixtureMb = 16;
  let fixtureMbExplicit = false;
  let keepFixture = false;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--root") {
      root = resolve(argv[++i] ?? root);
      explicitRoot = true;
    } else if (a === "--db") {
      db = resolve(argv[++i] ?? db);
      explicitDb = true;
    } else if (a === "--source") {
      source = argv[++i] ?? source;
    } else if (a === "--runs") {
      runsPerQuery = parsePositiveInt(argv[++i], DEFAULT_RUNS_PER_QUERY);
    } else if (a === "--read-runs") {
      readRunsPerProbe = parsePositiveInt(argv[++i], DEFAULT_READ_RUNS_PER_PROBE);
    } else if (a === "--status-runs") {
      statusRuns = parsePositiveInt(argv[++i], DEFAULT_STATUS_RUNS);
    } else if (a === "--dogfood") {
      dogfoodPath = resolve(argv[++i] ?? "");
    } else if (a === "--best-effort") {
      bestEffortSync = true;
    } else if (a === "--skip-sync") {
      skipSync = true;
    } else if (a === "--collect-rss") {
      collectRss = true;
    } else if (a === "--fixture-mb") {
      fixtureMb = parsePositiveInt(argv[++i], 16);
      fixtureMbExplicit = true;
    } else if (a === "--fixture") {
      // no-op alias: synthetic smoke is the default when both paths are omitted
    } else if (a === "--keep-fixture") {
      keepFixture = true;
    } else if (a === "--bin") {
      executable = argv[++i];
    } else if (a === "--cli-argv-json") {
      cliArgvJson = argv[++i];
    } else if (a === "--artifact") {
      artifactPath = argv[++i];
    } else if (a === "--json-only") {
      jsonOnly = true;
    } else if (a === "--help" || a === "-h") {
      console.log("Usage: npm run eval:perf -- [--source <id>] [--fixture-mb <n>] [--keep-fixture] | --root <dir> --db <path> [--skip-sync] [--runs <n>] [--read-runs <n>] [--status-runs <n>] [--bin <executable> | --cli-argv-json <json>] [--artifact <path>] [--collect-rss] [--dogfood <goldens.jsonl>] [--best-effort] [--json-only]");
      process.exit(0);
    }
  }

  let workload;
  try {
    workload = resolvePerfWorkload({ explicitRoot, explicitDb, fixtureMbExplicit });
  } catch (error) {
    if (error instanceof PerfDataSourceError) {
      console.error(`error: ${error.message}`);
      process.exit(1);
    }
    throw error;
  }

  // Default: isolated synthetic smoke. Real local data is opt-in only when
  // BOTH --root and --db are explicit — one flag must not revive the other
  // developer-machine default.
  let fixture: FixturePaths | null = null;
  if (workload.kind === "synthetic_smoke") {
    fixture = generateFixture(fixtureMb, source);
    root = fixture.root;
    db = fixture.db;
    skipSync = false; // fixture is fresh — must sync
  }

  const commandUnderTest = resolveCommandUnderTest({
    root: ROOT,
    cliEntry: CLI_ENTRY,
    executable,
    argvJson: cliArgvJson,
    artifactPath,
  });
  return {
    root,
    db,
    source,
    jsonOnly,
    runsPerQuery,
    readRunsPerProbe,
    statusRuns,
    dogfoodPath,
    bestEffortSync,
    skipSync,
    collectRss,
    commandUnderTest,
    fixture,
    keepFixture,
  };
}

function parsePositiveInt(value: string | undefined, fallback: number): number {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

const args = parseArgs(process.argv.slice(2));

if (!existsSync(args.root)) {
  console.error(`error: --root not found: ${args.root}`);
  process.exit(1);
}

interface RunResult {
  stdout: string;
  stderr: string;
  exitCode: number;
  ms: number;
  peakRssBytes: number | null;
  rssSampler: string | null;
}

async function run(cmd: string[], options: { collectRss?: boolean } = {}): Promise<RunResult> {
  const resourceProbe = options.collectRss ? resourceSamplerCommand(cmd) : null;
  const effectiveCommand = resourceProbe?.command ?? cmd;
  const t0 = performance.now();
  const result = await spawnAndCapture(effectiveCommand, ROOT);
  const ms = performance.now() - t0;
  return {
    ...result,
    ms,
    peakRssBytes: resourceProbe ? parsePeakRssBytes(result.stderr, resourceProbe.sampler) : null,
    rssSampler: resourceProbe?.sampler ?? null,
  };
}

function spawnAndCapture(cmd: string[], cwd: string): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  return new Promise((resolve, reject) => {
    const proc = childSpawn(cmd[0]!, cmd.slice(1), { cwd, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    proc.stdout!.setEncoding("utf8");
    proc.stderr!.setEncoding("utf8");
    proc.stdout!.on("data", (chunk: string) => { stdout += chunk; });
    proc.stderr!.on("data", (chunk: string) => { stderr += chunk; });
    proc.on("error", reject);
    proc.on("close", (code) => {
      resolve({ stdout, stderr, exitCode: code ?? 0 });
    });
  });
}

async function runOrThrow(cmd: string[], options: { collectRss?: boolean } = {}): Promise<RunResult> {
  const r = await run(cmd, options);
  if (r.exitCode !== 0) {
    throw new Error(`command failed (exit ${r.exitCode}): ${cmd.join(" ")}\n${r.stderr || r.stdout}`);
  }
  return r;
}

async function runJsonOrThrow<T>(
  cmd: string[],
  options: { collectRss?: boolean } = {},
): Promise<{ run: RunResult; payload: T }> {
  const result = await runOrThrow(cmd, options);
  try {
    return { run: result, payload: JSON.parse(result.stdout) as T };
  } catch (error) {
    throw new Error(`command did not emit JSON: ${cmd.join(" ")}\n${String(error)}\n${result.stdout}`);
  }
}

async function benchJsonCommand<T>(cmd: string[], runs: number): Promise<{ latency: LatencyStats; payload: T }> {
  const samplesAll: number[] = [];
  const operationSamples: Array<number | null> = [];
  let payload: T | null = null;
  let lastStdout = "";
  let peakRssBytes: number | null = null;
  let rssSampler: string | null = null;
  for (let i = 0; i < runs; i++) {
    // RSS sampling wraps only the warmup process so /usr/bin/time does not
    // distort measured latency samples. With a single run, the result is still
    // useful but the wrapper overhead is necessarily included.
    const result = await runJsonOrThrow<T>(cmd, { collectRss: args.collectRss && i === 0 });
    samplesAll.push(result.run.ms);
    operationSamples.push(reportedElapsedMs(result.payload));
    lastStdout = result.run.stdout;
    payload = result.payload;
    if (result.run.peakRssBytes !== null) peakRssBytes = result.run.peakRssBytes;
    if (result.run.rssSampler !== null) rssSampler = result.run.rssSampler;
  }
  if (!payload) throw new Error(`no payload produced for command: ${cmd.join(" ")}`);
  return {
    latency: latencyStats(samplesAll, lastStdout, operationSamples, peakRssBytes, rssSampler),
    payload,
  };
}

function latencyStats(
  samplesAll: number[],
  stdout = "",
  operationSamples: Array<number | null> = [],
  peakRssBytes: number | null = null,
  rssSampler: string | null = null,
): LatencyStats {
  const processE2E = timingStats(samplesAll);
  const hasCompleteOperationSamples = operationSamples.length === samplesAll.length
    && operationSamples.length > 0
    && operationSamples.every((sample): sample is number => sample !== null);
  const operationTiming = hasCompleteOperationSamples
    ? timingStats(operationSamples as number[])
    : null;
  const overheadTiming = hasCompleteOperationSamples
    ? timingStats(samplesAll.map((sample, index) => Math.max(0, sample - (operationSamples[index] as number))))
    : null;
  return {
    runs: processE2E.runs,
    samplesMs: processE2E.samplesMs,
    p50Ms: processE2E.p50Ms,
    p95Ms: processE2E.p95Ms,
    outputBytes: Buffer.byteLength(stdout, "utf8"),
    outputChars: stdout.length,
    processE2E,
    operation: operationTiming ? { ...operationTiming, source: "payload.elapsedMs" } : null,
    processOverhead: overheadTiming,
    peakRssBytes,
    rssSampler,
  };
}

function reportedElapsedMs(payload: unknown): number | null {
  if (typeof payload !== "object" || payload === null || !("elapsedMs" in payload)) return null;
  const elapsedMs = (payload as { elapsedMs?: unknown }).elapsedMs;
  return typeof elapsedMs === "number" && Number.isFinite(elapsedMs) && elapsedMs >= 0 ? elapsedMs : null;
}

function fmtMs(n: number): string {
  return n.toFixed(1).padStart(8);
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function formatOptionalMs(value: number | null | undefined): string {
  return typeof value === "number" ? fmtMs(value) : "-";
}

function formatOptionalBytes(value: number | null | undefined): string {
  return typeof value === "number" ? fmtBytes(value) : "unavailable";
}

function cliCommand(...command: string[]): string[] {
  return commandArgv(args.commandUnderTest, ...command);
}

function publicArgv(cmd: string[]): string[] {
  return ["shlog", ...cmd.slice(1 + args.commandUnderTest.prefixArgv.length)];
}

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const outDir = args.jsonOnly ? "" : join(OUT_BASE, stamp);
if (!args.jsonOnly) {
  mkdirSync(outDir, { recursive: true });
}

let sessionCount = 0;
let syncMs = 0;
let syncPeakRssBytes: number | null = null;
let syncRssSampler: string | null = null;

// 1. sync (default-compatible). --skip-sync benchmarks an already-built,
// explicit DB without mutating it; this is the recommended read-path mode.
if (args.skipSync) {
  if (!existsSync(args.db)) {
    console.error(`error: --skip-sync requires an existing --db: ${args.db}`);
    process.exit(1);
  }
} else {
  // Default to strict sync so coverage is actually written and the status
  // probe below measures the fresh path (the one agents hit in practice).
  const syncCmd = ["sync", "--source", args.source, "--db", args.db, "--root", args.root];
  if (args.bestEffortSync || args.fixture) syncCmd.push("--best-effort");
  const syncRun = await runOrThrow(cliCommand(...syncCmd, "--json"), { collectRss: args.collectRss });
  syncMs = syncRun.ms;
  syncPeakRssBytes = syncRun.peakRssBytes;
  syncRssSampler = syncRun.rssSampler;
  try {
    const parsed = JSON.parse(syncRun.stdout) as { scanned?: number };
    sessionCount = typeof parsed.scanned === "number" ? parsed.scanned : 0;
  } catch {
    // 解析失败保持 0
  }
}

// 2. coverage/freshness cost
const statusSelector = JSON.stringify({ source: args.source, kind: "all", root: args.root });
const statusCommand = cliCommand(
  "status",
  "--source",
  args.source,
  "--root",
  args.root,
  "--selector",
  statusSelector,
  "--db",
  args.db,
  "--json",
);
const statusResult = await benchJsonCommand<StatusJsonPayload>(statusCommand, args.statusRuns);
const coverage = coverageCostSummary(statusResult.latency, statusResult.payload);

// 3. find + raw-read probes
const perQuery: PerQueryRecord[] = [];
for (const q of BENCH_QUERIES) {
  const findCommand = findCommandArgv(args.commandUnderTest, q, args.source, args.db, args.root, 10);
  const { latency, payload } = await benchJsonCommand<FindJsonPayload>(findCommand, args.runsPerQuery);
  const topHit = topHitFromFind(payload);
  perQuery.push({
    query: q,
    ...latency,
    resultCount: Array.isArray(payload.results) ? payload.results.length : 0,
    scannedMessageCount: typeof payload.scannedMessageCount === "number" ? payload.scannedMessageCount : 0,
    topHit,
    readRange: topHit ? await measureReadRange(topHit, q) : null,
    readPage: topHit ? await measureReadPage(topHit) : null,
  });
}

// 4. stats -> db size and indexed counts
const statsRun = await runJsonOrThrow<StatsJsonPayload>(cliCommand("stats", "--source", args.source, "--db", args.db, "--json"));
let dbSizeBytes = 0;
let messageCount = 0;
if (typeof statsRun.payload.dbSizeBytes === "number") dbSizeBytes = statsRun.payload.dbSizeBytes;
if (typeof statsRun.payload.sessionCount === "number" && statsRun.payload.sessionCount > 0) {
  sessionCount = statsRun.payload.sessionCount;
}
if (typeof statsRun.payload.messageCount === "number") {
  messageCount = statsRun.payload.messageCount;
}
const storage = collectDbStorage(args.db, dbSizeBytes);
const dogfood = args.dogfoodPath
  ? await runDogfoodScorecard(args.dogfoodPath, args.commandUnderTest)
  : null;

const report: Report = {
  generatedAt: new Date().toISOString(),
  commandUnderTest: args.commandUnderTest,
  collectRss: args.collectRss,
  sourceId: args.source,
  dbPath: args.db,
  rootDir: args.root,
  sessionCount,
  messageCount,
  syncMs: Number(syncMs.toFixed(2)),
  syncMode: args.skipSync ? "skip" : "run",
  syncPeakRssBytes,
  syncRssSampler,
  dbSizeBytes,
  storage,
  runsPerQuery: args.runsPerQuery,
  readRunsPerProbe: args.readRunsPerProbe,
  statusRuns: args.statusRuns,
  coverage,
  perQuery,
  dogfood,
};

if (!args.jsonOnly) {
  writeFileSync(join(outDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  writeFileSync(join(outDir, "report.md"), buildMarkdown(report));
}

const readProbes = perQuery.flatMap((row) => [row.readRange, row.readPage].filter((probe): probe is ReadProbeRecord => probe !== null));
const slowest = [...perQuery].sort((a, b) => b.p95Ms - a.p95Ms)[0];
const slowestRead = [...readProbes].sort((a, b) => b.p95Ms - a.p95Ms)[0];
const summary = {
  outDir: outDir || null,
  sourceId: report.sourceId,
  commandUnderTest: report.commandUnderTest,
  collectRss: report.collectRss,
  sessionCount,
  messageCount,
  syncMs: report.syncMs,
  syncMode: report.syncMode,
  syncPeakRssBytes: report.syncPeakRssBytes,
  syncRssSampler: report.syncRssSampler,
  dbSizeBytes,
  tableSizeCount: storage.tableSizes.length,
  largestTables: storage.tableSizes.slice(0, 5),
  dogfood: dogfood ? {
    path: dogfood.path,
    exitCode: dogfood.exitCode,
    scoreboard: dogfood.scoreboard,
    outDir: dogfood.outDir,
    scorecard: dogfood.scorecard,
  } : null,
  coverage: report.coverage,
  queryCount: perQuery.length,
  readProbeCount: readProbes.length,
  slowestQuery: slowest ? {
    query: slowest.query,
    processE2EP95Ms: slowest.processE2E.p95Ms,
    operationP95Ms: slowest.operation?.p95Ms ?? null,
  } : null,
  slowestRead: slowestRead ? {
    kind: slowestRead.kind,
    processE2EP95Ms: slowestRead.processE2E.p95Ms,
    operationP95Ms: slowestRead.operation?.p95Ms ?? null,
  } : null,
};
console.log(JSON.stringify(args.jsonOnly ? report : summary, null, 2));

// Clean up synthetic fixture unless --keep-fixture was requested.
if (args.fixture && !args.keepFixture) {
  cleanupFixture(args.fixture);
}

function topHitFromFind(payload: FindJsonPayload): TopHitRecord | null {
  const first = payload.results?.[0];
  if (!first || typeof first.sourceId !== "string" || typeof first.sessionRef !== "string" || typeof first.matchSource !== "string") {
    return null;
  }
  const matchSeq = typeof first.matchSeq === "number" ? first.matchSeq : null;
  return {
    sourceId: first.sourceId,
    sessionRef: first.sessionRef,
    matchSource: first.matchSource,
    matchSeq,
  };
}

async function measureReadRange(hit: TopHitRecord, query: string): Promise<ReadProbeRecord> {
  const anchorArgs = hit.matchSeq === null ? ["--query", query] : ["--seq", String(hit.matchSeq)];
  const cmd = cliCommand(
    "read-range",
    hit.sessionRef,
    "--source",
    args.source,
    ...anchorArgs,
    "--before",
    "2",
    "--after",
    "2",
    "--db",
    args.db,
    "--json",
  );
  const { latency, payload } = await benchJsonCommand<ReadJsonPayload>(cmd, args.readRunsPerProbe);
  return {
    kind: "read-range",
    sourceId: hit.sourceId,
    sessionRef: hit.sessionRef,
    argv: publicArgv(cmd),
    messagesReturned: Array.isArray(payload.messages) ? payload.messages.length : 0,
    messageContentChars: messageContentChars(payload),
    anchorSeq: typeof payload.anchorSeq === "number" ? payload.anchorSeq : undefined,
    ...latency,
  };
}

async function measureReadPage(hit: TopHitRecord): Promise<ReadProbeRecord> {
  const cmd = cliCommand(
    "read-page",
    hit.sessionRef,
    "--source",
    args.source,
    "--offset",
    "0",
    "--limit",
    "40",
    "--db",
    args.db,
    "--json",
  );
  const { latency, payload } = await benchJsonCommand<ReadJsonPayload>(cmd, args.readRunsPerProbe);
  return {
    kind: "read-page",
    sourceId: hit.sourceId,
    sessionRef: hit.sessionRef,
    argv: publicArgv(cmd),
    messagesReturned: Array.isArray(payload.messages) ? payload.messages.length : 0,
    messageContentChars: messageContentChars(payload),
    totalCount: typeof payload.totalCount === "number" ? payload.totalCount : undefined,
    ...latency,
  };
}

function messageContentChars(payload: ReadJsonPayload): number {
  return (payload.messages ?? []).reduce((sum, message) => {
    return sum + (typeof message.contentText === "string" ? message.contentText.length : 0);
  }, 0);
}

function coverageCostSummary(latency: LatencyStats, payload: StatusJsonPayload): CoverageCostSummary {
  const coverageRows = Array.isArray(payload.coverage) ? payload.coverage : [];
  const freshness = countBy(coverageRows.map((row) => row.freshness ?? "unknown"));
  const staleReasons = countBy(coverageRows.map((row) => staleReasonForCoverage(row)));
  const requested = payload.requestedCoverage;
  return {
    statusMs: latency.processE2E.p50Ms,
    statusProcessE2E: latency.processE2E,
    peakRssBytes: latency.peakRssBytes,
    rssSampler: latency.rssSampler,
    coverageCount: coverageRows.length,
    freshness,
    staleReasons,
    requestedCoverage: requested ? {
      freshness: requested.freshness ?? "unknown",
      staleReason: requested.staleReason ?? "unknown",
      sourceFileCount: requested.sourceFileCount ?? 0,
      recommendedAction: requested.recommendedAction ?? "unknown",
    } : null,
  };
}

function staleReasonForCoverage(row: NonNullable<StatusJsonPayload["coverage"]>[number]): string {
  if (row.freshness !== "stale") return "none";
  return row.sourceFileSetFingerprint && row.currentSourceFileSetFingerprint === row.sourceFileSetFingerprint
    ? "source_content_changed"
    : "source_set_changed";
}

function countBy(values: string[]): Record<string, number> {
  return values.reduce<Record<string, number>>((acc, value) => {
    acc[value] = (acc[value] ?? 0) + 1;
    return acc;
  }, {});
}

function collectDbStorage(dbPath: string, fallbackDbSizeBytes: number): DbStorageSummary {
  const dbSizeBytes = safeFileSize(dbPath) ?? fallbackDbSizeBytes;
  const db = new DatabaseSync(dbPath, { readOnly: true });
  try {
    const pageSize = pragmaNumber(db, "page_size");
    const pageCount = pragmaNumber(db, "page_count");
    const freelistCount = pragmaNumber(db, "freelist_count");
    const tableSizes = db.prepare(`
      SELECT name, SUM(pgsize) AS bytes
      FROM dbstat
      GROUP BY name
      ORDER BY bytes DESC, name ASC
    `).all().map((row) => {
      const record = row as { name: unknown; bytes: unknown };
      return {
        name: String(record.name),
        bytes: Number(record.bytes) || 0,
      };
    });
    return { dbSizeBytes, pageSize, pageCount, freelistCount, tableSizes };
  } catch {
    return { dbSizeBytes, pageSize: 0, pageCount: 0, freelistCount: 0, tableSizes: [] };
  } finally {
    db.close();
  }
}

function pragmaNumber(db: DatabaseSync, name: string): number {
  const row = db.prepare(`PRAGMA ${name}`).get() as Record<string, unknown> | undefined;
  const value = row ? Object.values(row)[0] : undefined;
  return typeof value === "number" ? value : Number(value) || 0;
}

function safeFileSize(path: string): number | null {
  try {
    return statSync(path).size;
  } catch {
    return null;
  }
}

async function runDogfoodScorecard(
  path: string,
  commandUnderTest: CommandUnderTest,
): Promise<DogfoodScorecardSummary> {
  if (!existsSync(path)) {
    return {
      path,
      exitCode: 1,
      stdoutBytes: 0,
      stdoutChars: 0,
      outDir: null,
      scorecard: null,
      scoreboard: null,
      error: `dogfood file not found: ${path}`,
    };
  }

  const candidateArgv = JSON.stringify([
    commandUnderTest.executable,
    ...commandUnderTest.prefixArgv,
  ]);
  const result = await run([
    process.execPath,
    "--import",
    "tsx",
    resolve(ROOT, "eval", "run-dogfood-eval.ts"),
    path,
    "--cli-argv-json",
    candidateArgv,
  ]);
  const parsed = parseDogfoodStdout(result.stdout);
  return {
    path,
    exitCode: result.exitCode,
    stdoutBytes: Buffer.byteLength(result.stdout, "utf8"),
    stdoutChars: result.stdout.length,
    outDir: parsed?.outDir ?? null,
    scorecard: parsed?.scorecard ?? null,
    scoreboard: parsed?.scoreboard ?? null,
    error: parsed ? null : (result.stderr || "dogfood runner did not emit parseable summary"),
  };
}

function parseDogfoodStdout(stdout: string): { outDir?: string; scorecard?: string; scoreboard?: DogfoodScoreboard } | null {
  try {
    const parsed = JSON.parse(stdout) as {
      outDir?: unknown;
      scorecard?: unknown;
      scoreboard?: Partial<DogfoodScoreboard>;
    };
    const scoreboard = parsed.scoreboard;
    if (!scoreboard) return null;
    return {
      outDir: typeof parsed.outDir === "string" ? parsed.outDir : undefined,
      scorecard: typeof parsed.scorecard === "string" ? parsed.scorecard : undefined,
      scoreboard: {
        total: Number(scoreboard.total) || 0,
        pass: Number(scoreboard.pass) || 0,
        fail: Number(scoreboard.fail) || 0,
        skip: Number(scoreboard.skip) || 0,
        hardFail: Number(scoreboard.hardFail) || 0,
        candidateFail: Number(scoreboard.candidateFail) || 0,
        assertionPass: Number(scoreboard.assertionPass) || 0,
        assertionFail: Number(scoreboard.assertionFail) || 0,
        facetPass: Number(scoreboard.facetPass) || 0,
        facetFail: Number(scoreboard.facetFail) || 0,
      },
    };
  } catch {
    return null;
  }
}

function buildMarkdown(r: Report): string {
  const lines: string[] = [];
  lines.push("# shlog 性能基准报告");
  lines.push("");
  lines.push(`- generated_at: ${r.generatedAt}`);
  lines.push(`- command: \`${[r.commandUnderTest.executable, ...r.commandUnderTest.prefixArgv].join(" ")}\``);
  lines.push(`- command_source: ${r.commandUnderTest.source}`);
  lines.push(`- resolved_executable: \`${r.commandUnderTest.resolvedExecutablePath ?? "unresolved"}\``);
  lines.push(`- executable_size: ${formatOptionalBytes(r.commandUnderTest.executableSizeBytes)}`);
  lines.push(`- artifact: \`${r.commandUnderTest.artifactPath ?? "unresolved"}\``);
  lines.push(`- artifact_size: ${formatOptionalBytes(r.commandUnderTest.artifactSizeBytes)}`);
  lines.push(`- source: \`${r.sourceId}\``);
  lines.push(`- root: \`${r.rootDir}\``);
  lines.push(`- db: \`${r.dbPath}\``);
  lines.push(`- session_count: ${r.sessionCount}`);
  lines.push(`- message_count: ${r.messageCount}`);
  lines.push(`- sync_mode: ${r.syncMode}`);
  lines.push(`- sync_process_e2e_ms: ${r.syncMode === "run" ? r.syncMs.toFixed(1) : "skipped"}`);
  lines.push(`- sync_peak_rss: ${formatOptionalBytes(r.syncPeakRssBytes)}`);
  lines.push(`- db_size: ${fmtBytes(r.dbSizeBytes)} (${r.dbSizeBytes} bytes)`);
  lines.push(`- db_pages: page_size=${r.storage.pageSize}, page_count=${r.storage.pageCount}, freelist=${r.storage.freelistCount}`);
  lines.push(`- find_runs_per_query: ${r.runsPerQuery} (first run is warmup when runs > 1)`);
  lines.push(`- read_runs_per_probe: ${r.readRunsPerProbe} (first run is warmup when runs > 1)`);
  lines.push(`- status_runs: ${r.statusRuns} (first run is warmup when runs > 1)`);
  lines.push(`- collect_rss: ${r.collectRss}`);
  if (r.dogfood) {
    lines.push(`- dogfood: exit=${r.dogfood.exitCode}, scoreboard=\`${JSON.stringify(r.dogfood.scoreboard)}\``);
    if (r.dogfood.scorecard) lines.push(`- dogfood_scorecard: \`${r.dogfood.scorecard}\``);
  }
  lines.push("");
  lines.push("## coverage and freshness cost");
  lines.push("");
  lines.push(`- status_process_e2e_p50_ms: ${r.coverage.statusProcessE2E.p50Ms.toFixed(1)}`);
  lines.push(`- status_process_e2e_p95_ms: ${r.coverage.statusProcessE2E.p95Ms.toFixed(1)}`);
  lines.push(`- status_peak_rss: ${formatOptionalBytes(r.coverage.peakRssBytes)}`);
  lines.push(`- coverage_count: ${r.coverage.coverageCount}`);
  lines.push(`- freshness: \`${JSON.stringify(r.coverage.freshness)}\``);
  lines.push(`- stale_reasons: \`${JSON.stringify(r.coverage.staleReasons)}\``);
  if (r.coverage.requestedCoverage) {
    lines.push(`- requested_coverage: \`${JSON.stringify(r.coverage.requestedCoverage)}\``);
  }
  lines.push("");
  lines.push("## db table sizes");
  lines.push("");
  lines.push("| table | size | bytes |");
  lines.push("|-------|------:|------:|");
  for (const row of r.storage.tableSizes.slice(0, 20)) {
    lines.push(`| \`${row.name}\` | ${fmtBytes(row.bytes)} | ${row.bytes} |`);
  }
  lines.push("");
  lines.push("## per-query latency and raw-read probes");
  lines.push("");
  lines.push("| query | results | scanned msgs | output bytes | top hit | find process p50 | find process p95 | find operation p50 | find operation p95 | find peak RSS | read-range process p95 | read-range operation p95 | read-page process p95 | read-page operation p95 |");
  lines.push("|-------|--------:|-------------:|-------------:|---------|-----------------:|-----------------:|-------------------:|-------------------:|--------------:|-----------------------:|-------------------------:|----------------------:|------------------------:|");
  for (const row of r.perQuery) {
    lines.push([
      `| \`${row.query}\``,
      row.resultCount.toString(),
      row.scannedMessageCount.toString(),
      row.outputBytes.toString(),
      row.topHit ? `\`${row.topHit.sourceId}/${row.topHit.matchSource}\`` : "-",
      fmtMs(row.processE2E.p50Ms),
      fmtMs(row.processE2E.p95Ms),
      formatOptionalMs(row.operation?.p50Ms),
      formatOptionalMs(row.operation?.p95Ms),
      formatOptionalBytes(row.peakRssBytes),
      row.readRange ? fmtMs(row.readRange.processE2E.p95Ms) : "-",
      formatOptionalMs(row.readRange?.operation?.p95Ms),
      row.readPage ? fmtMs(row.readPage.processE2E.p95Ms) : "-",
      `${formatOptionalMs(row.readPage?.operation?.p95Ms)} |`,
    ].join(" | "));
  }
  lines.push("");
  lines.push("> process 指父进程观测的完整进程 wall time；operation 来自被测 executable JSON 的 elapsedMs（若提供），不是纯 SQLite 时间。p50/p95 使用去掉首轮 warmup 后的线性插值 percentile；默认保留 20 个测量样本。报告不包含 transcript 内容。");
  lines.push("");
  return lines.join("\n");
}

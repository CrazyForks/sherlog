#!/usr/bin/env -S node --import tsx

import { mkdirSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import { join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { cleanupFixture, generateFixture, type FixturePaths } from "./perf-fixture";
import {
  USAGE,
  HelpRequested,
  aggregateLevelStats,
  buildConcurrencyReportMarkdown,
  parseConcurrencyArgs,
  shapeCommand,
  type ConcurrencyArgs,
  type ConcurrencyReport,
  type OpSample,
  type ShapeLevelResult,
} from "./concurrency-bench-core";

const ROOT = resolve(import.meta.dirname, "..");
const OUT_BASE = resolve(ROOT, "data", "shlog-perf", "concurrency");

let args: ConcurrencyArgs;
try {
  args = parseConcurrencyArgs(process.argv.slice(2));
} catch (error) {
  if (error instanceof HelpRequested) {
    console.log(USAGE);
    process.exit(0);
  }
  console.error(`error: ${error instanceof Error ? error.message : String(error)}`);
  console.error(USAGE);
  process.exit(1);
}

// Synthetic smoke: generate an isolated fixture and sync it. Private
// calibration (--root and --db) is read-only against the existing index.
let fixture: FixturePaths | null = null;
if (args.workload === "synthetic_smoke") {
  fixture = generateFixture(args.fixtureMb, args.source);
  args.root = fixture.root;
  args.db = fixture.db;
  const syncCmd = [
    "sync", "--source", args.source, "--db", args.db, "--root", args.root,
    "--best-effort", "--json",
  ];
  const syncResult = spawnSync(args.commandUnderTest.executable, [...args.commandUnderTest.prefixArgv, ...syncCmd], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (syncResult.status !== 0) {
    console.error(`error: fixture sync failed (exit ${syncResult.status}): ${syncResult.stderr.toString().slice(0, 500)}`);
    cleanupFixture(fixture);
    process.exit(1);
  }
}

if (!args.db) {
  console.error("error: --db is required");
  process.exit(1);
}

async function runOnce(cmd: string[]): Promise<OpSample> {
  return new Promise((resolvePromise) => {
    const t0 = performance.now();
    const proc = spawn(args.commandUnderTest.executable, [...args.commandUnderTest.prefixArgv, ...cmd], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    proc.stdout!.setEncoding("utf8");
    proc.stderr!.setEncoding("utf8");
    proc.stdout!.on("data", (c: string) => { stdout += c; });
    proc.stderr!.on("data", (c: string) => { stderr += c; });
    proc.on("error", (err) => {
      resolvePromise({ ok: false, exitCode: null, e2eMs: performance.now() - t0, opMs: null, stdoutLen: 0, stderr: String(err) });
    });
    proc.on("close", (code) => {
      let opMs: number | null = null;
      try {
        const parsed = JSON.parse(stdout) as { elapsedMs?: unknown };
        if (typeof parsed.elapsedMs === "number") opMs = parsed.elapsedMs;
      } catch { /* non-JSON output */ }
      resolvePromise({ ok: code === 0, exitCode: code ?? 0, e2eMs: performance.now() - t0, opMs, stdoutLen: stdout.length, stderr });
    });
  });
}

async function runLevel(level: number, total: number, command: string[]): Promise<{ samples: OpSample[]; wallMs: number }> {
  let next = 0;
  const samples: OpSample[] = [];
  async function worker() {
    for (;;) {
      const i = next++;
      if (i >= total) return;
      samples.push(await runOnce(command));
    }
  }
  const t0 = performance.now();
  await Promise.all(Array.from({ length: level }, worker));
  const wallMs = performance.now() - t0;
  return { samples, wallMs };
}

function parseSessionRefFromList(stdout: string): string | null {
  try {
    const parsed = JSON.parse(stdout) as { sessions?: unknown; results?: unknown };
    const rows = (Array.isArray(parsed.sessions) ? parsed.sessions : [])
      .concat(Array.isArray(parsed.results) ? parsed.results : []);
    const first = rows[0] as { sessionRef?: unknown; sessionUuid?: unknown } | undefined;
    if (!first) return null;
    if (typeof first.sessionRef === "string") return first.sessionRef;
    if (typeof first.sessionUuid === "string") return first.sessionUuid;
    return null;
  } catch {
    return null;
  }
}

// Session ref resolution: pick the most recent session from the index so the
// read shapes have a real anchor. Best effort — a missing ref only skips the
// read shapes with a warning, it never fails the run.
async function resolveSessionRefCached(): Promise<string | null> {
  const cmd = ["list", "--source", args.source, "--db", args.db, "--limit", "1", "--json"];
  return new Promise((resolvePromise) => {
    const proc = spawn(args.commandUnderTest.executable, [...args.commandUnderTest.prefixArgv, ...cmd], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    proc.stdout!.setEncoding("utf8");
    proc.stdout!.on("data", (c: string) => { stdout += c; });
    proc.on("close", () => resolvePromise(parseSessionRefFromList(stdout)));
    proc.on("error", () => resolvePromise(null));
  });
}

// Session/message counts for report context.
async function collectIndexCounts(): Promise<{ sessionCount: number; messageCount: number }> {
  try {
    const cmd = ["stats", "--source", args.source, "--db", args.db, "--json"];
    const proc = await spawnCapture(cmd);
    const parsed = JSON.parse(proc.stdout) as { sessionCount?: unknown; messageCount?: unknown };
    return {
      sessionCount: typeof parsed.sessionCount === "number" ? parsed.sessionCount : 0,
      messageCount: typeof parsed.messageCount === "number" ? parsed.messageCount : 0,
    };
  } catch {
    return { sessionCount: 0, messageCount: 0 };
  }
}

function spawnCapture(cmd: string[]): Promise<{ stdout: string; exitCode: number }> {
  return new Promise((resolvePromise) => {
    const proc = spawn(args.commandUnderTest.executable, [...args.commandUnderTest.prefixArgv, ...cmd], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    proc.stdout!.setEncoding("utf8");
    proc.stdout!.on("data", (c: string) => { stdout += c; });
    proc.on("close", (code) => resolvePromise({ stdout, exitCode: code ?? 0 }));
    proc.on("error", () => resolvePromise({ stdout: "", exitCode: 1 }));
  });
}

const sessionRef = await resolveSessionRefCached();
const counts = await collectIndexCounts();
const ctx = { source: args.source, root: args.root, db: args.db, sessionRef };

const shapes: ShapeLevelResult[] = [];
for (const shape of args.shapes) {
  const command = shapeCommand(shape, ctx);
  if (command === null) {
    console.error(`warning: shape "${shape}" needs a resolvable session ref; skipping`);
    continue;
  }
  const levels: ShapeLevelResult["levels"] = [];
  for (const level of args.levels) {
    const { samples, wallMs } = await runLevel(level, args.totalPerLevel, command);
    levels.push(aggregateLevelStats(level, args.totalPerLevel, wallMs, samples));
    const last = levels[levels.length - 1]!;
    console.error(`[${shape}] level=${level}: ${last.throughputPerSec.toFixed(1)} ops/s  p50=${last.p50E2E.toFixed(1)}ms  p95=${last.p95E2E.toFixed(1)}ms  errors=${last.errors}`);
  }
  shapes.push({ shape, command, levels });
}

const report: ConcurrencyReport = {
  generatedAt: new Date().toISOString(),
  commandUnderTest: args.commandUnderTest,
  sourceId: args.source,
  dbPath: args.db,
  rootDir: args.root,
  sessionCount: counts.sessionCount,
  messageCount: counts.messageCount,
  totalPerLevel: args.totalPerLevel,
  shapes,
};

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const outDir = args.jsonOnly ? "" : join(OUT_BASE, stamp);
if (!args.jsonOnly) {
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  writeFileSync(join(outDir, "report.md"), buildConcurrencyReportMarkdown(report));
}

const slowestShape = [...report.shapes].sort((a, b) => (b.levels.at(-1)?.p95E2E ?? 0) - (a.levels.at(-1)?.p95E2E ?? 0))[0];
const summary = {
  outDir: outDir || null,
  sourceId: report.sourceId,
  commandUnderTest: report.commandUnderTest,
  sessionCount: report.sessionCount,
  messageCount: report.messageCount,
  totalPerLevel: report.totalPerLevel,
  shapeCount: report.shapes.length,
  slowestShape: slowestShape ? {
    shape: slowestShape.shape,
    maxConcurrencyP95E2EMs: slowestShape.levels.at(-1)?.p95E2E ?? null,
  } : null,
};
console.log(JSON.stringify(args.jsonOnly ? report : summary, null, 2));

// Clean up synthetic fixture unless --keep-fixture was requested.
if (fixture && !args.keepFixture) {
  cleanupFixture(fixture);
}

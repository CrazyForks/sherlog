import { accessSync, constants, existsSync, statSync } from "node:fs";
import { delimiter, isAbsolute, resolve } from "node:path";
import { resolveCliUnderTest, type CliUnderTestSource } from "./cli-under-test";

export const DEFAULT_TOTAL_RUNS = 21;

export interface CommandUnderTest {
  executable: string;
  prefixArgv: string[];
  source: CliUnderTestSource;
  resolvedExecutablePath: string | null;
  executableSizeBytes: number | null;
  artifactPath: string | null;
  artifactSizeBytes: number | null;
}

export interface TimingStats {
  runs: number;
  samplesMs: number[];
  warmupMs: number | null;
  p50Ms: number;
  p95Ms: number;
}

export interface CommandUnderTestOptions {
  root: string;
  cliEntry: string;
  env?: NodeJS.ProcessEnv;
  executable?: string;
  argvJson?: string;
  artifactPath?: string;
}

export function resolveCommandUnderTest(options: CommandUnderTestOptions): CommandUnderTest {
  const env = options.env ?? process.env;
  const cliExecutable = options.executable?.trim() || null;
  const argvJson = options.argvJson
    ?? (cliExecutable ? JSON.stringify([cliExecutable]) : undefined);
  const cli = resolveCliUnderTest({ argvJson, env });
  const [executable = process.execPath, ...prefixArgv] = cli.argv;
  const resolvedExecutablePath = resolveExecutablePath(executable, options.root, env.PATH);
  const explicitArtifact = options.artifactPath?.trim() || env.SHLOG_ARTIFACT_UNDER_TEST?.trim() || null;
  const artifactPath = explicitArtifact
    ? resolve(options.root, explicitArtifact)
    : resolvedExecutablePath;

  return {
    executable,
    prefixArgv,
    source: cli.source,
    resolvedExecutablePath,
    executableSizeBytes: safeFileSize(resolvedExecutablePath),
    artifactPath,
    artifactSizeBytes: safeFileSize(artifactPath),
  };
}

export function commandArgv(command: CommandUnderTest, ...argv: string[]): string[] {
  return [command.executable, ...command.prefixArgv, ...argv];
}

export function findCommandArgv(
  command: CommandUnderTest,
  query: string,
  source: string,
  db: string,
  root: string,
  limit = 10,
): string[] {
  return commandArgv(
    command,
    "find",
    query,
    "--source",
    source,
    "--root",
    root,
    "--db",
    db,
    "--limit",
    String(limit),
    "--json",
  );
}

export function timingStats(samplesAll: number[], discardWarmup = true): TimingStats {
  const warmupMs = discardWarmup && samplesAll.length > 1 ? samplesAll[0]! : null;
  const measured = discardWarmup && samplesAll.length > 1 ? samplesAll.slice(1) : [...samplesAll];
  const sorted = [...measured].sort((left, right) => left - right);
  return {
    runs: measured.length,
    samplesMs: measured.map(round2),
    warmupMs: warmupMs === null ? null : round2(warmupMs),
    p50Ms: round2(percentile(sorted, 0.5)),
    p95Ms: round2(percentile(sorted, 0.95)),
  };
}

/** R-7/linear percentile: unlike the old implementation, p95 is not max(). */
export function percentile(sortedSamples: number[], p: number): number {
  if (sortedSamples.length === 0) return 0;
  if (sortedSamples.length === 1) return sortedSamples[0]!;
  const bounded = Math.min(1, Math.max(0, p));
  const position = (sortedSamples.length - 1) * bounded;
  const lowerIndex = Math.floor(position);
  const upperIndex = Math.ceil(position);
  const lower = sortedSamples[lowerIndex]!;
  const upper = sortedSamples[upperIndex]!;
  return lower + (upper - lower) * (position - lowerIndex);
}

export function resourceSamplerCommand(
  command: string[],
  platform: NodeJS.Platform = process.platform,
): { command: string[]; sampler: string } | null {
  if (!existsSync("/usr/bin/time")) return null;
  if (platform === "darwin") {
    return { command: ["/usr/bin/time", "-l", ...command], sampler: "time-darwin" };
  }
  if (platform === "linux") {
    return { command: ["/usr/bin/time", "-v", ...command], sampler: "time-gnu" };
  }
  return null;
}

export function parsePeakRssBytes(stderr: string, sampler: string): number | null {
  if (sampler === "time-darwin") {
    const match = stderr.match(/^\s*(\d+)\s+maximum resident set size\s*$/m);
    return match ? Number(match[1]) : null;
  }
  if (sampler === "time-gnu") {
    const match = stderr.match(/^\s*Maximum resident set size \(kbytes\):\s*(\d+)\s*$/mi);
    return match ? Number(match[1]) * 1024 : null;
  }
  return null;
}

function resolveExecutablePath(executable: string, cwd: string, pathValue: string | undefined): string | null {
  if (isAbsolute(executable)) return isExecutableFile(executable) ? executable : null;
  if (executable.includes("/")) {
    const candidate = resolve(cwd, executable);
    return isExecutableFile(candidate) ? candidate : null;
  }
  for (const dir of (pathValue ?? "").split(delimiter)) {
    if (!dir) continue;
    const candidate = resolve(dir, executable);
    if (isExecutableFile(candidate)) return candidate;
  }
  return null;
}

function isExecutableFile(path: string): boolean {
  try {
    accessSync(path, constants.X_OK);
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function safeFileSize(path: string | null): number | null {
  if (!path) return null;
  try {
    const stat = statSync(path);
    return stat.isFile() ? stat.size : null;
  } catch {
    return null;
  }
}

function round2(value: number): number {
  return Number(value.toFixed(2));
}

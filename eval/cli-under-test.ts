import { existsSync } from "node:fs";
import { spawn as childSpawn } from "node:child_process";
import { resolve } from "node:path";

export const SHLOG_BIN_UNDER_TEST = "SHLOG_BIN_UNDER_TEST";
export const SHLOG_CLI_ARGV_JSON = "SHLOG_CLI_ARGV_JSON";

export type CliUnderTestSource = "argv-json" | "env-bin" | "native-release" | "native-debug";

export interface CliUnderTest {
  argv: string[];
  source: CliUnderTestSource;
}

export interface ResolveCliUnderTestOptions {
  /**
   * Explicit command prefix encoded as a JSON string array. This is the
   * unambiguous path for wrappers that need fixed arguments, for example:
   * `["cargo","run","--quiet","--manifest-path","...","--"]`.
   */
  argvJson?: string;
  env?: NodeJS.ProcessEnv;
  /** Checkout root used to locate target/{release,debug}/shlog. */
  repoRoot?: string;
}

export interface CliRunResult {
  exitCode: number;
  stdout: string;
  stderr: string;
}

export class NativeCliMissingError extends Error {
  constructor(repoRoot: string) {
    super(
      `no shlog binary under ${repoRoot}/target/{release,debug}/shlog; run cargo build --release --locked --bin shlog or set SHLOG_BIN_UNDER_TEST / SHLOG_CLI_ARGV_JSON`,
    );
    this.name = "NativeCliMissingError";
  }
}

const DEFAULT_REPO_ROOT = resolve(import.meta.dirname, "..");

/**
 * Resolve one executable command prefix for black-box evals.
 *
 * Precedence:
 * 1. explicit argv JSON,
 * 2. SHLOG_CLI_ARGV_JSON,
 * 3. SHLOG_BIN_UNDER_TEST (one path, never shell-split),
 * 4. checkout target/release/shlog, then target/debug/shlog.
 */
export function resolveCliUnderTest(options: ResolveCliUnderTestOptions = {}): CliUnderTest {
  const env = options.env ?? process.env;
  const argvJson = options.argvJson ?? env[SHLOG_CLI_ARGV_JSON];
  if (argvJson !== undefined) {
    return { argv: parseCliArgvJson(argvJson), source: "argv-json" };
  }

  const envBin = env[SHLOG_BIN_UNDER_TEST]?.trim();
  if (envBin) return { argv: [envBin], source: "env-bin" };

  const repoRoot = options.repoRoot ?? DEFAULT_REPO_ROOT;
  const release = resolve(repoRoot, "target", "release", "shlog");
  if (existsSync(release)) return { argv: [release], source: "native-release" };
  const debug = resolve(repoRoot, "target", "debug", "shlog");
  if (existsSync(debug)) return { argv: [debug], source: "native-debug" };
  throw new NativeCliMissingError(repoRoot);
}

export function parseCliArgvJson(value: string): string[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value) as unknown;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`CLI argv JSON must be a non-empty string array: ${message}`);
  }

  if (
    !Array.isArray(parsed)
    || parsed.length === 0
    || !parsed.every((item) => typeof item === "string" && item.length > 0)
  ) {
    throw new Error("CLI argv JSON must be a non-empty string array");
  }
  return [...parsed];
}

export function runCliUnderTest(
  cli: CliUnderTest,
  args: string[],
  options: { cwd?: string; env?: NodeJS.ProcessEnv } = {},
): Promise<CliRunResult> {
  const [executable, ...prefixArgs] = cli.argv;
  if (!executable) throw new Error("CLI under test has no executable");

  return new Promise((resolvePromise, reject) => {
    const proc = childSpawn(executable, [...prefixArgs, ...args], {
      cwd: options.cwd,
      env: options.env ?? process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    proc.stdout.setEncoding("utf8");
    proc.stderr.setEncoding("utf8");
    proc.stdout.on("data", (chunk: string) => { stdout += chunk; });
    proc.stderr.on("data", (chunk: string) => { stderr += chunk; });
    proc.on("error", reject);
    proc.on("close", (code) => {
      resolvePromise({ exitCode: code ?? 1, stdout, stderr });
    });
  });
}

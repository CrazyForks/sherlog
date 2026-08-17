/**
 * Shared workload selection for eval:perf and eval:perf:concurrency.
 *
 * Two modes only:
 *   - omit both --root and --db → synthetic smoke fixture (isolated, not representative)
 *   - pass both --root and --db → private calibration against an existing corpus
 *
 * Passing only one path used to fall back to the other real default
 * (~/.codex/sessions or the developer state DB). That is rejected.
 */

export type PerfWorkloadKind = "synthetic_smoke" | "private_calibration";

export class PerfDataSourceError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PerfDataSourceError";
  }
}

export const PERF_DATA_SOURCE_USAGE =
  "omit both --root and --db for the synthetic smoke fixture; pass both --root and --db for private read-only calibration";

export function resolvePerfWorkload(input: {
  explicitRoot: boolean;
  explicitDb: boolean;
  fixtureMbExplicit?: boolean;
}): { kind: PerfWorkloadKind } {
  const { explicitRoot, explicitDb, fixtureMbExplicit = false } = input;
  if (explicitRoot && explicitDb) {
    if (fixtureMbExplicit) {
      throw new PerfDataSourceError(
        `do not mix --fixture-mb with --root/--db; ${PERF_DATA_SOURCE_USAGE}`,
      );
    }
    return { kind: "private_calibration" };
  }
  if (!explicitRoot && !explicitDb) {
    return { kind: "synthetic_smoke" };
  }
  throw new PerfDataSourceError(
    `private calibration requires both --root and --db; ${PERF_DATA_SOURCE_USAGE}`,
  );
}

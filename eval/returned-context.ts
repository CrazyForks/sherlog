export interface ReturnedContextMetric {
  read: boolean;
  kind: "read-range" | "read-page" | null;
  chars: number;
  bytes: number;
}

export interface ReturnedContextSummary {
  reads: number;
  charsP50: number | null;
  charsP95: number | null;
  charsMax: number | null;
  bytesMax: number | null;
}

export function measureReturnedContext(input: {
  text?: string;
  kind?: "read-range" | "read-page";
  unavailableReason?: string;
}): ReturnedContextMetric {
  if (input.unavailableReason || input.text === undefined) {
    return { read: false, kind: input.kind ?? null, chars: 0, bytes: 0 };
  }
  return {
    read: true,
    kind: input.kind ?? null,
    chars: input.text.length,
    bytes: Buffer.byteLength(input.text, "utf8"),
  };
}

export function summarizeReturnedContext(metrics: ReturnedContextMetric[]): ReturnedContextSummary {
  const reads = metrics.filter((metric) => metric.read);
  const chars = reads.map((metric) => metric.chars).sort((left, right) => left - right);
  const bytes = reads.map((metric) => metric.bytes);
  return {
    reads: reads.length,
    charsP50: percentile(chars, 0.5),
    charsP95: percentile(chars, 0.95),
    charsMax: chars.length > 0 ? chars[chars.length - 1]! : null,
    bytesMax: bytes.length > 0 ? Math.max(...bytes) : null,
  };
}

function percentile(sorted: number[], fraction: number): number | null {
  if (sorted.length === 0) return null;
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * fraction) - 1));
  return sorted[index]!;
}

# Public JSON Shapes

以下是 standalone Rust CLI 的 agent-facing contract。字段使用 camelCase；示例省略不影响决策的长文本。不要从 SQLite 内部 row shape 推导 public JSON。

## Shared selector

```ts
type SourceId = "codex" | "claude-code" | "pi";

type Selector =
  | { kind: "all"; source: SourceId; root: string }
  | { kind: "cwd"; source: SourceId; root: string; cwd: string }
  | {
      kind: "date_range";
      source: SourceId;
      root: string;
      fromDate: string;
      toDate: string;
    }
  | {
      kind: "cwd_date_range";
      source: SourceId;
      root: string;
      cwd: string;
      fromDate: string;
      toDate: string;
    };
```

## Stored coverage

```ts
interface CoverageRecord {
  id: number;
  selector: Selector;
  sourceFingerprint: string;
  sourceFileSetFingerprint: string;
  sourceFileCount: number;
  indexedSessionCount: number;
  completedAt: string;
  indexVersion: string;
}

interface CoverageStatus {
  requested: Selector | null;
  complete: boolean;
  freshness: "fresh" | "stale" | "missing" | "not_checked";
  staleReason?: "none" | "missing" | "source_content_changed" | "source_set_changed";
  coveringSelectors: CoverageRecord[];
}
```

`find/list/read/stats` 的 coverage 只来自 SQLite stored proof。`find/list` 的 `CoverageStatus.freshness` 是 `not_checked`，即使 `complete=true` 也只表示存在 compatible covering record；`read/stats` 只返回 stored entries/rows。Live comparison 只在 `status` 中出现。Status 不返回/检索正文、不写 index，但 inventory cache miss 会流式读取 raw accepted projection；exact `mtime_ns`/checkpoint cache hit 不重 parse。

## `status --json`

```ts
interface StatusPayload {
  context: {
    cwd: string;
    root: string;
    dbPath: string;
    indexVersion: string;
  };
  sourceInventory: {
    root: string;
    totalFiles: number;
    pathDateRange: { from: string | null; to: string | null };
    cwdGroups: Array<{
      cwd: string;
      fileCount: number;
      pathDateRange: { from: string | null; to: string | null };
    }>;
  };
  index: {
    exists: boolean;
    sessionCount: number;
    messageCount: number;
    earliestStartedAt: string | null;
    latestEndedAt: string | null;
    dbSizeBytes: number;
    lastSyncAt: string | null;
  };
  coverageCount: number;
  coverage: CoverageInventoryStatus[];
  requestedCoverage?: RequestedCoverageStatus;
}

interface CoverageInventoryStatus extends CoverageRecord {
  freshness: "fresh" | "stale";
  staleReason: "none" | "source_content_changed" | "source_set_changed";
  advisory: boolean;
  currentSourceFingerprint: string;
  currentSourceFileSetFingerprint: string;
  currentSourceFileCount: number;
}

interface RequestedCoverageStatus {
  requested: Selector;
  complete: boolean;
  freshness: "fresh" | "stale" | "missing";
  staleReason: "none" | "missing" | "source_content_changed" | "source_set_changed";
  sourceFingerprint: string;
  sourceFileSetFingerprint: string;
  sourceFileCount: number;
  coveringSelectors: CoverageInventoryStatus[];
  recommendedAction: "query" | "sync";
}
```

默认 `coverage=[]` 且 `cwdGroups=[]`；`--inventory` 才展开。`requestedCoverage` 只在 `--cwd` / `--selector` 时出现。

## `sync --json`

```ts
interface SyncPayload {
  scanned: number;
  added: number;
  updated: number;
  skipped: number;
  filtered: number;
  removed: number;
  retainedCold: number;
  errors: number;
  errorDetails: Array<{ filePath: string; message: string }>;
  selector: Selector;
  coverage: {
    written: boolean;
    selector: Selector;
    sourceFingerprint: string;
    sourceFileSetFingerprint: string;
    sourceFileCount: number;
    indexedSessionCount: number;
    reason?: string;
    staleReason?: "source_content_changed";
    recommendedAction?: "query" | "sync";
  };
}
```

strict failure 的 JSON report 写 stderr 并 non-zero；`--best-effort` report 可写 stdout且包含 errors。`coverage.written=true` 才代表写入 coverage record；仍需观察可选 staleReason。

## `find --json`

```ts
interface FindPayload {
  query: string;
  sourceIds: SourceId[];
  sort: "relevance" | "ended" | "started";
  excludedSessions: string[];
  results: FindResult[];
  scannedMessageCount: number;
  coverage: CoverageStatus;
  coverageBySource?: Array<{ sourceId: SourceId; coverage: CoverageStatus }>;
  nextAction?: QueryNextAction;
  zeroResults?: {
    reason: "fresh_miss" | "stale_or_missing_coverage" | "coverage_not_confirmed";
    overConstrained: boolean;
    suggestedQueries: string[];
    hints: string[];
  };
  elapsedMs: number;
}

interface FindResult {
  rank: number;
  sourceId: SourceId;
  sessionUuid: string;
  sessionRef: string;
  title: string;
  summaryText: string;
  cwd: string;
  startedAt: string;
  endedAt: string;
  matchCount: number;
  matchSource: "message" | "session";
  matchSeq: number | null;
  matchRole: "user" | "assistant" | "session";
  matchTimestamp: string | null;
  score: number;
  snippet: string;
  matchedFields: Array<"message" | "title" | "summary" | "compact" | "reasoningSummary">;
  sessionMessageCount: number;
  evidenceRead: EvidenceRead;
}

type EvidenceRead =
  | {
      kind: "read-range";
      reason: "message_match" | "session_level_match";
      sourceId: SourceId;
      sessionRef: string;
      seq?: number;
      query?: string;
      before: number;
      after: number;
      argv: string[];
    }
  | {
      kind: "read-page";
      reason: "session_level_match";
      sourceId: SourceId;
      sessionRef: string;
      offset: number;
      limit: number;
      argv: string[];
    };
```

总是执行完整 `evidenceRead.argv`。`matchSource="session"` 时 `matchSeq=null`；不要构造虚假的 `--seq -1`。

当前 native `find` 不扫描 raw，因此正常 zero-result diagnosis 使用 `coverage_not_confirmed`。需要把同 selector 的 `status.requestedCoverage` live proof 与 find miss 组合起来；不要从 stored `complete` 推断 `fresh_miss`。Schema 保留其他 reason 供兼容/演进。

当前 public JSON 没有 candidate/filter/exact stage counts、`weakMatch` 或 `matchMode`；这些是后续 observability 工作，不能依赖。

## `read-range --json`

```ts
interface ReadRangePayload {
  session: SessionRecord;
  anchorSeq: number;
  rangeStartSeq: number;
  rangeEndSeq: number;
  messages: MessageRecord[];
  coverage: { entries: CoverageRecord[] };
  elapsedMs: number;
}
```

## `read-page --json`

```ts
interface ReadPagePayload {
  session: SessionRecord;
  offset: number;
  limit: number;
  totalCount: number;
  hasMore: boolean;
  messages: MessageRecord[];
  coverage: { entries: CoverageRecord[] };
  elapsedMs: number;
}
```

## Shared session/message

```ts
interface SessionRecord {
  id: number;
  sourceId: SourceId;
  nativeSessionId: string;
  sessionKey: string;
  sessionUuid: string;
  filePath: string;
  sourceRoot: string;
  title: string;
  summaryText: string;
  cwd: string;
  model: string;
  startedAt: string;
  endedAt: string;
  pathDate: string;
  messageCount: number;
}

interface MessageRecord {
  sessionUuid: string;
  seq: number;
  role: "user" | "assistant";
  contentText: string;
  timestamp: string;
  sourceKind: string;
  elision?: {
    originalCharCount: number;
    displayedCharCount: number;
    omittedCharCount: number;
    strategy: "head_tail" | "around_query";
    query?: string;
    hint: string;
  };
}
```

## `list --json`

```ts
interface ListPayload {
  query: {
    sourceId?: SourceId;
    cwd?: string;
    since?: string;
    selector?: Selector;
    sort: "ended" | "started" | "messages";
    limit: number;
  };
  results: Array<{
    sessionUuid: string;
    title: string;
    summaryText: string;
    cwd: string;
    startedAt: string;
    endedAt: string;
    pathDate: string;
    messageCount: number;
  }>;
  coverage: CoverageStatus;
  nextAction?: QueryNextAction;
}
```

`list` 当前 result 不带 `sessionRef`/`sourceId`；非 Codex 工作流若需要 source-qualified read，保持已知 `--source` 并构造对应 source ref，或优先用 `find` 获取 `sessionRef`。

## `stats --json`

```ts
interface StatsPayload {
  sessionCount: number;
  messageCount: number;
  earliestStartedAt: string | null;
  latestEndedAt: string | null;
  topCwds: Array<{ cwd: string; count: number }>;
  indexVersion: string;
  dbPath: string;
  dbSizeBytes: number;
  lastSyncAt: string | null;
  coverage: CoverageRecord[];
}
```

## `cold --json`

```ts
interface RegisteredColdRoot {
  sourceId: SourceId;
  root: string;
  addedAt: string;
}

// cold add
{ ok: true; configPath: string; entry: RegisteredColdRoot }

// cold list
{ configPath: string; roots: RegisteredColdRoot[] }

// cold remove
{ ok: true; removed: boolean; configPath: string; root: string; sourceId: SourceId }
```

在 v8 中 `configPath` 只标识 legacy tombstone compatibility path。registration truth 是 `roots` / SQLite `cold_roots`，不要读取/写入该路径当配置文件。

## Query next action

```ts
interface QueryNextAction {
  kind: "check_coverage_then_retry" | "choose_selector_then_check_coverage";
  reason:
    | "zero_results_with_unconfirmed_selector_coverage"
    | "zero_results_without_selector"
    | "stale_or_missing_coverage";
  selector?: Selector;
  steps: string[];
  commands?: Array<{
    label: string;
    recommended: boolean;
    argv: string[];
    selector?: Selector;
  }>;
}
```

## Error envelope

Typed business/index errors：

```ts
interface ErrorEnvelope {
  error: {
    code: string;
    message: string;
    [key: string]: unknown;
  };
}
```

常见扩展：

- `index_unavailable`：`dbPath`、`hint`、`nextAction.commands[].argv`；
- `index_schema_upgrade_required`：`dbPath`、`missingColumns`、`hint`；
- `session_not_found`：`sessionRef`、`sourceId`、`nativeSessionId`、`dbPath`、`nextAction`；
- `unsupported_source`：`source`；
- `invalid_selector` / `invalid_cold_root` / `index_error`：message。

CLI parse error 不保证 JSON envelope；例如缺少 `find` query 是 plain stderr compatibility text。

## SQLite storage truth

v8 internal object：`meta`、`session_rows`、read-only `sessions` view、`source_files`、`documents`、contentless `documents_fts`、`coverage`、`cold_roots`。高级 metadata SQL 只依赖 `sessions` view；不要查询 FTS content columns，也不要手写内部表。

## Source of truth

- `rust/src/model.rs`
- `rust/src/retrieval/evidence.rs`
- `rust/src/app/output.rs`
- `rust/src/error.rs`
- `rust/src/index/v8.sql`

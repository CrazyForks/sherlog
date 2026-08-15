# Source Adapter Contracts

This document defines the searchable projection contract for the native Rust source adapters. It is not a promise that upstream transcript formats are stable. `claude-code` and `pi` remain experimental transcript readers.

## Public seam

`rust/src/sources/mod.rs` exposes one narrow seam:

- `SourceCatalog::scan(selector, cache)` performs one traversal plus cache-aware inventory projection and returns accepted files, inventory, snapshot, and per-file scan failures. A cache miss may stream raw records/body; an exact metadata/checkpoint hit avoids re-parsing.
- projection converts one byte-bounded `SourceFile` into a privacy-reviewed `SessionProjection`, accepted message documents, `ReadProof`, and an opaque checkpoint.
- source-specific JSON keys, allowlists, reducer state, and raw-format drift remain private adapter implementation.

The registry is static: `codex`, `claude-code`, `pi`. Adding a runtime plugin is not part of this contract.

## Shared invariants

### Privacy projection

- Only allowlisted session metadata, user/assistant text, and explicitly accepted session-profile fields enter SQLite.
- Tool calls/results, attachments, diagnostics, thinking, sidechain/meta records, and unsupported content parts are rejected by default.
- Accepted message text becomes `documents.kind = "message"`; profile fields become one `session_profile` document during index write.
- Rejected/private records must not enter `documents`, `documents_fts`, snippets, summaries, inventory cwd grouping, or accepted metadata fingerprint.
- Status uses the same privacy decision for inventory proof: only accepted projection may influence cwd/time/session identity/fingerprint. It does not return or search body text, but a cache miss can still read raw bytes; exact `mtime_ns`/checkpoint cache hits avoid that parse.
- Broaden an allowlist only with source-specific positive and negative privacy fixtures.

### Identity and read isolation

- Every accepted session has `sourceId`, `nativeSessionId`, `sessionKey = sourceId:nativeSessionId`, and a compatibility `sessionUuid`.
- `find` returns a source-aware `sessionRef`; `read-*` consumes it without guessing the source.
- Same native id across sources cannot collide.

### Bounded JSONL read

- Content projection reads no bytes beyond the captured limit.
- Only fully newline-terminated records advance the append-safe cursor.
- Malformed/non-object JSONL records are skipped as format noise; an unterminated tail is not projected.
- `ReadProof` records opened/completed file stamps, bytes read, safe offset, content digest, and safe-prefix digest.
- Device/inode identity is used on Unix; other platforms retain deterministic path identity.

### Scan failure semantics

- Missing/non-directory root and traversal failure are fatal source errors.
- Per-file stat/path/accepted-metadata errors are returned in `SourceScan.failures` and included in snapshot proof.
- Strict sync publishes no partial complete coverage when failures exist.
- Best-effort may commit good files but must not report complete coverage.

### Incremental equivalence

- Checkpoint bytes are opaque outside the adapter.
- Delta is an optimization; full replay defines projection semantics.
- Any source/checkpoint/identity/prefix/cursor uncertainty returns `FullRequired` instead of guessing.
- For the same final raw bytes, delta + base must equal full replay.

## Codex adapter

Implementation: `rust/src/sources/codex.rs`.

Accepted metadata/profile input:

- `session_meta`: id and cwd when non-empty;
- `turn_context`: model and cwd when non-empty;
- `compacted`: non-empty message -> session `compactText`;
- `response_item` with payload type `reasoning`: non-empty summary text -> session `reasoningSummaryText`.

Accepted message input:

- `event_msg.payload.type = user_message` -> user message;
- `event_msg.payload.type = agent_message` -> assistant message;
- non-empty payload message and non-empty record timestamp;
- messages matching internal approval/evaluation markers are filtered.

Rejected:

- other record/payload types, including tool-like events;
- empty message/profile fields;
- malformed/non-object lines;
- internal marker messages.

Codex is the only adapter currently implementing true append delta. The checkpoint contains reducer state, next seq, indexed bytes, file identity, and prefix digest. Identity change, invalid reducer, cursor mismatch, prefix rewrite, or session identity change requires full replay.

## Claude Code adapter (experimental)

Implementation: `rust/src/sources/claude_code.rs`.

Accepted:

- top-level record type `user` or `assistant`;
- direct `content` string, or `message.content` string/array;
- only array items with `type = "text"`;
- accepted record timestamp/sessionId/cwd metadata.

Rejected:

- `isMeta = true`;
- `isSidechain = true`;
- non-user/assistant record types;
- tool/thinking/attachment-like non-text content parts;
- empty extracted text;
- malformed/non-object lines.

Claude Code currently requests full replay when an existing checkpoint is offered (`DeltaUnsupported`). Do not describe it as append-incremental yet.

## Pi adapter (experimental)

Implementation: `rust/src/sources/pi.rs`.

Accepted metadata/profile input:

- `session` record with non-empty cwd and timestamp; id when available;
- latest `model_change` model id;
- `compaction` record with non-empty summary -> session `compactText`.

Accepted message input:

- `message.message.role = user | assistant`;
- content string/array;
- array strings or objects with `type = "text"` only;
- timestamp from record or nested message.

Rejected:

- tool-result roles;
- tool-call/thinking/unsupported content parts;
- incomplete session/compaction/message records;
- malformed/non-object lines.

Pi currently requests full replay when an existing checkpoint is offered (`DeltaUnsupported`).

## Derived profile fields

When upstream data does not provide a stable generated profile, the adapter may derive:

- title from the first accepted user message;
- summary from bounded first/latest user/assistant accepted messages;
- start/end time from accepted records with deterministic fallback.

Derived values must use accepted projection only. A private/rejected append must not alter searchable metadata.

## Storage contract

Adapters never write SQLite directly. Sync maps adapter output to:

- `session_rows` / public `sessions` view;
- message and session-profile rows in `documents`;
- `documents_fts`;
- append proof in `source_files`;
- `coverage` only after complete snapshot proof.

This locality matters: source adapters decide what is safe to project; index decides how projection persists; retrieval decides how candidates rank.

## Current limits

- Synthetic/contract fixtures cover accepted and rejected shapes, not every upstream raw variant.
- Experimental source readers may change as upstream formats move.
- Cold-presence destructive prune currently has a reliable filename identity mapping only for Codex.
- No watcher/daemon consumes upstream changes automatically.
- No adapter may use raw grep to bypass projection privacy.

# Sherlog

[https://sherlog.net](https://sherlog.net)

`Sherlog` is a local-first CLI for searching local Codex, Claude Code, and Pi session logs. It is built for agents that know how to investigate: find the right session first, then read only the relevant range or page.

## Native Runtime Status

The production runtime is a standalone Rust CLI with bundled SQLite/FTS5. Node.js and the TypeScript implementation remain only as development contract/evaluation oracles; they are not part of the production runtime.

**v0.5.1 is the current native release.** Install the latest without Node.js:

```bash
curl -fsSL https://github.com/catoncat/sherlog/releases/latest/download/install.sh | sh
```

Or via Homebrew (macOS / Linux x64):

```bash
brew install https://github.com/catoncat/sherlog/releases/download/v0.5.1/sherlog.rb
```

To pin a published native version, replace `X.Y.Z` with that release number:

```bash
curl -fsSL https://github.com/catoncat/sherlog/releases/download/vX.Y.Z/install.sh \
  | SHERLOG_VERSION=X.Y.Z sh
```

The installer defaults to `$HOME/.local/bin`, verifies SHA-256, installs both `shlog` and the `sherlog` alias, and never invokes `sudo`. Set `SHERLOG_INSTALL_DIR` to choose another user-writable directory; replacing an existing command requires explicit `SHERLOG_FORCE=1`.

**Upgrading from 0.4.x:** run `shlog sync` once. The legacy index migrates to v8 automatically — a `.v7.bak` backup is kept, coverage is rebuilt for the synced scope, and old 0.4.4 writers fail closed. Content commands on a not-yet-migrated index return a typed `index_schema_upgrade_required` nextAction pointing at that sync; `status` reports `index.layout: legacy_v7`.

Install the optional agent skill separately. This uses the external `skills` package manager; it does not make the Sherlog CLI depend on Node.js:

```bash
npx skills add -g catoncat/sherlog
```

## Supported native targets

Native releases publish archives for macOS arm64, macOS x64, and Linux x64
GNU. Linux arm64, musl, and Windows archives are not declared yet.
Node.js >= 22.13 is required only when developing or running the TypeScript
differential oracle in this repository; it is not a user runtime dependency.

## Quick Start

Initialize the default Codex index:

```bash
shlog sync
```

Search and read progressively:

```bash
shlog find "health check"
shlog read-range <sessionRef> --seq <matchSeq>
shlog read-page <sessionRef> --offset 0 --limit 20
```

If `find` prints `next:` or JSON includes `nextAction`, refresh the suggested coverage and retry before treating the results as complete. Codex active-session tail drift is softer: `status` may report `freshness: "stale"` with `staleReason: "source_content_changed"` and `recommendedAction: "query"` when an existing JSONL is still growing; query/read first, and sync only when the latest tail or a strict completeness claim matters.

The same distinction applies during `sync`: a Codex JSONL that only appends after its bounded read no longer aborts the whole run. Stable sources and the bounded prefix are committed, the successful sync summary marks `coverage.staleReason: "source_content_changed"`, and the next sync fills the tail. Truncation, prefix rewrite/replacement, and source-set changes still fail strict sync.

If a new, unindexed Codex file already changed before its bounded read opened, Sherlog cannot prove that the old snapshot prefix was not rewritten. It conservatively defers that file and complete coverage, commits other stable sources, and returns `coverage.reason: "active_source_deferred"` with `recommendedAction: "sync"`.

For project-scoped agent work, check and refresh only that coverage:

```bash
shlog status --cwd /Users/you/work/project --json
shlog sync --cwd /Users/you/work/project
```

## Documentation

- [Design Philosophy](docs/PHILOSOPHY.md) - Why FTS? Why not `ripgrep` or embeddings?
- [Usage Guide](docs/USAGE.md) - Full commands, selectors, sync, and storage details.
- [Architecture](docs/ARCHITECTURE.md) - Retrieval model and how it works under the hood.
- [Roadmap](docs/ROADMAP.md) - What's coming next.
- [Agent Rules](AGENTS.md) - Project rules and contribution notes.

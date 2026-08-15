# Sherlog

[https://sherlog.net](https://sherlog.net)

`Sherlog` is a local-first CLI for searching local Codex, Claude Code, and Pi session logs. It is built for agents that know how to investigate: find the right session first, then read only the relevant range or page.

## Quick Install

Install the CLI globally:

```bash
npm i -g @act0r/sherlog
shlog --help
```

Install the agent skill separately:

```bash
npx skills add -g catoncat/sherlog
```

## Requirements

- **Node.js >= 22.13.0** — Sherlog uses Node's built-in SQLite (`node:sqlite`).
  There are **no native addons** to compile, download, or load, so the CLI can
  never hit a Node-ABI mismatch and works on macOS, Linux, and Windows alike.
- On Node 22.x, the runtime prints a one-line
  `ExperimentalWarning: SQLite is an experimental feature` to stderr once per
  process. It is harmless; silence it with
  `NODE_OPTIONS=--disable-warning=ExperimentalWarning` if your agent inspects
  stderr. Node 24+ prints nothing.
- Something looks off? Run `shlog doctor` — it reports the Node version, the
  built-in SQLite version, and whether the index database is readable.

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

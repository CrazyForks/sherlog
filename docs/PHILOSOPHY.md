# Why Sherlog? (Design Philosophy)

`Sherlog` searches local Codex, Claude Code, and Pi session history. It turns privacy-filtered conversation projections into queryable runbooks.

## Why not `ripgrep`?

`rg` dumps raw JSONL lines. Agents need conversational context, provenance, and a safe projection rather than unparsed records. Sherlog understands session structure: it locates a message or session-profile candidate, then exposes `read-range` and `read-page` to recover surrounding projected dialogue.

## Why not embedding/vector search?

Agents already provide semantic interpretation. A compact full-text candidate layer preserves the causal timeline—command, error, decision, fix—then lets the agent reason over an explicit evidence window. Heavier semantic retrieval can be evaluated later without becoming a second storage truth.

## Zero documentation tax

You do not need to stop and rewrite every solved problem as a clean note. Explicit `sync` derives an allowlisted searchable projection from prior sessions; tool results, thinking, attachments, and unsupported records do not enter that projection by default.

## A composable primitive

Sherlog is a short-lived CLI with stable JSON output. Use it to locate a session, pipe metadata through `jq`, and read projected context with `read-*`; it can also supply evidence to another tool such as Mainline.

Reading raw transcript data is not the normal search path. Only when `read-*` has already located the relevant session and the projection is clearly insufficient for a complete tool call, patch, long code block, or original event may an agent perform a narrowly scoped raw fallback. That fallback is agent-side evidence collection, must preserve source/privacy constraints, and must be identified separately from Sherlog projection evidence.

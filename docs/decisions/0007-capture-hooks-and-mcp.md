# 0007. Capturing changes: hooks plus MCP, joined by claims

- Status: proposed
- Date: 2026-11-24

## Context

There are two ways to learn about an AI change, and neither is enough alone:

- **The agent tells us**, through an MCP tool (`record_decision`). This gives us reasoning,
  alternatives and the task, but agents forget, and they can be wrong about which lines they
  changed.
- **The editor tells us**, through a Claude Code PostToolUse hook on edit and write tools.
  It always fires, with exact hunks, but carries no reasoning.

We want the reasoning when the agent provides it, and we never want a change to go
unrecorded when it doesn't.

## Decision

### Hook: `concede capture`

A PostToolUse hook matching Claude Code's file-editing tools runs `concede capture`, which
reads the hook's JSON from stdin and writes `.concede/captures/<ULID>.json`:

```jsonc
{ "schema": 1, "id": "01JDB6K8…", "timestamp": "…",
  "agent": { "tool": "claude-code", "session": "5b1e…" },
  "tool_name": "Edit",
  "base_commit": "3f9c2a1…",
  "change": { "path": "src/OrderService.cs", "after_blob": "…", "hunks": [ … ] } }
```

- **Hunks come from the hook payload** where Claude Code includes the applied patch, and
  otherwise from `ChangeStore::changed_hunks` for that file. The exact payload format will be
  checked against a real hook run in a sample repo before this is implemented, not assumed.
- **Captures are immutable**, like decisions (ADR 0005).
- **The hook must never block or break the agent.** It finishes quickly, exits 0 even on
  errors, and writes failures to `.concede/capture-errors.log` (git-ignored).
- Files outside the repository are ignored.

### MCP: `concede mcp`

`concede mcp` runs an MCP server over stdio (official Rust SDK, `rmcp`) with one tool,
`record_decision`. Its inputs are the agent-facing fields of the decision record: `task`,
`reasoning`, `alternatives`, `allowed_by`, `files` (paths, with optional line ranges), and
optionally `model`. The server fills in the id, timestamp, `base_commit` and blobs itself.

### Claims: joining the two

When a decision is recorded, it **claims** every unclaimed capture for the same files:

- If the session id is known on both sides, only captures from that session.
- Otherwise, captures for those files since the previous decision that touched them.

Claimed capture ids are stored in the decision's `captures` field. The decision's hunks are
taken from those captures, unless the agent provided explicit ranges and no capture exists.
A capture counts as claimed if any decision lists it, which is computed on read, so no file
is ever edited.

### Inbox

`concede inbox` shows two lists:

1. **Unreviewed decisions**, grouped by agent session.
2. **Unexplained changes**: captures no decision claims. They can be reviewed directly
   (`concede review` accepts capture ids too, wrapping them in a `source: hook` decision), or
   explained after the fact with `concede log --claim <capture-id>`.

### Examples shipped in the repo

`examples/claude-code/` gets a `.claude/settings.json` snippet (the PostToolUse hook) and a
`.mcp.json` snippet (the MCP server), plus a short instruction for the agent's CLAUDE.md
asking it to call `record_decision` after each logical change. `docs/capture.md` explains the
setup.

## Alternatives rejected

- **MCP only.** Agents forget to call tools, so changes would go missing silently.
- **Hooks only.** Exact and reliable, but every record would lack the "why", which is the
  point of the tool.
- **Hook turns each capture into a decision immediately.** Floods the inbox with one record
  per edit, and the agent's later explanation would have to edit or duplicate them.
- **Ask the model for reasoning from inside the hook** (calling an LLM per edit). Slow,
  costly, adds network access to every edit, and reconstructs the reasoning after the fact
  instead of capturing it from the agent that made the change.
- **Parse the session transcript afterwards.** Depends on an undocumented file format that
  can change, and runs after the fact.

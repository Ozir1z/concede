# Capturing decisions from Claude Code

Concede records two things while an agent works (ADR 0007):

- **Captures**: a PostToolUse hook runs `concede capture` after every file edit. It
  records exactly which lines changed, but not why.
- **Decisions**: the agent calls the `record_decision` MCP tool with the task, its
  reasoning and the alternatives it rejected. The decision claims the captures from the
  same session, so the reasoning is attached to the exact lines.

Captures that no decision claims show up in `concede inbox` as **unexplained changes**,
so nothing goes unrecorded when an agent forgets to log.

## Setup

1. Install the CLI so `concede` is on your PATH:

   ```sh
   cargo install --path crates/concede-cli
   ```

2. Copy the example configuration from [`examples/claude-code/`](../examples/claude-code)
   into the root of the repository you want to track:

   | File | What it does |
   |---|---|
   | `.claude/settings.json` | The PostToolUse hook, enables the `concede` MCP server, and allows its two tools without a prompt. Merge it into an existing `settings.json`. |
   | `.mcp.json` | Registers `concede mcp` as a project MCP server. |
   | `CLAUDE.md` | Tells the agent to call `record_decision` after each change and `why` before changing code it didn't write. Append it to an existing `CLAUDE.md`. |

3. Start `claude` interactively in that repository once and accept the trust prompt.
   Claude Code only applies a project's permission rules after you trust the folder,
   so until then it asks before each Concede tool call (and denies them in `claude -p`).
   For a non-interactive run in an untrusted folder, pass
   `--allowedTools mcp__concede__record_decision mcp__concede__why`.

4. Commit `.concede/` along with your code. It holds one small JSON file per decision,
   review and capture. `concede capture` writes problems to
   `.concede/capture-errors.log`, which is git-ignored.

## Daily use

```sh
concede inbox                       # unreviewed decisions by session, then unexplained changes
concede why src/Order.cs:42         # which decisions shaped this line, and why
concede review 01M87SHQB8DV --approve
concede review 01M87SGCP4JT --reject --note "not asked for"
git commit -am "..."
concede link                        # attach approved decisions to the commit
```

## How sessions are matched

Claude Code passes the session id to hooks in their JSON payload (`session_id`) and to
MCP servers as the `CLAUDE_CODE_SESSION_ID` environment variable. Both were checked
against a real session and match, so a decision claims only its own session's edits,
even when several agents work in the same repository. If the session id is missing, a
decision claims edits to its files made since the previous decision on those files.

## What the hook payload looks like

The parser was written against real payloads (sanitised copies in
`crates/concede-core/tests/fixtures/claude-code/`):

- `tool_input.file_path` is the absolute path of the edited file.
- `tool_response.structuredPatch` is the applied patch *with context lines*. Concede
  derives the exact changed lines from its `-`/`+` markers.
- A `Write` that creates a file sends an empty `structuredPatch`; Concede then diffs the
  file with git instead.
- The payload has no model name; agents pass theirs to `record_decision`.

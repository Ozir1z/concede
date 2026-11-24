# Claude Code kickoff prompt

The original prompt used to kick off the project with Claude Code. Kept for history; VISION.md and the ADRs in `docs/decisions/` are authoritative.

---

You're helping me build Concede, a free, open-source, native IDE for AI-augmented coding. Read `docs/VISION.md` before doing anything; it is the source of truth for goals, interaction model, pillars, AI rules and architecture.

## Context

- I'm a senior .NET developer, newer to Rust. Explain non-obvious Rust choices briefly (ownership, lifetimes, async) in commit messages or ADRs, not in long chat replies.
- Everything is Rust. UI is GPUI (Zed's UI framework). Build on GPUI only; do not copy code from the Zed editor (GPL-3.0).
- Snappy is non-negotiable. No blocking work on the UI thread; heavy work runs in background tasks.
- Language intelligence comes from LSP (Roslyn LSP for C# first) and tree-sitter. Don't write custom parsers.
- All logic lives in `concede-core`. The CLI and the app are thin layers on top.
- I have older code in a previous project (foa-torch). Ignore it unless I point you to it.

## Working rules

- Show me a plan and wait for my OK before writing code for each phase.
- Small, reviewable commits with clear messages. Never push, merge or rebase.
- Don't read or write outside this repo.
- Prefer well-known crates. Ask before adding a dependency.
- Write tests for core logic as you go (`cargo test`). Run `cargo clippy` and `cargo fmt` before each commit.
- When you make a design decision, record it as a short ADR in `docs/decisions/` (context, decision, alternatives rejected). We're building a decision-traceability tool, so we follow the practice ourselves.

## Phase 0: Scaffold

- Cargo workspace with `crates/concede-core`, `crates/concede-cli`, `crates/concede-app`, plus `/docs`.
- `concede-app`: minimal GPUI window that opens and shows "Concede". Check the current way to depend on GPUI (crates.io vs git) and record the choice in an ADR. Look at `gpui-component` as a possible component library and tell me if it's worth using.
- Apache-2.0 license, README (summarize VISION.md in under 20 lines), `.editorconfig`, `.gitignore`, `rustfmt.toml`.
- A `CLAUDE.md` with build/test/run commands and the working rules above.
- GitHub Actions workflow: build, test, clippy, fmt check on Linux, macOS and Windows.

Done when: `cargo test` passes, `cargo run -p concede-app` opens a window, and CI is green.

## Phase 1: Decision log (the first real feature)

Goal: every AI-made change leaves a "why" record in the repo, queryable per file and per line range.

1. **Schema**: one JSON file per decision under `.concede/decisions/`, append-only. Fields at minimum: id, timestamp, agent/model, prompt or task summary, reasoning, files and hunk-level line ranges touched, alternatives considered, rule that allowed the action, review status (unreviewed / approved / rejected), linked commit (filled in later). Propose the schema as an ADR first. Hunk-level detail matters: later, a story must map to exactly the hunks it produced.
2. **Core API** in `concede-core`: record a decision, list decisions for a file, list decisions for a line range, mark reviewed, link to commit. All git access goes through a `ChangeStore` trait. Implement it with plain git for now (shelling out to the git CLI or using `gix`; propose one in an ADR). Only add the methods this phase needs, like current HEAD and changed hunks.
3. **CLI** (`concede-cli`): `concede log` (record), `concede why <file>[:line]` (history), `concede inbox` (unreviewed decisions), `concede link` (attach to HEAD commit).
4. **Capture**:
   - An MCP server in `concede-cli` exposing a `record_decision` tool, so agents log their reasoning as they work.
   - A Claude Code hook config example (PostToolUse on edit/write tools) that records file changes automatically, so nothing is missed even if the agent forgets to log.
5. **Tests** for schema validation, querying by line range, and the review status flow.

Done when: I can run a Claude Code session in a sample repo, then run `concede why src/SomeFile.cs` and see which decisions touched it, with reasoning, and `concede inbox` shows what I haven't reviewed.

## Phase 2: Visual mode shell

- `concede-app` opens a repo and shows the story inbox (unreviewed decisions from Phase 1), grouped by agent session.
- Keyboard-first: default keybinds plus a vim-mode setting. Default: arrows move, Enter opens, `A`/`R` approve/reject. Vim mode: `hjkl`, `a`, `r`, `/`, `?`. Keybinds live in a config file.
- Selecting a decision shows its reasoning and the touched code read-only, with the changed lines highlighted.
- One key switches to code mode on the selected hunk (a basic editable text view is fine for now); Esc returns to the exact same spot in visual mode.
- Measure and log frame times; anything over 16 ms on the UI thread is a bug.

Done when: I can review and approve a day of agent work without leaving the keyboard, and it feels instant.

## Later (don't build yet, but don't design against them)

- Code Map: tree-sitter + LSP graph with change heat and semantic zoom into code.
- Criticality engine (VISION.md section), starting with declared zones and LSP/tree-sitter code facts.
- Change Stories view grouping changes by intent and ordering by dependency.
- Version control layer: commit or branch a whole story at once, show all active branches and worktrees in one view, isolated worktrees for critical stories, Jujutsu as an alternative `ChangeStore` engine.
- AI Rules config file (`.concede/rules`) enforced via hooks.
- Full code mode: LSP (Roslyn), DAP debugging (netcoredbg), syntax via tree-sitter.
- Project Wizard.
- "Open in VS Code / Rider" escape hatch.

Start by reading `docs/VISION.md`, then give me your plan for Phase 0.

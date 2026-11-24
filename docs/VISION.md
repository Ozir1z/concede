# Torch: Vision (working name)

Whiteboard: https://claude.ai/artifact/FzVe3gGvpkRHq7TwwTPFrR (private to the owner; this file is the in-repo copy)

## Problem

AI now writes most code, but developers lose agency. Agents produce more code than anyone wants to review, diffs are walls of text, and the reasoning behind each change disappears when the session ends. AI tools sit beside the IDE and take it over instead of being part of it.

## Goal

A free, open-source, native IDE for a new era of writing code. Not another VS Code clone. The human mostly steers, reviews and decides, and types sometimes, so the IDE is visual first and drops into code on a keypress. Snappy is non-negotiable: Rust, GPU-rendered UI.

## Principles

- Open source, local-first, bring your own model(s).
- Fast: every interaction should feel instant.
- Deterministic systems first, LLM judgment second.
- Every automated decision is logged and needs human sign-off. Critical ones block the commit.
- Decision history lives in the repo, next to the code.
- Don't chase feature parity with VS Code. Build a great navigator and reviewer plus a good-enough editor.

## Interaction model

- **Visual mode (home):** Code Map, story inbox, decision timelines. Fully keyboard-driven.
- **Code mode:** one key on any node zooms into the code for that symbol (not just the file). Esc zooms back to the same spot on the map. The transition is a real zoom, so the user never loses their place.
- **Code in context:** while reviewing a story, code is shown inline in visual mode with criticality and history overlaid. Switch to code mode only to type.
- **Keybinds:** sensible default keybinds (like Zed) plus an optional vim mode. Example vim-style bindings in visual mode: `hjkl` move between nodes, `a` approve, `r` reject, `/` search, `?` show why.
- **Escape hatch:** "open in VS Code / Rider" on any file, so people can adopt Torch as a command center before switching editors.

## Pillars

1. **Project Wizard**: pick the project type (web app, mobile app, API, class library, game). Sets up a modular structure and writes a project spec (modules, critical zones) that the other pillars read. Can be re-run as a modularity check.
2. **Code Map**: structure first, files second. Changed areas glow, critical zones are marked. File view stays available on demand.
3. **Change Stories**: interactive change overview. AI output grouped by intent, ordered by dependency, boilerplate collapsed, critical code highlighted. Accept, edit or reject per group.
4. **History in Place**: hover or select any symbol to see its decision timeline: prompt, reasoning, rejected options, commits. Captured at write time, not reconstructed.
5. **AI Rules**: choose one or more models and give each a role (author, reviewer, test writer). Authority set per action and per zone. Rules stored in the repo.
6. **One Surface**: visual mode by default, code mode on a key. Hand edits and AI suggestions in the same place; AI treats your edits as context.

## Default AI rules (all editable)

| Action | Default | Who |
|---|---|---|
| Write code, low-risk zones | Allowed | Author model |
| Write code, critical zones | Suggest only | Author model, human approves |
| Write tests | Allowed | Test model |
| Review critical code | Required | A second, different model |
| Git commit | Ask | Any |
| Git push, merge, rebase | Never | Human only |
| Run terminal commands | Ask, or allowlist | Any |
| Install packages, network | Ask | Any |
| Read outside project folder | Never | Human can grant a path |
| Write outside project folder | Never | Human only |

Every AI action is logged against the rule that allowed it.

## Criticality engine

Criticality is computed from layered signals, not by an LLM re-scanning code:

1. **Declared**: wizard spec, zone rules, user tags (e.g. a `[Critical]` attribute or comment marker).
2. **Code facts**: static analysis via tree-sitter and the language server (Roslyn LSP for C#): touches auth, money, data writes, public API.
3. **Blast radius**: number of dependents.
4. **History**: churn, bug-fix commits, reverts.
5. **AI tag**: set once by the model when it writes the code; only recomputed when that code changes.

Output: a score plus a human-readable reason, e.g. "Critical: writes the Orders table, 14 dependents".

## Version control layer

Build on top of git, don't replace it: GitHub, CI and review tools all speak git. AI works on several features at once, so the human-era model of one branch at a time plus tracked/untracked files doesn't fit.

Flow: agent sessions → decision log (knows which intent produced each change) → Change Stories (changes grouped by intent, all active branches and worktrees in one view) → review and approve (critical stories need sign-off) → clean commit, branch or PR.

- The decision log is the differentiator: existing tools (GitButler virtual branches) have to guess or ask which hunk belongs to which feature. We already know.
- Version control sits behind a `ChangeStore` trait in core so the engine can be swapped.
- Engine v1: plain git, worktrees and hunk-level staging. Engine later: Jujutsu (jj, Apache-2.0, Rust, no staging area, conflicts as data, operation-log undo).
- Shared working folder (one build) by default; isolated worktree when a story touches critical code so its tests run separately.
- Prior art to study, not depend on: git worktrees, GitButler (check its license), Jujutsu, stacked-diff tools, Whiteboard (/dev/fast), Bitloops.

## Architecture

Rust workspace. Language intelligence comes from standard protocols, not custom parsers.

```
/crates/torch-core   decision log, rules, criticality, ChangeStore (git)
/crates/torch-cli    CLI + MCP server for agents
/crates/torch-app    native GPUI app (visual mode, code mode)
/docs                vision and decision records
```

- UI: GPUI (Zed's GPU-accelerated UI framework, Apache-2.0). Do not fork the Zed editor itself (GPL-3.0); build on GPUI only.
- Language intelligence: LSP (Roslyn language server for C#), tree-sitter for syntax and structure.
- Debugging: DAP (netcoredbg for .NET).
- Core is a library used by both the CLI and the app; an optional JSON-RPC interface lets other clients connect later.

## Prototype path

1. Decision log: agent writes the why for each change into the repo.
2. Change stories: group a real AI diff by intent and review it.
3. Code map: tree-sitter + LSP graph of a real repo with change heat.
4. Native shell: Rust + GPUI with visual mode and one-key zoom into code.
5. Full IDE: open source, snappy, default or vim keybinds.

## Open questions

- Final name (working: Torch).
- First language: C#/.NET via Roslyn LSP (assumed).

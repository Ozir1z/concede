# Status

Phase tracking for Concede. The roadmap lives in `docs/KICKOFF.md`; this file
records what is actually done and how the "done when" criteria were met.

## Phase 0 — Scaffold ✅

Cargo workspace (`concede-core`, `concede-cli`, `concede-app`), a GPUI window that
opens and shows "Concede", Apache-2.0 + README + editor configs, and CI (build,
test, clippy, fmt) green on Linux, macOS and Windows.

## Phase 1 — Decision log ✅

Every AI-made change leaves a queryable "why" record in `.concede/`.

- ADRs 0005–0007 (schema/storage, `ChangeStore` via git CLI, capture via hooks + MCP).
- `concede-core`: records + validation, immutable `Store`, `Log` queries, line-drift
  mapping, and `Concede` (`record_capture`, `record_decision`, `review`, `link`, `why`).
- `concede` CLI: `log`, `why <file>[:lines]`, `inbox`, `review`, `link`, `capture`.
- `concede mcp`: MCP server (rmcp) exposing `record_decision` and `why`.
- Example Claude Code wiring in `examples/claude-code/` and `docs/capture.md`.
- A C# sample (`examples/sample-csharp/`) and setup scripts (`examples/try-it.sh`,
  `examples/try-it.ps1`) for trying the whole loop in a scratch repo.

**Done-when, verified on macOS (2026-11-24):** ran a Claude Code session (Haiku) in a
scratch copy of the sample, then `concede inbox` listed the unreviewed decision
(grouped by session) and `concede why src/Order.cs` showed the reasoning, the
rejected alternatives, and the touched line ranges mapped through later edits.

~81 tests (`cargo test --workspace`), clippy and fmt clean.

## Phase 2 — Visual mode shell ⏳

Not started. See `docs/KICKOFF.md` Phase 2. First task is re-evaluating
`gpui-component` (ADR 0003 deferred the decision).

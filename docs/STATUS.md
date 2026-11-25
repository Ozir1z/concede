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

## Phase 2 — Visual mode shell ✅

`concede-app` opens a repo and lets you review a day of agent work from the
keyboard. All app logic is one file (`crates/concede-app/src/main.rs`), a thin
layer over `concede-core`, built on raw GPUI (ADR 0008).

- **ADR 0008** — re-evaluated `gpui-component`; it still rides a republished GPUI
  snapshot (a different type universe from our pinned official GPUI), so adopting
  it means replacing our GPUI. Decision: build Phase 2 on raw GPUI; revisit at the
  full code-editor stage.
- **Story inbox**: unreviewed decisions grouped by agent session, from an owned
  `AppData` snapshot (the view can't borrow from `Log`).
- **Detail pane**: selecting a decision shows task, reasoning, rejected
  alternatives, and each touched file's current code with the changed lines
  highlighted (ranges via `why`). Draggable scrollbars: vertical on both panes,
  horizontal per code block.
- **Keyboard-first + review + keymap**: arrows/`j`/`k` move, Enter/Esc (or `l`/`h`)
  switch panes, Shift-A/Shift-R (or `a`/`r`) approve/reject via the core `review`
  API. Keymap config at `$CONCEDE_KEYS` or `~/.config/concede/keymap.conf`.
- **Code mode (ADR 0009)**: `c` opens the selected decision's file in a basic
  editable view (caret on its first changed line); ⌘S/Ctrl-S saves to disk, Esc
  discards and returns to the exact same spot. Editing is a pure, unit-tested
  `EditBuffer`; a `ConcedeCode` key context stops visual-mode binds firing while
  typing.
- **Frame-time instrumentation (ADR 0010)**: times each `render` build and warns
  on stderr when it exceeds the 16 ms budget (throttled); `CONCEDE_FRAME_TRACE=1`
  logs every frame, `CONCEDE_FRAME_BUDGET_MS` tunes the threshold.

**Done-when, verified on macOS (2026-11-24):** reviewed, approved/rejected and
edited decisions in the sample repo entirely from the keyboard; with
`CONCEDE_FRAME_TRACE=1`, render builds measured ~0.2–1.5 ms, well under the 16 ms
budget — it feels instant.

~93 tests (`cargo test --workspace`), clippy and fmt clean.

Deferred to a later phase (noted in the ADRs): a real code editor (LSP +
tree-sitter), text selection/IME/mouse-caret and horizontal scroll in code mode,
file cycling for multi-file decisions, logging human edits as decisions, and
moving the synchronous log/file reads off the UI thread with file-watching.

## Phase 3 — Change Stories ✅

AI output is grouped by **intent**, not just session, and reviewed a story at a
time (VISION pillar 3). See `docs/KICKOFF.md` "Later" / VISION.

- **Model (ADR 0011)**: `Decision` gains an optional `intent` label (additive,
  schema stays v1); `Log::stories()` groups decisions by recorded intent, falling
  back to session then ungrouped — decisions sharing an intent group across
  sessions. Each story carries its touched-file union and aggregate review state
  (`StoryReview`/`StoryStatus`), ordered by earliest-touch. Dependency ordering,
  boilerplate-collapsing and criticality highlighting are deferred (need code
  facts / later engines).
- **Capture**: `concede log --intent`, the same field in `log --stdin` JSON, and an
  optional `intent` on the MCP `record_decision` tool.
- **CLI**: `concede stories [--json]` — stories with status, decisions and files.
- **App**: the inbox is grouped into stories (intent / session fallback) with a
  status chip; **whole-story approve/reject** (`⇧A`/`⇧R` in vim, `⌃A`/`⌃R` in the
  default keymap) reviews every unreviewed decision in the selected story at once.
  Per-decision review, detail and code mode are unchanged.

**Done-when, verified on macOS (2026-11-24):** in the sample repo, a decision
tagged `--intent order-validation` forms its own story in both `concede stories`
and the app, while untagged work falls back to session groups; approving part of a
story shows a `N of M reviewed` status.

~96 tests (`cargo test --workspace`), clippy and fmt clean.

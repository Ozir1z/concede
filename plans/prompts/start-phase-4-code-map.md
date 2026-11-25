# Start Concede: Phase 4 — the Code Map

Paste everything below the line into Claude Code at the root of the `concede` repository.

---

We're building **Concede**, a free, open-source, native IDE for AI-augmented coding, in Rust
with GPUI. Read these first, in this order; they are the source of truth:

1. `CLAUDE.md`: commands, layout and working rules (follow them).
2. `docs/VISION.md`: goals, interaction model, the six pillars, criticality, version control.
3. `docs/KICKOFF.md`: the original phased roadmap and the "Later" list.
4. `docs/STATUS.md`: what's actually done per phase (Phases 0–3 done; Phase 4 is next).
5. `docs/decisions/`: ADRs 0001–0012. Most relevant now: **0008** (build on raw GPUI, don't
   adopt `gpui-component` yet), **0011** (change-stories model), and **0012** (this phase's
   direction decision).

## Why we're here (the point)

AI now writes most of the code, so the developer's job is shifting to steering, reviewing and
deciding. Concede is meant to be a **visual-first navigator and reviewer** for that, with a
good-enough editor — not another VS Code. The differentiator is the **decision log**: because
agents record *which intent produced each change at write time*, Concede knows which hunk
belongs to which feature (tools like GitButler have to guess).

Honest status after Phases 0–3: it works, but it **feels like a code reviewer** — a PR queue.
We've built the ledger and the review surface; we have **not** built the *navigator* half. The
home is a list, not a map. **Phase 4 fixes that** by building the Code Map. First impression
matters for "a different kind of IDE," and it must feel instant.

## Where we are (all on `main`, pushed)

- **Phase 0** scaffold, **Phase 1** decision log (core + CLI + MCP), **Phase 2** visual-mode
  shell (story inbox, detail pane, keyboard-first review, basic code mode, frame-time
  instrumentation), **Phase 3** Change Stories (recorded `intent`, `Log::stories()`, `concede
  stories`, and the app grouped by story with whole-story review).
- `cargo test --workspace` is green (~96 tests), clippy and fmt clean.
- App logic is one file: `crates/concede-app/src/main.rs`. Core is `crates/concede-core`.

## Phase 4 — Code Map (pillar 2)

**Goal (from the vision):** "structure first, files second. Changed areas glow, critical zones
are marked. File view stays available on demand." The map becomes the **home** of the app, and
one key zooms from a node into its code and back to the same spot. Scope v1 to **existing
codebases** (ADR 0012); the new-project **wizard is deferred** — a new project has nothing to
map yet.

**Show me a plan and wait for my OK before writing code for each step.** Proposed steps (adjust
in the plan):

1. **Initial read / structure index (core).** Walk the repo and parse files with **tree-sitter**
   to build a `CodeMap` model: modules/namespaces → types → members, with each node's file +
   line span. **C# first** (the sample is C#). Keep it off the UI thread — a big repo must not
   block. Propose the dependency (`tree-sitter` + `tree-sitter-c-sharp`) and a short ADR; **ask
   before adding the crates** (working rules). No custom parsers (VISION): tree-sitter only.
2. **Map rendering (app) — the new home.** Render the structure as a legible, **snappy** map
   with keyboard navigation between nodes (`hjkl` / arrows, consistent with the existing
   `FocusArea` + keymap model). This is the first impression — make it land. **Decide the
   layout** in the plan: I lean toward a deterministic, zoomable **nested/treemap-style**
   structure (modules containing types containing members) over a force-directed graph, which
   is non-deterministic and tends to feel janky — but make the case either way. The old story
   inbox should stay reachable (a view toggle), since it's how you review.
3. **Change heat overlay.** Make changed areas **glow**, computed from data we already have —
   the decision log (unreviewed/recent decisions per file+lines) and git churn via
   `ChangeStore`. This is where the map connects to Phases 1–3: you *see* where the AI has been
   working, and jump straight to the story/decision for a node.
4. **One-key zoom into code.** From a selected node, one key zooms into that symbol's code
   (reuse/extend the existing code view / code mode); Esc zooms back to the **same spot** on the
   map. The transition should feel like a zoom, not a context switch — the user never loses
   their place (interaction model in VISION).

**Deferred, don't design against (ADR 0012 + VISION):** the criticality engine (declared zones
first, then code facts / blast radius) that will make "glow" mean "this matters"; LSP (Roslyn)
for cross-references and blast radius; version-control-by-story; the agent loop / One Surface;
the Project Wizard.

**Keep on raw GPUI (ADR 0008).** tree-sitter is a parsing library and does not touch GPUI's
type universe, so it does not reopen the `gpui-component` question. The full code editor + LSP
is still the later trigger for that re-evaluation.

## Running and testing the app (macOS)

- **Toolchain:** this Mac uses **rustup** (`~/.cargo/bin`), pinned by `rust-toolchain.toml`.
  **The Bash tool runs non-login zsh, so prefix tool commands with
  `export PATH=$HOME/.cargo/bin:$PATH`.**
- **Metal:** the GPUI app needs Apple's Metal Toolchain (already installed). Core/CLI don't.
- **Run the app:** `cargo run -p concede-app -- <repo>` (defaults to cwd).
- **Test data:** `scratch/sample/` (git-ignored) is a C# repo with a `.concede/` log and a few
  change stories (including an `order-validation` intent). It's a good first target for the map.
  Add more with `concede log` (build the binary, then run it from `scratch/sample`). The C#
  files are `src/Order.cs`, `src/OrderService.cs`, `src/Reporting.cs`, plus `tests/`.
- **I'm on the Mac and can watch the window** — launch the app and ask me what I see; I can't
  screenshot (screen-recording permission is off). I care about the first impression, so show
  me the map early and iterate on how it reads and feels.

## GPUI gotchas we already hit (pinned Zed rev c016f436) — don't rediscover these

- **Scrollable elements need a stateful element**: call `.id(...)` before `overflow_*_scroll`.
- **Scroll geometry is only known after a layout.** `ScrollHandle::{offset,max_offset,bounds}`
  read the previous frame; the view draws one extra "primed" frame so scrollbars size right.
  Use `on_scroll` (not the deprecated `on_scroll_wheel`) + `cx.notify()`.
- **Keyboard = GPUI actions**: `actions!` unit structs + `KeyBinding` under a key context, with
  `on_action` on a `track_focus`'d root. Code mode switches the root's key context to
  `ConcedeCode` so typing doesn't trigger visual-mode binds — reuse this pattern for any new
  mode (ADR 0009). Config keys are `Box::leak`ed to get `'static`.
- **No per-side border colour** (`border_color` sets all sides); the focused-pane accent is a
  2px absolute strip. **`Pixels` wraps a private f32** — convert with `f32::from(p)`.
- **Don't recreate a `ScrollHandle` under the same element id** — GPUI loses its geometry; reuse
  a stable handle and `set_offset`.
- **Snappy bar:** anything over 16 ms on the UI thread is a bug. `FrameStats` already logs slow
  render builds; run with `CONCEDE_FRAME_TRACE=1` while building the map to watch frame times,
  and keep the initial repo read on a background task.
- **Never copy code from the Zed editor (GPL-3.0).** Build on GPUI only.

## Working rules (short version; full list in CLAUDE.md)

- Plan first, wait for my OK per step. Small commits with clear messages, and brief Rust notes
  for me (senior .NET dev, newer to Rust) in commit messages or ADRs.
- Never push, merge or rebase unless I say so. **Ask before adding a dependency** (tree-sitter
  will be the first real one for this phase).
- Record design decisions as ADRs. Run `cargo clippy` and `cargo fmt` before each commit.
- I like to watch each step in the running app; show progress and what's left as you go, and
  treat the first impression of the map as a feature in itself.

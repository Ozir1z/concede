# Continue Concede: Phase 2, step 5 (code mode) and step 6 (frame times)

Paste everything below the line into Claude Code at the root of the `concede` repository.

---

We're building **Concede**, a free, open-source, native IDE for AI-augmented coding, in Rust
with GPUI. Read these first, in this order; they are the source of truth:

1. `CLAUDE.md`: commands, layout and working rules (follow them).
2. `docs/VISION.md`: goals, interaction model, pillars.
3. `docs/KICKOFF.md`: the phased roadmap (Phase 2 is the current one).
4. `docs/STATUS.md`: what's actually done per phase.
5. `docs/decisions/`: ADRs 0001–0008. 0008 is the Phase 2 GPUI-component decision.

## Where we are

Phase 0 (scaffold) and Phase 1 (decision log: core, CLI, MCP server) are done and on `main`.

**Phase 2 (visual mode shell) steps 1–4 are done and committed to `main` (not pushed):**

1. **ADR 0008** — re-evaluated `gpui-component`; it still rides on a republished GPUI
   snapshot (`gpui-pre`), a different type universe from our pinned official GPUI, so adopting
   it means replacing our GPUI. Decision: **build Phase 2 on raw GPUI**, revisit only at the
   full code-editor stage.
2. **Story inbox** (`concede-app`): opens the repo in the cwd (or a path arg), reads the log
   once, renders unreviewed decisions grouped by agent session. Owned `AppData` snapshot (the
   view can't borrow from `Log`).
3. **Detail pane**: two-pane layout; selecting a decision shows task, reasoning, rejected
   alternatives, and each touched file's current code with changed lines highlighted (ranges
   via `why`). Plus **draggable scrollbars**: vertical on both panes, horizontal per code block.
4. **Keyboard-first + review + keymap**: Up/Down (vim `j/k`) move selection (list auto-scrolls
   to follow); Enter/Esc (vim `l/h`) switch panes (accent strip + header hint); Shift-A/Shift-R
   (vim `a/r`) approve/reject via the core `review` API (decision drops from the inbox). Keymap
   config at `$CONCEDE_KEYS` or `~/.config/concede/keymap.conf` (`mode = vim|default`, plus
   `bind <action> = <key>`).

All logic for the app lives in one file: `crates/concede-app/src/main.rs`. `cargo test
--workspace` is green (~85 total, incl. app helper tests), clippy and fmt clean.

## Next: step 5 (code mode), then step 6 (frame times)

From the kickoff, the Phase 2 "done when" is: *I can review and approve a day of agent work
without leaving the keyboard, and it feels instant.* Two steps remain.

**Step 5 — code-mode toggle** (show me the plan and wait for my OK before writing code):

- One key opens the selected hunk in a **basic editable** text view (a plain editable text
  area is fine for now — not a real code editor; that's "Later" in the kickoff).
- `Esc` returns to the **exact same spot** in visual mode (same selected decision, same scroll).
- Decide the interaction: which key enters code mode, what's editable, whether edits are
  saved to disk or discarded for now (propose the simplest useful behavior and an ADR if it's
  a real design choice).
- Keep it keyboard-first and consistent with the existing `FocusArea` model.

**Step 6 — frame-time instrumentation**:

- Measure UI-thread frame times and log anything over **16 ms** (the snappiness bar in
  CLAUDE.md). Check how GPUI exposes per-frame timing at our pinned rev (there's a profiler
  module; `window.on_next_frame` / a frame callback may be the hook). Keep it off the hot path.

Then update `docs/STATUS.md` (mark Phase 2 done when both land) and propose the Phase 3 / the
next "Later" item from the kickoff.

## Running and testing the app (macOS)

- **Toolchain:** this Mac uses **rustup** (`~/.cargo/bin`), pinned to Rust 1.100.0 by
  `rust-toolchain.toml`. Homebrew's `rust` was removed (it was broken against a newer Homebrew
  LLVM). Fish picks up `~/.cargo/bin` via `~/.config/fish/conf.d/rust.fish`. **The Bash tool
  runs non-login zsh, so prefix tool commands with `export PATH=$HOME/.cargo/bin:$PATH`.**
- **Metal:** the GPUI app needs Apple's Metal Toolchain (already installed via
  `xcodebuild -downloadComponent MetalToolchain`). `concede-core`/`concede-cli` don't need it.
- **Run the app:** `cargo run -p concede-app -- <repo>` (defaults to cwd). It shows the inbox
  of whatever repo you point it at; from the project root it says "No decision log" (there's no
  `.concede/` there), which is expected.
- **Test data:** `examples/try-it.sh <name>` makes a scratch repo (`scratch/`, git-ignored)
  from `examples/sample-csharp/` wired for Claude Code. `scratch/sample/` already exists with a
  `.concede/` log. Add more decisions fast (no LLM needed) with `concede log`, e.g.:
  `concede log --agent claude-code --model claude-sonnet-5-5 --session demo-a --task "..."
  --reasoning "..." --file src/Order.cs:10-14`. A file with a long line (e.g.
  `scratch/sample/src/Reporting.cs`) is useful to exercise the horizontal scrollbar.
- **I'm on the Mac and can watch the window** — launch the app and ask me what I see; I'll
  confirm visual things you can't (you can't screenshot: screen-recording permission is off).

## GPUI gotchas we already hit (at the pinned Zed rev c016f436) — don't rediscover these

- **`overflow_y_scroll` / `overflow_x_scroll` need a stateful element**: call `.id(...)` first.
- **Scroll geometry is only known after a layout.** `ScrollHandle::{offset,max_offset,bounds}`
  read the *previous* frame. The view draws one extra frame on startup and after each selection
  (a `primed` flag) so scrollbars size correctly. `on_scroll` (NOT the deprecated
  `on_scroll_wheel`) + `cx.notify()` keeps them live while scrolling.
- **Flex/block children fill their parent's width**, so no-wrap code was clipped, not
  scrollable. Code content is `flex_none` inside a `flex` scroller so it takes its widest
  line's width; the detail chain carries **`min_w_0`** so a wide line can't blow out the pane
  (that blowout had also pushed the vertical scrollbar off-screen).
- **Don't recreate a `ScrollHandle` under the same element id** — GPUI loses its measured
  geometry and the scrollbar silently breaks. Use a stable, reused pool (see `code_scroll_pool`)
  and reset offsets with `set_offset` instead.
- **`scroll_to_item(ix)` indexes a scroll container's *direct* children.** The list is a flat
  sequence of `[heading, row, row, heading, …]`; `list_child_index` maps a decision to its
  child position so selection keeps it on screen.
- **No per-side border colour**: `border_color` sets all sides. The focused-pane accent is a 2px
  absolute strip (`focus_strip`), not a top border.
- **`Pixels` wraps a private f32** — convert with `f32::from(p)`, not `.0`.
- **Keyboard** = GPUI actions: `actions!` unit structs, `KeyBinding` under the `"ConcedeRoot"`
  key context, `on_action` on the `track_focus`'d root (focused once on startup). `cx.listener`
  adapts a view closure to GPUI's `Fn(&E, &mut Window, &mut App)`. `KeyBinding::new` needs a
  `'static` keystroke, so config-supplied keys are `Box::leak`ed.
- **chrono** is a direct dep of `concede-app` (local-time formatting), same version as core/cli.

## Working rules (short version; full list in CLAUDE.md)

- Plan first, wait for my OK per step. Small commits with clear messages, and brief Rust notes
  for me (I'm a senior .NET dev, newer to Rust) in commit messages or ADRs.
- Never push, merge or rebase unless I say so. Ask before adding a dependency.
- Record design decisions as ADRs. Run `cargo clippy` and `cargo fmt` before each commit.
- I like to watch each step in the running app; show progress and what's left as you go.

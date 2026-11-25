# 0009. Basic code mode: whole-file editable buffer, explicit save, minimal input

- Status: accepted
- Date: 2026-11-24

## Context

Phase 2, step 5 (KICKOFF.md): "One key switches to code mode on the selected hunk (a basic
editable text view is fine for now); Esc returns to the exact same spot in visual mode."

The real code editor — LSP, tree-sitter syntax, DAP — is explicitly a "Later" concern. So
this step needs a *basic* editable view, not an editor. Three sub-decisions were open, and
two were put to the user.

GPUI (at our pinned Zed rev, ADR 0002) ships **no** text-editor widget; that lives in Zed's
GPL `editor` crate, which we must not use (ADR 0008 / VISION). The only editable-text
primitive GPUI gives us is a low-level one: its `examples/input.rs` implements a text field
in ~900 lines by hand (a custom `Element` that lays out glyphs and a caret, plus an
`EntityInputHandler` that the platform's IME calls into).

## Decision

1. **Edit the whole file, not a sliced-out hunk.** Code mode opens the selected decision's
   first on-disk file as one editable buffer, with the decision's changed lines still
   highlighted and the caret placed on the first changed line. Editing a slice and splicing
   it back into its surrounding context is fiddly and error-prone; a whole-file buffer makes
   both the edit model and saving trivial. Decisions touching several files open the first
   readable one; cycling files is deferred.

2. **Explicit save to disk; Esc discards.** `cmd/ctrl-s` writes the buffer to the working
   tree; `Esc` leaves code mode and drops any unsaved edits, returning to the exact same
   visual-mode spot (same selected decision, same scroll — code mode never touches the list
   or detail scroll handles). This makes code mode genuinely useful for a small fix during
   review, rather than a read-only toy. A human edit saved this way is **not** yet recorded
   as a decision/capture entry — logging human edits is future work and out of this step's
   scope.

3. **Minimal input, hand-rolled on `on_key_down` — not the full GPUI input primitive.**
   We handle typed characters (`keystroke.key_char`), Backspace/Delete, Enter, Tab (→ four
   spaces), the arrow keys, Home/End, and clipboard paste (`cmd/ctrl-v`). No text selection,
   no IME, no mouse-placed caret, no undo, no syntax highlighting, no horizontal
   caret-follow. This is ~150 lines against ~500+ to adapt `input.rs`, and it matches the
   "basic ... fine for now" bar. The full primitive is the right move only when we build the
   real editor, at which point ADR 0008's `gpui-component`/editor question reopens anyway.

All edit/caret logic lives in a pure `EditBuffer` struct (`lines`, caret `row`/`col`) with
unit tests; the view only renders it and feeds it key events.

### Rust/GPUI note (for a .NET reader)

GPUI keybindings are matched against the **whole ancestor context stack** of the focused
element, like event bubbling up the visual tree. If the editor were a child under a root
that still advertised the `"ConcedeRoot"` context, our single-letter visual-mode binds
(`a` = approve, `j`/`k` = move, …) would fire *while you type* — pressing `a` would approve
the decision instead of inserting an `a`. So the root element reports a **different**
context, `"ConcedeCode"`, whenever code mode is active. In that context only `Esc`, save and
paste are bound; every other key has no binding and so flows to the editor's `on_key_down`
handler as plain text input. Switching the one context string is the whole mechanism — no
per-key suppression needed.

## Alternatives rejected

- **Adapt GPUI's `input.rs` now** (full `EntityInputHandler` + custom glyph/caret element):
  selection, IME and mouse caret, but ~500+ lines of machinery for a step whose bar is
  "basic." Deferred to the real-editor stage.
- **Edit only the hunk's lines.** Avoids showing the whole file, but splicing an edited slice
  back into the file (and keeping line numbers/highlights coherent) is more complex than
  editing the whole buffer, for no real benefit at this scale.
- **In-memory-only / always discard** (a pure preview). Simpler and side-effect-free, but
  code mode then can't actually fix anything; the user chose explicit save.
- **Reuse the read-only code view's 2-D scroll + horizontal scrollbar.** Code mode uses a
  vertical scroller with caret-follow only; horizontal scrolling of long lines in the editor
  is a known gap, deferred with the rest of the "real editor" work.

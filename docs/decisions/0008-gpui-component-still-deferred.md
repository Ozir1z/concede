# 0008. Keep deferring gpui-component; build Phase 2 UI on raw GPUI

- Status: accepted
- Date: 2026-11-24

## Context

ADR 0003 deferred adopting `gpui-component` (Longbridge) and promised a time-boxed
re-evaluation at the start of Phase 2, when we first need real UI (a story inbox, then a
detail/code view). This ADR is that re-evaluation.

State of `gpui-component` today (2026-11-24, checked against its `Cargo.toml` on `main`):

- It now ships under a `gpui-kit` umbrella crate; license is Apache-2.0 (no GPL concern).
- Its GPUI still does **not** come from official `zed-industries/zed`. The workspace pins
  republished snapshots with exact `=` requirements and renames them to the official names:
  `gpui = { package = "gpui-pre", version = "=0.3.14" }`,
  `gpui_platform = { package = "gpui-pre-platform", ... }`, etc. An optional `gpui-fast`
  backend (Longbridge's own GPUI fork) exists too. There is no `[patch]` section, no
  `git`/`rev`, and no documented way to build it against official GPUI.

The blocker is not build cost; it is type coupling. `gpui-component`'s widgets are generic
over the types of *its* `gpui` (the `gpui-pre` snapshot): `Window`, `App`, `Element`,
`Context`, and so on. Our `concede-app` uses the official `gpui` crate from the Zed rev we
pin in ADR 0002. To Rust these are two different crates, so their identically-named types
are different, incompatible types: you cannot pass our `gpui::Window` to a component that
expects `gpui_pre::Window`. Adopting `gpui-component` therefore means replacing our GPUI
with Longbridge's republished snapshot across the whole app — the precise coupling ADR 0002
rejected ("no third-party maintainer between us and our most critical dependency"), plus its
weekly, breaking release cadence (ADR 0003).

> Rust note (for a .NET reader): there is no single global `gpui` the way a .NET assembly is
> resolved once per app domain. Two packages (`gpui` and `gpui-pre`) can both be in the
> build at once, and a type from one is not assignable to the same-named type from the other
> — closer to two different `System.Windows.Window` types from two different, unrelated DLLs
> than to two versions of one NuGet package. So this cannot be bridged by a cast or a shim;
> only by making the whole app use one of them.

What Phase 2's early steps actually need — a scrollable selectable list, a text/detail pane,
a split layout, and keyboard dispatch — are all things raw GPUI provides directly. The one
place `gpui-component` would clearly save work is a full code editor with tree-sitter + LSP,
but Phase 2 only calls for a "basic editable text view," and the real editor is a "Later"
concern in the kickoff.

A full compile spike was not run: it would pull Longbridge's entire GPUI fork for a build
whose outcome (a second, incompatible `gpui` in the graph) is already determined by the
dependency facts above, and a successful compile would not make the two type universes
interoperate.

## Decision

Keep deferring `gpui-component`. Build the Phase 2 visual shell (story inbox, detail pane,
keyboard navigation, basic code view) directly on the official GPUI we already pin.

Re-evaluate once more only when we reach the full code editor (LSP + tree-sitter,
"Later" in the kickoff). At that point the trade changes — the editor is a large build we'd
otherwise own — and the question becomes whether that single feature justifies moving the
whole app onto `gpui-pre`/`gpui-fast`, or whether we embed just the editor behind our own
`ChangeStore`-style boundary.

## Alternatives rejected

- **Adopt `gpui-component` now (via `gpui-kit`).** Replaces official GPUI with a republished
  snapshot for the whole app before we need any component it uniquely provides; reintroduces
  the third-party-maintainer coupling and weekly breakage ADR 0002/0003 guarded against.
- **Patch `gpui-pre` to our pinned Zed `gpui` via `[patch]`.** Only works if the snapshot is
  API-identical to our exact rev; it is a moving target on a weekly cadence, so this trades
  our reproducible pin for constant, fragile reconciliation.
- **Run a full build spike anyway.** The adopt/defer question turns on coupling, which the
  `Cargo.toml` already answers; a compile would only measure build cost for an option we are
  declining on other grounds.
- **Rule `gpui-component` out permanently.** Its editor/list/dock could still save months at
  the full-code-mode stage; keep it on the table for that one decision.

## Consequences

- Phase 2 proceeds on raw GPUI. We write the list, detail and keyboard layers ourselves;
  they stay small and fully under our control, and the app keeps a single GPUI (our pin).
- We take on the code-editor build ourselves if and when we reach it, unless the later
  re-evaluation flips. Nothing in Phase 2 is designed in a way that blocks adopting
  `gpui-component`'s editor later behind a boundary.
- No new dependency is added in this step.

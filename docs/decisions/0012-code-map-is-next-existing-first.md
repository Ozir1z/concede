# 0012. Phase 4 is the Code Map; map existing codebases first, wizard later

- Status: accepted
- Date: 2026-11-24

## Context

After Phases 0–3 we have a working decision log (core), a CLI + MCP server, and a GPUI
review surface: the story inbox grouped into change stories, a detail pane with
reasoning/alternatives/code, keyboard-first navigation and review, a basic code mode, and
frame-time instrumentation.

Stepping back, it works but **feels like a code reviewer** — a PR queue — not the "navigator"
half of the vision. `VISION.md` is explicit that Concede should be "a great navigator and
reviewer plus a good-enough editor," and its home (interaction model + pillar 2) is a **Code
Map**, not a list. We've built the reviewer half (the decision-log ledger + the review loop);
the three things that make it feel like a place you *work* are all missing:

1. **No map — the home is a list**, which is inherently a reviewer's artifact. The map is what
   makes it a navigator.
2. **Nothing triages attention** (no criticality), so every change looks equal — the "too much
   to review" problem restated.
3. **Concede doesn't drive anything** — it reads logs an external agent wrote. The "One
   Surface" / agent loop is unbuilt, so it's a companion to someone else's session.

For "a different kind of IDE," the **first impression** has to land and has to be snappy. The
map is the highest-leverage move for that.

## Decision

1. **Phase 4 is the Code Map (pillar 2)** — prioritized above the criticality engine, the
   version-control layer and the agent loop, specifically to fix the first impression and
   deliver the navigator half. All six pillars remain in scope; this ADR sets the *order*.

2. **Scope v1 to existing codebases.** Opening a repo triggers an **initial read** that builds
   a structure map (modules/namespaces → types → members) with **tree-sitter**, C# first (our
   sample). Changed areas **glow** using data we already have (the decision log + git change
   heat). One key **zooms** from a node into its code; Esc zooms back to the same spot, so you
   never lose your place.

3. **Defer the new-project flow (Project Wizard, pillar 1).** A new project has nothing to
   map; the wizard writes the initial structure/spec and is more useful once the map exists to
   render what it creates.

4. **Stay on raw GPUI (ADR 0008 still holds).** `tree-sitter` is a parsing library, not part
   of GPUI's type universe, so it does **not** reopen the `gpui-component` question. LSP
   (Roslyn) and the full editor remain the later trigger for that re-evaluation; map v1 leans
   on tree-sitter structure, with LSP-powered blast-radius / cross-references deferred.

## Deferred (noted, not against)

- **Criticality engine** — layered onto the map later (declared zones first), so "glow" can
  mean "this matters," not just "this changed."
- **Version-control-by-story** — commit/branch a whole story once the map + criticality make
  risk visible; the decision log already knows which hunk belongs to which intent.
- **Agent loop / One Surface** — Concede hosting the models is the deepest remaining gap; do it
  after the navigator exists.

## Alternatives rejected

- **Version control next.** Makes a more *powerful* reviewer, not a more *intuitive* IDE;
  doesn't change the first impression.
- **Criticality next.** Valuable triage, but without the map it only risk-ranks the existing
  list — still a reviewer.
- **Agent loop next.** Arguably the deepest gap, but higher-risk and a weaker visible
  first-impression win than the map; better once the navigator exists.

## Note on dependencies (for when the phase starts)

The Code Map will likely need the `tree-sitter` crate plus a C# grammar
(`tree-sitter-c-sharp`). These are new dependencies, to be proposed and agreed per the working
rules (CLAUDE.md) before adding them.

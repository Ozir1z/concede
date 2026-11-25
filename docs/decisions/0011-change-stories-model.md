# 0011. Change Stories: a recorded `intent`, fallback grouping, earliest-touch order

- Status: accepted
- Date: 2026-11-24

## Context

Phase 3 builds the **Change Stories** pillar (VISION.md): "AI output grouped by intent,
ordered by dependency, boilerplate collapsed, critical code highlighted. Accept, edit or
reject per group." VISION is pointed that intent is first-class and recorded, not inferred:
"the decision log knows which intent produced each change," and history is "captured at write
time, not reconstructed."

Until now a `Decision` had no notion of intent; the inbox grouped only by agent `session`.
This ADR covers the **core model** for stories (step 1 of the phase). Recording intent from
the CLI/MCP (step 2) and the app view with whole-story review (step 3) build on it.

## Decision

1. **A recorded `intent` field.** `Decision` gains `intent: Option<String>` — a short,
   free-text label of what a change is *for* (free text like `task` and `allowed_by`, until
   richer engines exist). It is additive and optional (`serde(default,
   skip_serializing_if)`), so existing `.concede/` logs deserialize unchanged with
   `intent: None` and **`SCHEMA_VERSION` stays at 1**. The checked-in JSON schema is
   regenerated (`CONCEDE_UPDATE_SCHEMA=1`).

2. **`Log::stories()` with fallback grouping.** A story's key is the recorded `intent` when
   present, **else** the agent `session`, **else** `Ungrouped`. Decisions that share an
   intent group together *even across sessions* — the whole point of a recorded intent.
   Untagged work still forms sensible stories (one per session) so existing logs are useful
   immediately; this is the "recorded + fallback" choice over intent-only or purely-derived
   grouping.

3. **Aggregate review state.** Each `Story` carries a `StoryReview` (counts of unreviewed /
   approved / rejected) and a derived `StoryStatus` (Unreviewed / Mixed / Reviewed), so the
   UI can show and act on a story as a unit, and the union of its touched files.

4. **Earliest-touch ordering, for now.** Stories come out ordered by their earliest decision,
   and decisions within a story stay in creation order — this falls straight out of iterating
   decisions oldest-first, no extra sort.

### Rust note (for a .NET reader)

`Story<'a>` and `StoryKey<'a>` borrow from the `Log` (the `&str` intent/session and the
`&Decision`s), so they live only as long as the borrow — like a `ref struct`/span over the
log's data, not an owned copy. The app already copies what it needs into an owned snapshot
(`AppData`) for the same reason the inbox does; step 3 will do the same for stories.

## Deferred (noted, not built in Phase 3 step 1)

- **"Ordered by dependency."** True dependency ordering needs code facts (which story's
  changes depend on another's), which come from the Code Map / LSP — not available yet. The
  earliest-touch order is the proxy until then.
- **Boilerplate collapsing** and **criticality highlighting** from the pillar depend on later
  engines (criticality, tree-sitter) and are out of scope here.
- **Intent inference for untagged work.** The fallback groups by session rather than guessing
  intent, keeping to VISION's "recorded, not reconstructed."

## Alternatives rejected

- **Intent-only grouping (no fallback).** Cleanest model, but existing logs and hook-only
  captures would show almost no structure until re-tagged.
- **Purely derived/heuristic stories** (cluster on shared task text / overlapping files). No
  schema change and ships faster, but intent becomes reconstructed rather than recorded,
  cutting against the VISION principle.

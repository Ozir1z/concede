# 0010. Frame-time instrumentation: time the render build, log over-budget frames

- Status: accepted
- Date: 2026-11-24

## Context

VISION / CLAUDE.md make snappiness a non-negotiable pillar: "Anything over 16 ms on the UI
thread is a bug." Phase 2, step 6 (KICKOFF.md): "Measure and log frame times; anything over
16 ms on the UI thread is a bug."

What GPUI exposes at our pinned rev (ADR 0002):

- A `profiler` module, but it is feature-gated behind `hdrhistogram` and measures **async
  task** timings across threads (`ThreadTaskTimings`, poll durations) — a different axis from
  "how long did this frame's UI-thread work take," and a dependency we'd rather not pull.
- `Window::on_next_frame(cb)` — a one-shot callback at the start of the next frame.
- No app-accessible hook *after* layout/paint, so the real on-GPU/paint cost of a frame
  cannot be measured from application code at this rev.

The UI-thread work we actually author, and that scales with our data, is the element-tree
**build** inside `Render::render` — e.g. the code view emits one row element per file line, so
a large file is exactly where we'd risk blowing the budget. That build is a clean `Instant`
span needing no GPUI hook.

## Decision

Time the body of `ConcedeRoot::render` with `std::time::Instant` and log to stderr when a
frame's build exceeds a budget (default **16 ms**). A small `FrameStats` helper owns the
budget and the logging policy:

- **Breach-only, throttled.** Over-budget frames log at most ~once per second, carrying a
  "+N more since" count of the ones suppressed in between, so a sustained slow state doesn't
  flood the console. The common (fast) path is just a subtract-and-compare — the format/print
  cost only happens on the rare slow path, keeping the instrument off the hot path.
- **Env knobs.** `CONCEDE_FRAME_BUDGET_MS` tunes the threshold; `CONCEDE_FRAME_TRACE=1` logs
  every frame's time for active profiling.
- **No new dependency.** `eprintln!` + `std::time::Instant`. If we later adopt a logging
  framework (`tracing`), the sink swaps behind `FrameStats` without touching callers.

The breach/throttle/trace decision is a pure method, `record_at(elapsed, now) -> Option<String>`,
so it is unit-tested without stderr or the real clock; `render` calls it via `record(elapsed)`
with `Instant::now()`.

### Known limitation

This measures *our build*, not GPUI's subsequent layout/paint, which we cannot time from app
code at this rev. It is the right proxy for the work we control; if we later need the full
frame cost we'll revisit (a GPUI upgrade may expose a post-frame hook, or we add the
feature-gated profiler deliberately). Separately, the synchronous file read in `enter_code`
is UI-thread I/O that a large file could make slow; it's flagged in code to move to a
background task when we add file-watching (already foreshadowed in `main`).

## Alternatives rejected

- **GPUI's `profiler` module.** Async-task timing, feature-gated behind `hdrhistogram`; wrong
  axis and an unwanted dependency for a per-frame UI-thread budget.
- **Inter-frame interval via `on_next_frame`.** The gap between frames is dominated by idle /
  vsync waiting (naturally >16 ms when nothing is happening), so it measures wall-clock
  between redraws, not UI-thread work — not a useful "bug" signal.
- **Always-on per-frame logging.** Spams the console and adds print cost to every frame;
  replaced by breach-only logging with an opt-in `CONCEDE_FRAME_TRACE` for when you do want
  every frame.

# 0003. Defer adopting gpui-component

- Status: accepted
- Date: 2026-11-24

## Context

`gpui-component` (Longbridge, Apache-2.0, v0.11.0 on 2026-11-23) is the main component
library for GPUI, now shipped under a `gpui-kit` umbrella crate. It offers 75+ components,
including virtualised lists and tables, dock layouts, and a code editor with tree-sitter
highlighting and LSP support. Those match needs we'll have in Phase 2 (story inbox) and
code mode.

However, it does not depend on official GPUI. It pins `gpui-pre =0.3.14` (a Zed snapshot
republished by its maintainer) and `gpui-fast` (Longbridge's GPUI fork). Releases come
weekly and include breaking changes.

## Decision

Don't adopt it in Phase 0, which needs one window with a label. At the start of Phase 2,
run a short time-boxed test: can it build against our pinned official GPUI (ADR 0002), and
what does it cost in build time, binary size and API churn? Decide then with a new ADR.

## Alternatives rejected

- **Adopt now via `gpui-kit`.** Would replace our GPUI with Longbridge's fork before we
  need any components, coupling us to their release cadence from day one.
- **Rule it out permanently.** Its editor, lists and dock could save months of work;
  worth evaluating when we need them.

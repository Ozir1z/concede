# 0001. Record architecture decisions

- Status: accepted
- Date: 2026-11-24

## Context

Torch is a decision-traceability tool: it records why each change was made. We should
follow that practice ourselves, so future contributors (human or AI) can see why the
project looks the way it does.

## Decision

We record each significant design decision as a short Markdown ADR in `docs/decisions/`,
numbered sequentially (`NNNN-kebab-title.md`). Each ADR has: status, date, context,
decision, alternatives rejected, and consequences where relevant. ADRs are append-only:
to change a decision, add a new ADR that supersedes the old one and update the old one's
status.

## Alternatives rejected

- **No formal records, rely on commit messages.** Commit messages explain a change, not
  the decision behind a design, and are hard to find later.
- **A wiki outside the repo.** Decision history belongs next to the code (VISION.md
  principle).
- **Heavier templates (MADR full form).** More fields than we need; short records get written.

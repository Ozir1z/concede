# 0005. Decision log schema and storage

- Status: proposed
- Date: 2026-11-24

## Context

Every AI-made change must leave a "why" record in the repo, queryable per file and per line
range (VISION.md, History in Place). Records must survive branching and merging, be readable
without Concede installed, and carry hunk-level detail so that later a Change Story can map
to exactly the hunks it produced.

Records also change state after they are written: they get reviewed, and they get linked to
a commit. If that state lived inside the decision file, two branches reviewing the same
decision would produce a merge conflict, and the file would no longer be append-only.

## Decision

### Layout

All data lives in `.concede/` at the repository root (the git toplevel) and is committed
with the code.

```
.concede/
  decisions/<ULID>.json   one file per decision, immutable once written
  events/<ULID>.json      one file per state change (review, link), immutable
  captures/<ULID>.json    raw hook captures, see ADR 0007
```

- **Immutable files only.** Nothing is edited or deleted after it is written. New state is a
  new file. New files never conflict in git, so branches merge cleanly.
- **IDs are ULIDs**: 128-bit, time-ordered, Crockford base32 (26 chars). They sort by
  creation time as plain strings, are safe as filenames on every OS, and can be generated
  without coordination.
- **Atomic writes**: write to a temp file in the same directory, then rename. A crash never
  leaves a half-written record.
- **Versioned**: every file has `"schema": 1`. Readers reject unknown major versions with a
  clear error instead of guessing.

### Decision record

```jsonc
{
  "schema": 1,
  "id": "01JDB6K9V2Y7T3W8XQ4N5M1P0R",
  "timestamp": "2026-11-24T21:03:11Z",          // UTC, RFC 3339
  "source": "agent",                              // agent | hook | human
  "agent": {                                      // optional for source = human
    "tool": "claude-code",
    "model": "claude-opus-5-5",                   // optional: not every source knows it
    "session": "5b1e…"                            // optional: agent session id
  },
  "task": "Add retry to OrderService.Submit",    // required: prompt or task summary
  "reasoning": "Submit fails on transient …",    // required for source = agent | human
  "alternatives": [                               // optional, may be empty
    { "option": "Polly policy", "rejected_because": "new dependency for one call site" }
  ],
  "allowed_by": "unconfigured",                   // required, free text until the rules engine
  "base_commit": "3f9c2a1…",                      // HEAD when the change was made, if any
  "changes": [
    {
      "path": "src/OrderService.cs",              // repo-relative, forward slashes
      "after_blob": "9f2e4d…",                    // git blob id of the file after the change
      "hunks": [
        { "old_start": 40, "old_lines": 3, "new_start": 40, "new_lines": 11 }
      ]
    }
  ],
  "captures": ["01JDB6K8…"]                      // hook captures this decision explains
}
```

Hunk semantics follow unified diff (`git diff -U0`):

- `old_*` refers to the file before this change, `new_*` to the content in `after_blob`.
- Lines are 1-based. A pure insertion has `old_lines = 0`, and a pure deletion has
  `new_lines = 0`, with `new_start` being the line after which the deletion happened.
- The ranges plus `after_blob` are enough to reconstruct the exact text of each hunk, so we
  don't store the code itself.

Validation (in `concede-core`, run on write and on read):

- `id` is a valid ULID and matches the filename. `timestamp` parses.
- `task` and `allowed_by` are non-empty. `reasoning` is non-empty unless `source = hook`.
- Paths are relative, use `/`, and contain no `..` segment.
- Hunks within a change are sorted, don't overlap, and have `*_start >= 1` (or `0` with
  zero lines, per diff convention for changes at the top of a file).
- `changes` may be empty (a decision not to change something is still a decision).

### Event record

```jsonc
{ "schema": 1, "id": "01JDB7…", "decision": "01JDB6K9…", "timestamp": "…",
  "actor": "robbert",                             // who did it (git user.name by default)
  "kind": "review", "status": "approved",         // approved | rejected | unreviewed
  "note": "optional free text" }

{ "schema": 1, "id": "01JDB8…", "decision": "01JDB6K9…", "timestamp": "…",
  "actor": "robbert", "kind": "link", "commit": "a1b2c3…" }
```

A decision's current state is a fold over its events in ULID order:

- **Review status** is the status of the latest `review` event, or `unreviewed` if there
  is none. Re-reviewing is allowed, and history is kept.
- **Linked commits** are the set of all `link` events. A decision can end up in more than
  one commit (cherry-picks, a split story).
- Events pointing at an unknown decision are reported as warnings, not errors (the decision
  may arrive with a later merge).

A JSON Schema for both records is generated from the Rust types (via `schemars`) and
published at `docs/schema/`, so other tools can validate without Concede.

## Alternatives rejected

- **Mutable decision file** (status and commit fields edited in place). Simplest to read, but
  not append-only, and reviews on two branches conflict at merge.
- **One append-only log file** (JSON Lines). Every writer appends to the same file, so
  every branch conflicts on it.
- **Git notes.** They attach to commits, but decisions exist before any commit. Notes aren't
  pushed or fetched by default and most tools don't show them.
- **SQLite database in the repo.** It's a binary file: can't diff, can't merge, can't read
  without tooling. An index can be added later as a local cache, never as the source of truth.
- **UUIDv4 ids.** Not time-ordered, so folding and listing need the timestamp field.
  UUIDv7 would also work; ULID is shorter as a filename and equally well supported.
- **Storing the hunk text.** It duplicates what git already stores, and secrets in code would
  get copied into a second place.

## Consequences

- `.concede/` grows by one small file per decision and per event. That's fine for years of
  work. If listing ever gets slow, add a local, git-ignored index cache.
- Reading state means reading every event file. For Phase 1 volumes that's milliseconds. The
  app (Phase 2) will keep an in-memory index and update it on file changes.

# 0006. ChangeStore trait, implemented with the git CLI

- Status: accepted
- Date: 2026-11-24

## Context

All version-control access goes through a `ChangeStore` trait in `concede-core`, so the
engine can be swapped later (Jujutsu is the planned second engine, VISION.md). Phase 1 needs
only a little from it: the current HEAD, the hunks changed in a file, a snapshot of a file's
content, and a way to map old line numbers onto the current file.

Line numbers drift. A decision records "lines 40–50", then later edits move that code to
52–62. `concede why file:55` must still find the decision.

The two realistic implementations are running the `git` binary or using `gix` (gitoxide, a
pure-Rust git implementation).

## Decision

### Implementation: the git CLI

`GitCli` implements `ChangeStore` by running `git` as a child process (`std::process::Command`,
with `-C <repo>`, `--no-pager`, machine-readable flags, and `LC_ALL=C` for stable output).

- It behaves exactly like the user's own git: their config, attributes, line-ending
  settings, and any future git features, with no gap to explain.
- It adds no compile time and no dependency tree.
- `git diff -U0 --no-color` already gives exactly the hunk headers we need.
- A missing `git` on PATH is reported once with a clear error.

### The trait (Phase 1 methods only)

```rust
pub trait ChangeStore {
    /// Repository root (git toplevel).
    fn root(&self) -> &Path;
    /// Current HEAD commit, or None in a repo with no commits yet.
    fn head(&self) -> Result<Option<CommitId>>;
    /// Whether a commit exists (for `link`).
    fn commit_exists(&self, id: &CommitId) -> Result<bool>;
    /// Hunks in the working-tree file compared with HEAD (fallback for captures).
    fn changed_hunks(&self, path: &RepoPath) -> Result<Vec<Hunk>>;
    /// Store the current content of a file and return its blob id.
    fn snapshot(&self, path: &RepoPath) -> Result<BlobId>;
    /// Hunks between a stored blob and the current working-tree file.
    fn diff_blob_to_worktree(&self, blob: &BlobId, path: &RepoPath) -> Result<Vec<Hunk>>;
}
```

The domain types (`CommitId`, `BlobId`, `RepoPath`, `Hunk`) belong to `concede-core`, not to
git, so a Jujutsu engine can produce them too. Methods are synchronous: the CLI calls them
directly, and the app will call them from background tasks, never on the UI thread.

### Line-drift mapping

At record time, the file's content is stored as a git blob (`git hash-object -w`), and the
blob id is kept in the decision (`after_blob`, ADR 0005). At query time:

1. Diff `after_blob` against the current file (`diff_blob_to_worktree`).
2. Shift each recorded range through those hunks. A range that falls inside a changed area is
   kept but marked **modified since**, so the user knows the code has been edited again.
3. If the blob can no longer be found, show the recorded range as is and mark it
   **approximate**.

Git may prune unreferenced blobs after about two weeks (`gc.pruneExpire`). Once a decision is
linked to a commit, the blob is normally reachable from that commit's tree, so it isn't
pruned. Unlinked decisions older than that may fall back to approximate ranges. That's
acceptable for Phase 1. If it turns out to be a problem, we can keep blobs alive under a
`refs/concede/` ref.

## Alternatives rejected

- **`gix` (gitoxide).** Pure Rust, no runtime dependency on git, and faster for many small
  calls. But its blob-diff support is still maturing, it adds a large dependency tree, and
  its behaviour can differ from the user's git in edge cases (filters, attributes). It stays
  a good candidate for a second `GitGix` engine if process spawning ever shows up in
  profiles; the trait makes that a local change.
- **`git2` (libgit2 bindings).** C library through FFI, slower to follow git's features,
  and painful to build on Windows.
- **Storing line ranges only, without mapping.** Simple, but `why file:line` would give wrong
  answers after the first unrelated edit.
- **Relocating ranges by searching for the text.** Fragile with repeated or reformatted code;
  diff-based mapping is what git blame does and is well understood.

## Consequences

- `git` must be installed. It already is wherever Concede makes sense.
- Each query spawns a few processes (a few milliseconds each). Fine for the CLI. The app will
  cache results.
- Tests create real temporary git repos (`tempfile`) rather than mocking git.

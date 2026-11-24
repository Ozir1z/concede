//! Version-control access behind one trait, so the engine can be swapped (ADR 0006).
//!
//! The required methods are small primitives; the higher-level operations named in the
//! ADR (`changed_hunks`, `diff_blob_to_worktree`) are provided on top of them, so a
//! second engine (Jujutsu, gix) only implements the primitives.

use std::path::Path;

use crate::error::Result;
use crate::model::{BlobId, CommitId, Hunk, RepoPath};

/// What `concede-core` needs from a version-control system.
///
/// Methods are synchronous and may spawn processes or do I/O: call them from the CLI
/// directly, and from background tasks in the app, never on the UI thread.
pub trait ChangeStore {
    /// Repository root (the git toplevel).
    fn root(&self) -> &Path;

    /// Current HEAD commit, or `None` in a repository with no commits yet.
    fn head(&self) -> Result<Option<CommitId>>;

    /// Whether `id` names a commit in this repository.
    fn commit_exists(&self, id: &CommitId) -> Result<bool>;

    /// The configured user name, used as the default actor for reviews and links.
    fn user_name(&self) -> Result<Option<String>>;

    /// Store the current working-tree content of `path` and return its blob id, or
    /// `None` when the file doesn't exist (it was deleted).
    fn snapshot(&self, path: &RepoPath) -> Result<Option<BlobId>>;

    /// The blob of `path` in `commit`, or `None` when the file isn't in that commit.
    fn blob_at(&self, commit: &CommitId, path: &RepoPath) -> Result<Option<BlobId>>;

    /// Whether a blob is still available (unreferenced blobs can be garbage collected).
    fn blob_exists(&self, blob: &BlobId) -> Result<bool>;

    /// Hunks from `old` to `new`; `None` stands for an empty (absent) file.
    fn diff_blobs(&self, old: Option<&BlobId>, new: Option<&BlobId>) -> Result<Vec<Hunk>>;

    /// Hunks in the working-tree file compared with `base` (or with an empty file when
    /// `base` is `None` or the file isn't in it).
    fn changed_hunks(&self, base: Option<&CommitId>, path: &RepoPath) -> Result<Vec<Hunk>> {
        let old = match base {
            Some(commit) => self.blob_at(commit, path)?,
            None => None,
        };
        let new = self.snapshot(path)?;
        self.diff_blobs(old.as_ref(), new.as_ref())
    }

    /// Hunks from a stored blob to the current working-tree file.
    fn diff_blob_to_worktree(&self, blob: &BlobId, path: &RepoPath) -> Result<Vec<Hunk>> {
        let current = self.snapshot(path)?;
        self.diff_blobs(Some(blob), current.as_ref())
    }
}

/// Parse the hunk headers of a unified diff (`@@ -a,b +c,d @@`). Counts default to 1
/// when omitted, as in git's output.
pub fn parse_hunk_headers(diff: &str) -> Vec<Hunk> {
    diff.lines().filter_map(parse_hunk_header).collect()
}

fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let (old_start, old_lines) = parse_range(old)?;
    let (new_start, new_lines) = parse_range(new)?;
    Some(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
    })
}

fn parse_range(text: &str) -> Option<(u32, u32)> {
    match text.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_hunk_headers() {
        let diff = "\
diff --git a/x b/x
index 1111111..2222222 100644
--- a/x
+++ b/x
@@ -0,0 +1,2 @@
+new
+lines
@@ -5 +7 @@ fn context()
-old
+new
@@ -10,3 +11,0 @@
-a
-b
-c
";
        assert_eq!(
            parse_hunk_headers(diff),
            vec![
                Hunk {
                    old_start: 0,
                    old_lines: 0,
                    new_start: 1,
                    new_lines: 2
                },
                Hunk {
                    old_start: 5,
                    old_lines: 1,
                    new_start: 7,
                    new_lines: 1
                },
                Hunk {
                    old_start: 10,
                    old_lines: 3,
                    new_start: 11,
                    new_lines: 0
                },
            ]
        );
    }

    #[test]
    fn ignores_lines_that_merely_look_like_headers() {
        assert!(parse_hunk_headers("+@@ -1 +1 @@\n@@ garbage @@\n").is_empty());
    }
}

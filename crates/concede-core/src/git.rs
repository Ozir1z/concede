//! [`ChangeStore`] implemented by running the `git` binary (ADR 0006).

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use crate::change_store::{ChangeStore, parse_hunk_headers};
use crate::error::{Error, Result};
use crate::model::{BlobId, CommitId, Hunk, RepoPath};

/// A git repository, accessed through the `git` command-line tool.
#[derive(Debug)]
pub struct GitCli {
    root: PathBuf,
    /// The empty blob's id (differs between SHA-1 and SHA-256 repositories).
    empty_blob: OnceLock<BlobId>,
}

impl GitCli {
    /// Open the repository containing `start` (any directory inside the work tree).
    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let start = start.as_ref();
        let output = run_git(start, ["rev-parse", "--show-toplevel"], None)?;
        if !output.status.success() {
            return Err(Error::NotARepository {
                path: start.to_path_buf(),
            });
        }
        // Keep git's own spelling of the root for running commands. Canonicalising here
        // would give `\\?\C:\...` paths on Windows, which git doesn't accept everywhere;
        // callers canonicalise when they compare paths instead.
        let root = PathBuf::from(stdout_line(&output));
        Ok(Self {
            root,
            empty_blob: OnceLock::new(),
        })
    }

    /// Run git in the repository root and return stdout, failing on a non-zero exit.
    fn git<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<OsStr>,
    {
        let output = run_git(&self.root, args.clone(), None)?;
        check(&output, args)?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Run git and report only whether it exited successfully (for `--verify`-style
    /// queries where failure means "no", not "error").
    fn git_ok<I, S>(&self, args: I) -> Result<Option<String>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = run_git(&self.root, args, None)?;
        Ok(output
            .status
            .success()
            .then(|| stdout_line(&output).to_owned()))
    }

    fn empty_blob(&self) -> Result<BlobId> {
        if let Some(blob) = self.empty_blob.get() {
            return Ok(blob.clone());
        }
        let args = ["hash-object", "-w", "--stdin"];
        let output = run_git(&self.root, args, Some(b""))?;
        check(&output, args)?;
        let blob = parse_object_id::<BlobId>(stdout_line(&output))?;
        Ok(self.empty_blob.get_or_init(|| blob).clone())
    }
}

impl ChangeStore for GitCli {
    fn root(&self) -> &Path {
        &self.root
    }

    fn head(&self) -> Result<Option<CommitId>> {
        self.git_ok(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])?
            .map(|id| parse_object_id(&id))
            .transpose()
    }

    fn commit_exists(&self, id: &CommitId) -> Result<bool> {
        let spec = format!("{id}^{{commit}}");
        Ok(self.git_ok(["cat-file", "-e", spec.as_str()])?.is_some())
    }

    fn user_name(&self) -> Result<Option<String>> {
        Ok(self
            .git_ok(["config", "user.name"])?
            .filter(|name| !name.trim().is_empty()))
    }

    fn snapshot(&self, path: &RepoPath) -> Result<Option<BlobId>> {
        if !self.root.join(path.as_str()).is_file() {
            return Ok(None);
        }
        // hash-object applies the same filters (line endings, attributes) git uses when
        // staging, so the blob matches what a commit would contain.
        let out = self.git(["hash-object", "-w", "--", path.as_str()])?;
        parse_object_id(out.trim()).map(Some)
    }

    fn blob_at(&self, commit: &CommitId, path: &RepoPath) -> Result<Option<BlobId>> {
        let spec = format!("{commit}:{path}");
        let Some(id) = self.git_ok(["rev-parse", "--verify", "--quiet", spec.as_str()])? else {
            return Ok(None);
        };
        // A path that names a directory resolves to a tree; that isn't a file blob.
        let kind = self.git(["cat-file", "-t", id.as_str()])?;
        if kind.trim() == "blob" {
            parse_object_id(&id).map(Some)
        } else {
            Ok(None)
        }
    }

    fn blob_exists(&self, blob: &BlobId) -> Result<bool> {
        let spec = format!("{blob}^{{blob}}");
        Ok(self.git_ok(["cat-file", "-e", spec.as_str()])?.is_some())
    }

    fn diff_blobs(&self, old: Option<&BlobId>, new: Option<&BlobId>) -> Result<Vec<Hunk>> {
        if old == new {
            return Ok(Vec::new());
        }
        let empty = self.empty_blob()?;
        let old = old.unwrap_or(&empty);
        let new = new.unwrap_or(&empty);
        let out = self.git([
            "diff",
            "-U0",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            old.as_str(),
            new.as_str(),
        ])?;
        Ok(parse_hunk_headers(&out))
    }
}

fn run_git<I, S>(dir: &Path, args: I, stdin: Option<&[u8]>) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        // Stable, machine-readable output regardless of the user's config and locale.
        .args([
            "--no-pager",
            "-c",
            "core.quotepath=off",
            "-c",
            "color.ui=never",
        ])
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let spawn_error = |source: std::io::Error| {
        if source.kind() == std::io::ErrorKind::NotFound {
            Error::GitNotFound
        } else {
            Error::Io {
                path: PathBuf::from("git"),
                source,
            }
        }
    };
    let mut child = command.spawn().map_err(spawn_error)?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input).map_err(spawn_error)?;
        // Dropping the pipe closes it, so git sees end of input.
    }
    child.wait_with_output().map_err(spawn_error)
}

fn check<I, S>(output: &Output, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if output.status.success() {
        return Ok(());
    }
    let command = args
        .into_iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    Err(Error::Git {
        command: format!("git {command}"),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

fn stdout_line(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout)
        .unwrap_or_default()
        .trim()
}

fn parse_object_id<T>(text: &str) -> Result<T>
where
    T: TryFrom<String, Error = crate::model::InvalidObjectId>,
{
    T::try_from(text.to_owned()).map_err(|e| Error::Git {
        command: "git (object id)".into(),
        stderr: e.to_string(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    /// A throwaway repository with deterministic config, independent of the user's.
    pub(crate) struct TestRepo {
        pub dir: tempfile::TempDir,
    }

    impl TestRepo {
        pub fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let repo = Self { dir };
            repo.git(&["init", "-q", "-b", "main"]);
            repo.git(&["config", "user.name", "Test User"]);
            repo.git(&["config", "user.email", "test@example.invalid"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo.git(&["config", "core.autocrlf", "false"]);
            repo
        }

        pub fn path(&self) -> &Path {
            self.dir.path()
        }

        pub fn git(&self, args: &[&str]) -> String {
            let output = run_git(self.path(), args, None).unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }

        pub fn write(&self, path: &str, content: &str) {
            let full = self.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, content).unwrap();
        }

        pub fn commit_all(&self, message: &str) -> CommitId {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "-m", message]);
            CommitId::new(self.git(&["rev-parse", "HEAD"])).unwrap()
        }

        pub fn store(&self) -> GitCli {
            GitCli::discover(self.path()).unwrap()
        }
    }

    /// `n` numbered lines: "line 1\nline 2\n...".
    pub(crate) fn lines(n: u32) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    fn p(path: &str) -> RepoPath {
        RepoPath::new(path).unwrap()
    }

    #[test]
    fn discover_finds_the_root_from_a_subdirectory() {
        let repo = TestRepo::new();
        repo.write("src/deep/a.txt", "x\n");
        let git = GitCli::discover(repo.path().join("src/deep")).unwrap();
        assert_eq!(
            git.root().canonicalize().unwrap(),
            repo.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn discover_outside_a_repository_fails_clearly() {
        let dir = tempfile::tempdir().unwrap();
        // Guard against the temp dir itself living inside some repository.
        if run_git(dir.path(), ["rev-parse"], None)
            .unwrap()
            .status
            .success()
        {
            return;
        }
        assert!(matches!(
            GitCli::discover(dir.path()),
            Err(Error::NotARepository { .. })
        ));
    }

    #[test]
    fn head_is_none_before_the_first_commit() {
        let repo = TestRepo::new();
        let git = repo.store();
        assert_eq!(git.head().unwrap(), None);
        repo.write("a.txt", "x\n");
        let commit = repo.commit_all("first");
        assert_eq!(git.head().unwrap(), Some(commit.clone()));
        assert!(git.commit_exists(&commit).unwrap());
        assert!(
            !git.commit_exists(&CommitId::new("0".repeat(40)).unwrap())
                .unwrap()
        );
    }

    #[test]
    fn user_name_comes_from_git_config() {
        assert_eq!(
            TestRepo::new().store().user_name().unwrap().as_deref(),
            Some("Test User")
        );
    }

    #[test]
    fn snapshot_blob_at_and_diff_agree() {
        let repo = TestRepo::new();
        repo.write("src/a.txt", &lines(10));
        let base = repo.commit_all("base");
        let git = repo.store();

        // Replace line 3, insert two lines after line 7, delete line 10.
        let mut content: Vec<String> = lines(10).lines().map(str::to_owned).collect();
        content[2] = "changed 3".into();
        content.insert(7, "new a".into());
        content.insert(8, "new b".into());
        content.pop();
        repo.write("src/a.txt", &(content.join("\n") + "\n"));

        let hunks = git.changed_hunks(Some(&base), &p("src/a.txt")).unwrap();
        assert_eq!(
            hunks,
            vec![
                Hunk {
                    old_start: 3,
                    old_lines: 1,
                    new_start: 3,
                    new_lines: 1
                },
                Hunk {
                    old_start: 7,
                    old_lines: 0,
                    new_start: 8,
                    new_lines: 2
                },
                Hunk {
                    old_start: 10,
                    old_lines: 1,
                    new_start: 11,
                    new_lines: 0
                },
            ]
        );

        let committed = git.blob_at(&base, &p("src/a.txt")).unwrap().unwrap();
        let current = git.snapshot(&p("src/a.txt")).unwrap().unwrap();
        assert_ne!(committed, current);
        assert!(git.blob_exists(&current).unwrap());
        assert_eq!(
            git.diff_blob_to_worktree(&current, &p("src/a.txt"))
                .unwrap(),
            vec![]
        );
        assert_eq!(
            git.diff_blobs(Some(&committed), Some(&current)).unwrap(),
            hunks
        );
    }

    #[test]
    fn new_and_deleted_files_diff_against_empty() {
        let repo = TestRepo::new();
        repo.write("keep.txt", "x\n");
        repo.write("gone.txt", &lines(3));
        let base = repo.commit_all("base");
        let git = repo.store();

        repo.write("new.txt", &lines(4));
        assert_eq!(
            git.changed_hunks(Some(&base), &p("new.txt")).unwrap(),
            vec![Hunk {
                old_start: 0,
                old_lines: 0,
                new_start: 1,
                new_lines: 4
            }]
        );
        // Also with no commits at all.
        assert_eq!(git.changed_hunks(None, &p("new.txt")).unwrap().len(), 1);

        fs::remove_file(repo.path().join("gone.txt")).unwrap();
        assert_eq!(git.snapshot(&p("gone.txt")).unwrap(), None);
        assert_eq!(
            git.changed_hunks(Some(&base), &p("gone.txt")).unwrap(),
            vec![Hunk {
                old_start: 1,
                old_lines: 3,
                new_start: 0,
                new_lines: 0
            }]
        );
    }

    #[test]
    fn blob_at_ignores_directories_and_missing_paths() {
        let repo = TestRepo::new();
        repo.write("src/a.txt", "x\n");
        let base = repo.commit_all("base");
        let git = repo.store();
        assert_eq!(git.blob_at(&base, &p("src")).unwrap(), None);
        assert_eq!(git.blob_at(&base, &p("missing.txt")).unwrap(), None);
        assert!(
            !git.blob_exists(&BlobId::new("1".repeat(40)).unwrap())
                .unwrap()
        );
    }

    #[test]
    fn paths_with_spaces_and_unicode_work() {
        let repo = TestRepo::new();
        repo.write("dir with space/ünïcode.txt", &lines(2));
        let git = repo.store();
        let path = p("dir with space/ünïcode.txt");
        assert!(git.snapshot(&path).unwrap().is_some());
        assert_eq!(git.changed_hunks(None, &path).unwrap().len(), 1);
    }
}

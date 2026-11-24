//! The decision log's operations: record, review, link and ask "why".
//!
//! [`Concede`] ties the on-disk [`Store`] to a [`ChangeStore`]. The CLI, the MCP server
//! and (later) the app all go through it, so the rules live in one place.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use crate::change_store::ChangeStore;
use crate::error::{Error, Result};
use crate::git::GitCli;
use crate::id::{CaptureId, DecisionId};
use crate::linemap::{MappedRange, map_range};
use crate::log::{DecisionState, Log};
use crate::model::{
    AgentInfo, Alternative, BlobId, Capture, Change, CommitId, Decision, Event, Hunk, LineRange,
    RepoPath, ReviewStatus, Source,
};
use crate::store::{Loaded, Store};

/// The value written to `allowed_by` until the rules engine exists.
pub const UNCONFIGURED_RULE: &str = "unconfigured";

/// The decision log of one repository.
#[derive(Debug)]
pub struct Concede<C: ChangeStore = GitCli> {
    vcs: C,
    store: Store,
}

/// A file a decision touched, optionally narrowed to line ranges (current line numbers).
#[derive(Debug, Clone)]
pub struct FileRef {
    /// Absolute, or relative to the repository root.
    pub path: std::path::PathBuf,
    pub lines: Vec<LineRange>,
}

/// Input for [`Concede::record_decision`].
#[derive(Debug, Clone)]
pub struct NewDecision {
    pub source: Source,
    pub agent: Option<AgentInfo>,
    pub task: String,
    pub reasoning: String,
    pub alternatives: Vec<Alternative>,
    /// Defaults to [`UNCONFIGURED_RULE`] when `None`.
    pub allowed_by: Option<String>,
    pub files: Vec<FileRef>,
    /// Captures to claim explicitly, in addition to the automatic claims (ADR 0007).
    pub claim: Vec<CaptureId>,
}

/// Input for [`Concede::record_capture`].
#[derive(Debug, Clone)]
pub struct NewCapture {
    pub agent: AgentInfo,
    pub tool_name: String,
    /// Absolute, or relative to the repository root.
    pub path: std::path::PathBuf,
    /// Hunks reported by the editor, if it reported any. Ignored if they're invalid.
    pub hunks: Option<Vec<Hunk>>,
}

/// The result of [`Concede::review`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewOutcome {
    pub decision: DecisionId,
    /// Set when an unexplained capture was reviewed directly and wrapped in a new
    /// `source: hook` decision.
    pub wrapped_capture: Option<CaptureId>,
}

/// The result of [`Concede::link`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkOutcome {
    pub commit: CommitId,
    /// Decisions newly linked to the commit.
    pub linked: Vec<DecisionId>,
    /// Decisions that were already linked to it.
    pub already_linked: Vec<DecisionId>,
}

/// One recorded hunk and where its lines are now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappedHunk {
    pub recorded: Hunk,
    pub current: MappedRange,
}

/// What explains a change in a "why" answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhyRecord {
    Decision {
        decision: Box<Decision>,
        state: DecisionState,
    },
    Unexplained {
        capture: Box<Capture>,
    },
}

/// One decision or unexplained capture in a "why" answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhyEntry {
    pub record: WhyRecord,
    pub hunks: Vec<MappedHunk>,
}

/// The answer to "why does this file (or these lines) look like this?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Why {
    pub path: RepoPath,
    pub lines: Option<LineRange>,
    /// Oldest first.
    pub entries: Vec<WhyEntry>,
}

/// A record an id (or unique id prefix) resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Decision(DecisionId),
    Capture(CaptureId),
}

impl Concede<GitCli> {
    /// Open the decision log of the git repository containing `dir`.
    pub fn discover(dir: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::new(GitCli::discover(dir)?))
    }
}

impl<C: ChangeStore> Concede<C> {
    /// Use `vcs` for version control and `<root>/.concede/` for records.
    pub fn new(vcs: C) -> Self {
        let store = Store::for_repo(vcs.root());
        Self { vcs, store }
    }

    /// The version-control backend.
    pub fn vcs(&self) -> &C {
        &self.vcs
    }

    /// The record store.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Load every record (for the inbox and listing).
    pub fn load(&self) -> Result<Loaded> {
        self.store.load()
    }

    /// Turn a user- or agent-supplied path into a repository path.
    pub fn repo_path(&self, path: &Path) -> Result<RepoPath> {
        let relative = if path.is_absolute() {
            let absolute = canonicalize_lenient(path);
            let root = canonicalize_lenient(self.vcs.root());
            absolute
                .strip_prefix(&root)
                .map(Path::to_path_buf)
                .map_err(|_| Error::OutsideRepository {
                    path: path.to_path_buf(),
                })?
        } else {
            path.to_path_buf()
        };
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
                Component::CurDir => {}
                _ => {
                    return Err(Error::OutsideRepository {
                        path: path.to_path_buf(),
                    });
                }
            }
        }
        RepoPath::new(parts.join("/")).map_err(|_| Error::OutsideRepository {
            path: path.to_path_buf(),
        })
    }

    /// Resolve a full id or a unique, case-insensitive id prefix.
    pub fn resolve(&self, log: &Log, text: &str) -> Result<Target> {
        let wanted = text.trim().to_ascii_uppercase();
        if wanted.len() < 4 {
            return Err(Error::NotFound(text.to_owned()));
        }
        let mut matches: Vec<Target> = log
            .decisions()
            .filter(|d| d.id.to_string().starts_with(&wanted))
            .map(|d| Target::Decision(d.id))
            .collect();
        matches.extend(
            log.captures()
                .filter(|c| c.id.to_string().starts_with(&wanted))
                .map(|c| Target::Capture(c.id)),
        );
        match matches.len() {
            0 => Err(Error::NotFound(text.to_owned())),
            1 => Ok(matches[0]),
            count => Err(Error::Ambiguous {
                prefix: text.to_owned(),
                count,
            }),
        }
    }

    /// Record a hook capture (ADR 0007). Returns `None` when the path is outside the
    /// repository or the edit changed nothing.
    pub fn record_capture(&self, input: NewCapture) -> Result<Option<Capture>> {
        let path = match self.repo_path(&input.path) {
            Ok(path) => path,
            Err(Error::OutsideRepository { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let head = self.vcs.head()?;
        let after = self.vcs.snapshot(&path)?;

        let mut capture = Capture::new(
            input.agent,
            input.tool_name,
            Change {
                path: path.clone(),
                after_blob: after,
                hunks: input.hunks.unwrap_or_default(),
            },
        );
        capture.base_commit = head.clone();
        if capture.change.hunks.is_empty() || capture.validate().is_err() {
            capture.change.hunks = self.vcs.changed_hunks(head.as_ref(), &path)?;
        }
        if capture.change.hunks.is_empty() {
            return Ok(None);
        }
        self.store.write_capture(&capture)?;
        Ok(Some(capture))
    }

    /// Record a decision: snapshot the files, claim matching captures, and work out the
    /// hunks it explains (ADR 0007).
    pub fn record_decision(&self, input: NewDecision) -> Result<Decision> {
        let log = self.store.load()?.log;

        // Files named by the agent, with the line ranges it mentioned.
        let mut files: BTreeMap<RepoPath, Vec<LineRange>> = BTreeMap::new();
        for file in &input.files {
            files
                .entry(self.repo_path(&file.path)?)
                .or_default()
                .extend(&file.lines);
        }

        // Explicit claims bring their files along.
        let mut claims: BTreeSet<CaptureId> = BTreeSet::new();
        for id in &input.claim {
            let capture = log
                .capture(id)
                .ok_or_else(|| Error::NotFound(id.to_string()))?;
            if let Some(decision) = log.claimed_by(id) {
                return Err(Error::CaptureExplained {
                    capture: *id,
                    decision: decision.id,
                });
            }
            files.entry(capture.change.path.clone()).or_default();
            claims.insert(*id);
        }
        let paths: Vec<RepoPath> = files.keys().cloned().collect();
        let session = input.agent.as_ref().and_then(|a| a.session.as_deref());
        claims.extend(log.claimable_captures(&paths, session));
        let claimed: Vec<&Capture> = claims.iter().filter_map(|id| log.capture(id)).collect();

        // Diff against the state before the earliest claimed capture when known, so
        // hunks survive the agent committing in between; otherwise against HEAD.
        let base = match claimed.iter().find_map(|c| c.base_commit.clone()) {
            Some(commit) => Some(commit),
            None => self.vcs.head()?,
        };

        let mut changes = Vec::with_capacity(files.len());
        for (path, ranges) in files {
            let after = self.vcs.snapshot(&path)?;
            let base_blob = match &base {
                Some(commit) => self.vcs.blob_at(commit, &path)?,
                None => None,
            };
            let diff = self.vcs.diff_blobs(base_blob.as_ref(), after.as_ref())?;

            // Evidence of which hunks belong to this decision: the agent's ranges plus
            // the claimed captures' ranges, mapped to the current file.
            let mut evidence = ranges;
            for capture in claimed.iter().filter(|c| c.change.path == path) {
                for mapped in self.map_hunks(&capture.change, after.as_ref())? {
                    evidence.extend(mapped.current.range);
                }
            }
            let hunks = if evidence.is_empty() {
                diff
            } else {
                diff.into_iter()
                    .filter(|h| evidence.iter().any(|r| h.touches(*r)))
                    .collect()
            };
            changes.push(Change {
                path,
                after_blob: after,
                hunks,
            });
        }

        let mut decision = Decision::new(
            input.source,
            input.task,
            input
                .allowed_by
                .unwrap_or_else(|| UNCONFIGURED_RULE.to_owned()),
        );
        decision.agent = input.agent;
        decision.reasoning = input.reasoning;
        decision.alternatives = input.alternatives;
        decision.base_commit = base;
        decision.changes = changes;
        decision.captures = claims.into_iter().collect();
        self.store.write_decision(&decision)?;
        Ok(decision)
    }

    /// Review a decision, or an unexplained capture (which is wrapped in a new
    /// `source: hook` decision first). `target` is an id or unique id prefix.
    pub fn review(
        &self,
        target: &str,
        status: ReviewStatus,
        note: Option<String>,
        actor: Option<String>,
    ) -> Result<ReviewOutcome> {
        let log = self.store.load()?.log;
        let actor = self.actor(actor)?;
        let note = note.filter(|n| !n.trim().is_empty());
        let (decision, wrapped_capture) = match self.resolve(&log, target)? {
            Target::Decision(id) => (id, None),
            Target::Capture(id) => {
                if let Some(decision) = log.claimed_by(&id) {
                    return Err(Error::CaptureExplained {
                        capture: id,
                        decision: decision.id,
                    });
                }
                let capture = log
                    .capture(&id)
                    .ok_or_else(|| Error::NotFound(id.to_string()))?;
                let wrapper = wrap_capture(capture);
                self.store.write_decision(&wrapper)?;
                (wrapper.id, Some(id))
            }
        };
        self.store
            .write_event(&Event::review(decision, actor, status, note))?;
        Ok(ReviewOutcome {
            decision,
            wrapped_capture,
        })
    }

    /// Link decisions to the HEAD commit. With no targets, links every approved decision
    /// that isn't linked to any commit yet.
    pub fn link(&self, targets: &[String], actor: Option<String>) -> Result<LinkOutcome> {
        let log = self.store.load()?.log;
        let commit = self.vcs.head()?.ok_or(Error::NoCommits)?;
        let actor = self.actor(actor)?;

        let ids: Vec<DecisionId> = if targets.is_empty() {
            log.decisions()
                .filter(|d| {
                    let state = log.state(&d.id);
                    state.review == ReviewStatus::Approved && state.commits.is_empty()
                })
                .map(|d| d.id)
                .collect()
        } else {
            targets
                .iter()
                .map(|t| match self.resolve(&log, t)? {
                    Target::Decision(id) => Ok(id),
                    Target::Capture(id) => match log.claimed_by(&id) {
                        Some(decision) => Ok(decision.id),
                        None => Err(Error::NotFound(format!(
                            "{t} (an unexplained capture; review or explain it first)"
                        ))),
                    },
                })
                .collect::<Result<_>>()?
        };

        let mut outcome = LinkOutcome {
            commit: commit.clone(),
            linked: Vec::new(),
            already_linked: Vec::new(),
        };
        for id in ids {
            if log.state(&id).commits.contains(&commit) {
                outcome.already_linked.push(id);
            } else {
                self.store
                    .write_event(&Event::link(id, actor.clone(), commit.clone()))?;
                outcome.linked.push(id);
            }
        }
        Ok(outcome)
    }

    /// Which decisions and unexplained captures shaped `path`, optionally narrowed to
    /// `lines` (current line numbers). Recorded ranges are mapped onto the current file.
    pub fn why(&self, path: &Path, lines: Option<LineRange>) -> Result<Why> {
        let path = self.repo_path(path)?;
        let log = self.store.load()?.log;
        let current = self.vcs.snapshot(&path)?;

        let mut entries = Vec::new();
        for decision in log.decisions_for_file(&path) {
            let Some(change) = decision.change_for(&path) else {
                continue;
            };
            let hunks = self.map_hunks(change, current.as_ref())?;
            if touches(&hunks, lines) {
                let record = WhyRecord::Decision {
                    decision: Box::new(decision.clone()),
                    state: log.state(&decision.id),
                };
                entries.push(WhyEntry { record, hunks });
            }
        }
        for capture in log.unexplained_for_file(&path) {
            let hunks = self.map_hunks(&capture.change, current.as_ref())?;
            if touches(&hunks, lines) {
                let record = WhyRecord::Unexplained {
                    capture: Box::new(capture.clone()),
                };
                entries.push(WhyEntry { record, hunks });
            }
        }
        // Decisions and captures interleaved by time, oldest first.
        entries.sort_by_key(|e| match &e.record {
            WhyRecord::Decision { decision, .. } => decision.timestamp,
            WhyRecord::Unexplained { capture } => capture.timestamp,
        });
        Ok(Why {
            path,
            lines,
            entries,
        })
    }

    /// Map a recorded change's hunks onto `current` (the file's blob now).
    fn map_hunks(&self, change: &Change, current: Option<&BlobId>) -> Result<Vec<MappedHunk>> {
        let diff = match &change.after_blob {
            Some(blob) if self.vcs.blob_exists(blob)? => {
                Some(self.vcs.diff_blobs(Some(blob), current)?)
            }
            _ => None,
        };
        Ok(change
            .hunks
            .iter()
            .map(|&recorded| {
                let range = recorded.new_side();
                let current = match &diff {
                    Some(diff) => map_range(range, diff),
                    None => MappedRange::approximate(range),
                };
                MappedHunk { recorded, current }
            })
            .collect())
    }

    fn actor(&self, actor: Option<String>) -> Result<String> {
        match actor.filter(|a| !a.trim().is_empty()) {
            Some(actor) => Ok(actor),
            None => Ok(self
                .vcs
                .user_name()?
                .unwrap_or_else(|| "unknown".to_owned())),
        }
    }
}

/// Whether any mapped hunk is in `lines` (or always, without a line filter).
fn touches(hunks: &[MappedHunk], lines: Option<LineRange>) -> bool {
    match lines {
        None => true,
        Some(lines) => hunks
            .iter()
            .any(|h| h.current.range.is_some_and(|r| r.overlaps(lines))),
    }
}

/// A `source: hook` decision explaining one capture, for reviewing it directly.
fn wrap_capture(capture: &Capture) -> Decision {
    let mut decision = Decision::new(
        Source::Hook,
        format!("{} on {}", capture.tool_name, capture.change.path),
        UNCONFIGURED_RULE,
    );
    decision.agent = Some(capture.agent.clone());
    decision.base_commit = capture.base_commit.clone();
    decision.changes = vec![capture.change.clone()];
    decision.captures = vec![capture.id];
    decision
}

/// Canonicalise a path that may not exist (a deleted file): canonicalise the deepest
/// existing ancestor and append the rest.
fn canonicalize_lenient(path: &Path) -> std::path::PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => canonicalize_lenient(parent).join(name),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{TestRepo, lines};
    use crate::linemap::Mapping;

    fn agent(session: &str) -> AgentInfo {
        AgentInfo {
            tool: "claude-code".into(),
            model: Some("claude-opus-5-5".into()),
            session: Some(session.into()),
        }
    }

    fn capture(path: &str, session: &str) -> NewCapture {
        NewCapture {
            agent: agent(session),
            tool_name: "Edit".into(),
            path: path.into(),
            hunks: None,
        }
    }

    fn decision(task: &str, session: &str, files: &[&str]) -> NewDecision {
        NewDecision {
            source: Source::Agent,
            agent: Some(agent(session)),
            task: task.into(),
            reasoning: format!("reasoning for {task}"),
            alternatives: vec![],
            allowed_by: None,
            files: files
                .iter()
                .map(|f| FileRef {
                    path: f.into(),
                    lines: vec![],
                })
                .collect(),
            claim: vec![],
        }
    }

    /// Replace 1-based line `n` of `content`.
    fn replace_line(content: &str, n: usize, with: &str) -> String {
        let mut all: Vec<&str> = content.lines().collect();
        all[n - 1] = with;
        all.join("\n") + "\n"
    }

    fn setup() -> (TestRepo, Concede) {
        let repo = TestRepo::new();
        repo.write("src/Order.cs", &lines(30));
        repo.commit_all("base");
        let concede = Concede::discover(repo.path()).unwrap();
        (repo, concede)
    }

    fn range(start: u32, end: u32) -> LineRange {
        LineRange::new(start, end).unwrap()
    }

    #[test]
    fn capture_falls_back_to_git_diff_and_skips_no_op_edits() {
        let (repo, concede) = setup();
        assert!(
            concede
                .record_capture(capture("src/Order.cs", "s1"))
                .unwrap()
                .is_none()
        );

        repo.write("src/Order.cs", &replace_line(&lines(30), 10, "retry()"));
        let c = concede
            .record_capture(capture("src/Order.cs", "s1"))
            .unwrap()
            .unwrap();
        assert_eq!(
            c.change.hunks,
            vec![Hunk {
                old_start: 10,
                old_lines: 1,
                new_start: 10,
                new_lines: 1
            }]
        );
        assert!(c.base_commit.is_some());
        assert!(c.change.after_blob.is_some());
    }

    #[test]
    fn capture_uses_reported_hunks_when_valid_and_ignores_outside_paths() {
        let (repo, concede) = setup();
        repo.write("src/Order.cs", &replace_line(&lines(30), 10, "retry()"));
        let reported = Hunk {
            old_start: 10,
            old_lines: 1,
            new_start: 10,
            new_lines: 1,
        };
        let mut input = capture(&repo.path().join("src/Order.cs").to_string_lossy(), "s1");
        input.hunks = Some(vec![reported]);
        assert_eq!(
            concede.record_capture(input).unwrap().unwrap().change.hunks,
            vec![reported]
        );

        let outside = tempfile::tempdir().unwrap();
        let input = capture(&outside.path().join("x.cs").to_string_lossy(), "s1");
        assert!(concede.record_capture(input).unwrap().is_none());
    }

    #[test]
    fn decision_claims_its_sessions_captures_and_keeps_only_their_hunks() {
        let (repo, concede) = setup();
        // Session s1 edits line 5, session s2 edits line 25.
        let v1 = replace_line(&lines(30), 5, "s1 edit");
        repo.write("src/Order.cs", &v1);
        let mine = concede
            .record_capture(capture("src/Order.cs", "s1"))
            .unwrap()
            .unwrap();
        repo.write("src/Order.cs", &replace_line(&v1, 25, "s2 edit"));
        let theirs = concede
            .record_capture(capture("src/Order.cs", "s2"))
            .unwrap()
            .unwrap();

        let d = concede
            .record_decision(decision("retry", "s1", &["src/Order.cs"]))
            .unwrap();
        assert_eq!(d.captures, vec![mine.id]);
        assert_eq!(
            d.changes[0].hunks,
            vec![Hunk {
                old_start: 5,
                old_lines: 1,
                new_start: 5,
                new_lines: 1
            }]
        );

        let log = concede.load().unwrap().log;
        let unexplained: Vec<_> = log.inbox().unexplained.iter().map(|c| c.id).collect();
        assert_eq!(unexplained, vec![theirs.id]);
    }

    #[test]
    fn decision_without_captures_or_ranges_takes_the_whole_diff() {
        let (repo, concede) = setup();
        repo.write(
            "src/Order.cs",
            &replace_line(&replace_line(&lines(30), 3, "a"), 20, "b"),
        );
        let d = concede
            .record_decision(decision("tidy", "s1", &["src/Order.cs"]))
            .unwrap();
        assert_eq!(d.changes[0].hunks.len(), 2);
        assert!(d.captures.is_empty());
    }

    #[test]
    fn agent_ranges_narrow_the_hunks() {
        let (repo, concede) = setup();
        repo.write(
            "src/Order.cs",
            &replace_line(&replace_line(&lines(30), 3, "a"), 20, "b"),
        );
        let mut input = decision("only b", "s1", &[]);
        input.files = vec![FileRef {
            path: "src/Order.cs".into(),
            lines: vec![range(19, 21)],
        }];
        let d = concede.record_decision(input).unwrap();
        assert_eq!(
            d.changes[0]
                .hunks
                .iter()
                .map(|h| h.new_start)
                .collect::<Vec<_>>(),
            vec![20]
        );
    }

    #[test]
    fn hunks_survive_the_agent_committing_before_logging() {
        let (repo, concede) = setup();
        repo.write("src/Order.cs", &replace_line(&lines(30), 7, "x"));
        concede
            .record_capture(capture("src/Order.cs", "s1"))
            .unwrap()
            .unwrap();
        repo.commit_all("agent committed");
        let d = concede
            .record_decision(decision("x", "s1", &["src/Order.cs"]))
            .unwrap();
        assert_eq!(
            d.changes[0]
                .hunks
                .iter()
                .map(|h| h.new_start)
                .collect::<Vec<_>>(),
            vec![7]
        );
    }

    #[test]
    fn why_maps_ranges_through_later_edits() {
        let (repo, concede) = setup();
        let v1 = replace_line(&lines(30), 10, "retry()");
        repo.write("src/Order.cs", &v1);
        let d = concede
            .record_decision(decision("retry", "s1", &["src/Order.cs"]))
            .unwrap();

        let path = Path::new("src/Order.cs");
        let why = concede.why(path, Some(range(10, 10))).unwrap();
        assert_eq!(why.entries.len(), 1);
        assert_eq!(why.entries[0].hunks[0].current.mapping, Mapping::Unchanged);

        // Someone inserts 3 lines at the top: line 10 is now line 13.
        repo.write("src/Order.cs", &format!("a\nb\nc\n{v1}"));
        assert!(
            concede
                .why(path, Some(range(10, 10)))
                .unwrap()
                .entries
                .is_empty()
        );
        let why = concede.why(path, Some(range(13, 13))).unwrap();
        assert_eq!(why.entries.len(), 1);
        let hunk = why.entries[0].hunks[0];
        assert_eq!(hunk.current.range, Some(range(13, 13)));
        assert_eq!(hunk.current.mapping, Mapping::Unchanged);
        match &why.entries[0].record {
            WhyRecord::Decision { decision, state } => {
                assert_eq!(decision.id, d.id);
                assert_eq!(state.review, ReviewStatus::Unreviewed);
            }
            other => panic!("expected a decision, got {other:?}"),
        }

        // The line itself is edited: still found, marked modified.
        let v2 = replace_line(&format!("a\nb\nc\n{v1}"), 13, "retry(3)");
        repo.write("src/Order.cs", &v2);
        let why = concede.why(path, Some(range(13, 13))).unwrap();
        assert_eq!(why.entries[0].hunks[0].current.mapping, Mapping::Modified);
    }

    #[test]
    fn why_reports_unexplained_captures_and_approximate_ranges() {
        let (repo, concede) = setup();
        repo.write("src/Order.cs", &replace_line(&lines(30), 4, "loose"));
        concede
            .record_capture(capture("src/Order.cs", "s9"))
            .unwrap()
            .unwrap();

        // A decision whose blob is gone (as if garbage collected).
        let mut lost = Decision::new(Source::Human, "old", UNCONFIGURED_RULE);
        lost.reasoning = "from long ago".into();
        lost.changes = vec![Change {
            path: RepoPath::new("src/Order.cs").unwrap(),
            after_blob: Some(BlobId::new("1".repeat(40)).unwrap()),
            hunks: vec![Hunk {
                old_start: 20,
                old_lines: 1,
                new_start: 20,
                new_lines: 1,
            }],
        }];
        concede.store().write_decision(&lost).unwrap();

        let why = concede.why(Path::new("src/Order.cs"), None).unwrap();
        assert_eq!(why.entries.len(), 2);
        assert!(matches!(
            why.entries[0].record,
            WhyRecord::Unexplained { .. }
        ));
        assert_eq!(
            why.entries[1].hunks[0].current.mapping,
            Mapping::Approximate
        );
        assert_eq!(why.entries[1].hunks[0].current.range, Some(range(20, 20)));
    }

    #[test]
    fn review_by_prefix_and_direct_capture_review() {
        let (repo, concede) = setup();
        repo.write("src/Order.cs", &replace_line(&lines(30), 4, "x"));
        let d = concede
            .record_decision(decision("x", "s1", &["src/Order.cs"]))
            .unwrap();

        let prefix = &d.id.to_string()[..20];
        let out = concede
            .review(&prefix.to_lowercase(), ReviewStatus::Approved, None, None)
            .unwrap();
        assert_eq!(
            out,
            ReviewOutcome {
                decision: d.id,
                wrapped_capture: None
            }
        );
        let log = concede.load().unwrap().log;
        let state = log.state(&d.id);
        assert_eq!(state.review, ReviewStatus::Approved);
        assert_eq!(state.last_review.unwrap().actor, "Test User");

        // An unexplained capture reviewed directly gets wrapped in a hook decision.
        repo.write(
            "src/Order.cs",
            &replace_line(&replace_line(&lines(30), 4, "x"), 9, "y"),
        );
        let c = concede
            .record_capture(capture("src/Order.cs", "s2"))
            .unwrap()
            .unwrap();
        let out = concede
            .review(
                &c.id.to_string(),
                ReviewStatus::Rejected,
                Some("not wanted".into()),
                Some("robbert".into()),
            )
            .unwrap();
        assert_eq!(out.wrapped_capture, Some(c.id));
        let log = concede.load().unwrap().log;
        let wrapper = log.decision(&out.decision).unwrap();
        assert_eq!(wrapper.source, Source::Hook);
        assert_eq!(wrapper.captures, vec![c.id]);
        assert_eq!(log.state(&wrapper.id).review, ReviewStatus::Rejected);
        assert!(log.inbox().unexplained.is_empty());

        // Reviewing the capture again now points at its decision.
        assert!(matches!(
            concede.review(&c.id.to_string(), ReviewStatus::Approved, None, None),
            Err(Error::CaptureExplained { .. })
        ));
        assert!(matches!(
            concede.review("ZZZZZZ", ReviewStatus::Approved, None, None),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn link_defaults_to_approved_unlinked_decisions_and_is_idempotent() {
        let (repo, concede) = setup();
        repo.write("src/Order.cs", &replace_line(&lines(30), 4, "x"));
        let approved = concede
            .record_decision(decision("a", "s1", &["src/Order.cs"]))
            .unwrap();
        let pending = concede.record_decision(decision("b", "s1", &[])).unwrap();
        concede
            .review(&approved.id.to_string(), ReviewStatus::Approved, None, None)
            .unwrap();
        let commit = repo.commit_all("ship it");

        let out = concede.link(&[], None).unwrap();
        assert_eq!(out.commit, commit);
        assert_eq!(out.linked, vec![approved.id]);

        // Nothing left by default; explicit re-link is reported, not duplicated.
        assert!(concede.link(&[], None).unwrap().linked.is_empty());
        let again = concede
            .link(&[approved.id.to_string(), pending.id.to_string()], None)
            .unwrap();
        assert_eq!(again.already_linked, vec![approved.id]);
        assert_eq!(again.linked, vec![pending.id]);
        assert_eq!(
            concede.load().unwrap().log.state(&approved.id).commits,
            vec![commit]
        );
    }

    #[test]
    fn link_needs_a_commit() {
        let repo = TestRepo::new();
        let concede = Concede::discover(repo.path()).unwrap();
        assert!(matches!(concede.link(&[], None), Err(Error::NoCommits)));
    }

    #[test]
    fn repo_path_normalises_and_rejects_escapes() {
        let (repo, concede) = setup();
        let abs = repo.path().join("src/./Order.cs");
        assert_eq!(concede.repo_path(&abs).unwrap().as_str(), "src/Order.cs");
        assert_eq!(
            concede
                .repo_path(Path::new("./src/New.cs"))
                .unwrap()
                .as_str(),
            "src/New.cs"
        );
        assert!(concede.repo_path(Path::new("../x")).is_err());
        assert!(
            concede
                .repo_path(Path::new("/definitely/elsewhere.cs"))
                .is_err()
        );
    }
}

//! Record validation (ADR 0005).
//!
//! Structural rules (valid ids, paths, object ids) are enforced by the types during
//! deserialisation. The rules here are the ones that span several fields.

use std::collections::BTreeSet;

use crate::error::ValidationError;
use crate::model::{Capture, Change, Decision, Event, EventKind, Hunk, SCHEMA_VERSION, Source};

/// Collects problems so one validation pass reports all of them, not just the first.
#[derive(Default)]
struct Problems(Vec<String>);

impl Problems {
    fn check(&mut self, ok: bool, problem: impl FnOnce() -> String) {
        if !ok {
            self.0.push(problem());
        }
    }

    fn finish(self) -> Result<(), ValidationError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(ValidationError { problems: self.0 })
        }
    }
}

fn is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// Positions on one side of a diff, doubled so a zero-length hunk ("between line `start`
/// and the next") can sit at the half-line `2 * start + 1`. Returns (first, last).
fn span2(start: u32, lines: u32) -> (u64, u64) {
    let start = u64::from(start);
    if lines == 0 {
        (2 * start + 1, 2 * start + 1)
    } else {
        (2 * start, 2 * (start + u64::from(lines) - 1))
    }
}

fn check_hunk(p: &mut Problems, path: &str, h: &Hunk) {
    p.check(h.old_lines > 0 || h.new_lines > 0, || {
        format!("{path}: hunk at line {} changes nothing", h.new_start)
    });
    p.check(h.old_lines == 0 || h.old_start >= 1, || {
        format!("{path}: hunk old_start must be at least 1 when old_lines > 0")
    });
    p.check(h.new_lines == 0 || h.new_start >= 1, || {
        format!("{path}: hunk new_start must be at least 1 when new_lines > 0")
    });
}

fn check_change(p: &mut Problems, change: &Change) {
    let path = change.path.as_str();
    for hunk in &change.hunks {
        check_hunk(p, path, hunk);
    }
    for pair in change.hunks.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let ordered = span2(b.old_start, b.old_lines).0 > span2(a.old_start, a.old_lines).1
            && span2(b.new_start, b.new_lines).0 > span2(a.new_start, a.new_lines).1;
        p.check(ordered, || {
            format!(
                "{path}: hunks must be sorted and must not overlap (hunk at new line {} follows hunk at new line {})",
                b.new_start, a.new_start
            )
        });
    }
    p.check(
        change.after_blob.is_some() || change.hunks.iter().all(|h| h.new_lines == 0),
        || format!("{path}: after_blob may only be absent when the change deletes the file"),
    );
}

impl Decision {
    /// Check the cross-field rules from ADR 0005.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut p = Problems::default();
        p.check(self.schema == SCHEMA_VERSION, || {
            format!("schema must be {SCHEMA_VERSION}, found {}", self.schema)
        });
        p.check(!is_blank(&self.task), || "task must not be empty".into());
        p.check(!is_blank(&self.allowed_by), || {
            "allowed_by must not be empty".into()
        });
        p.check(
            self.source == Source::Hook || !is_blank(&self.reasoning),
            || "reasoning must not be empty (only hook records may omit it)".into(),
        );
        p.check(self.source == Source::Human || self.agent.is_some(), || {
            "agent is required unless source is human".into()
        });
        if let Some(agent) = &self.agent {
            p.check(!is_blank(&agent.tool), || {
                "agent.tool must not be empty".into()
            });
        }
        for alt in &self.alternatives {
            p.check(!is_blank(&alt.option), || {
                "alternative option must not be empty".into()
            });
        }
        let mut seen = BTreeSet::new();
        for change in &self.changes {
            p.check(seen.insert(&change.path), || {
                format!("{}: listed more than once in changes", change.path)
            });
            check_change(&mut p, change);
        }
        let unique_captures: BTreeSet<_> = self.captures.iter().collect();
        p.check(unique_captures.len() == self.captures.len(), || {
            "captures must not contain duplicates".into()
        });
        p.finish()
    }
}

impl Event {
    /// Check the event's own rules. Whether the decision exists is checked by the caller,
    /// since events may legitimately arrive before their decision (in a merge).
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut p = Problems::default();
        p.check(self.schema == SCHEMA_VERSION, || {
            format!("schema must be {SCHEMA_VERSION}, found {}", self.schema)
        });
        p.check(!is_blank(&self.actor), || "actor must not be empty".into());
        if let EventKind::Review {
            note: Some(note), ..
        } = &self.kind
        {
            p.check(!is_blank(note), || {
                "note must be omitted rather than empty".into()
            });
        }
        p.finish()
    }
}

impl Capture {
    /// Check the capture's rules.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut p = Problems::default();
        p.check(self.schema == SCHEMA_VERSION, || {
            format!("schema must be {SCHEMA_VERSION}, found {}", self.schema)
        });
        p.check(!is_blank(&self.agent.tool), || {
            "agent.tool must not be empty".into()
        });
        p.check(!is_blank(&self.tool_name), || {
            "tool_name must not be empty".into()
        });
        check_change(&mut p, &self.change);
        p.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentInfo, Alternative, BlobId, RepoPath, ReviewStatus};
    use std::time::SystemTime;

    fn blob() -> Option<BlobId> {
        Some(BlobId::new("a".repeat(40)).unwrap())
    }

    fn hunk(old_start: u32, old_lines: u32, new_start: u32, new_lines: u32) -> Hunk {
        Hunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
        }
    }

    fn change(path: &str, hunks: Vec<Hunk>) -> Change {
        Change {
            path: RepoPath::new(path).unwrap(),
            after_blob: blob(),
            hunks,
        }
    }

    fn agent_decision() -> Decision {
        let mut d = Decision::new(Source::Agent, "Add retry", "unconfigured");
        d.agent = Some(AgentInfo {
            tool: "claude-code".into(),
            model: None,
            session: None,
        });
        d.reasoning = "Transient failures".into();
        d.changes = vec![change("src/a.cs", vec![hunk(40, 3, 40, 11)])];
        d
    }

    fn problems(d: &Decision) -> Vec<String> {
        d.validate().err().map(|e| e.problems).unwrap_or_default()
    }

    #[test]
    fn valid_agent_decision_passes() {
        assert_eq!(problems(&agent_decision()), Vec::<String>::new());
    }

    #[test]
    fn decision_without_changes_is_allowed() {
        let mut d = agent_decision();
        d.changes.clear();
        assert!(d.validate().is_ok());
    }

    #[test]
    fn reports_every_problem_at_once() {
        let mut d = agent_decision();
        d.task = "  ".into();
        d.reasoning.clear();
        d.allowed_by.clear();
        d.agent = None;
        assert_eq!(problems(&d).len(), 4, "{:?}", problems(&d));
    }

    #[test]
    fn hook_decision_may_omit_reasoning_but_human_needs_no_agent() {
        let mut hook = agent_decision();
        hook.source = Source::Hook;
        hook.reasoning.clear();
        assert!(hook.validate().is_ok());

        let mut human = agent_decision();
        human.source = Source::Human;
        human.agent = None;
        assert!(human.validate().is_ok());
    }

    #[test]
    fn rejects_overlapping_or_unsorted_hunks() {
        let mut d = agent_decision();
        d.changes = vec![change("a", vec![hunk(10, 5, 10, 5), hunk(12, 1, 12, 1)])];
        assert!(d.validate().is_err());
        d.changes = vec![change("a", vec![hunk(20, 1, 20, 1), hunk(10, 1, 10, 1)])];
        assert!(d.validate().is_err());
    }

    #[test]
    fn adjacent_hunks_and_top_of_file_insertions_are_fine() {
        let mut d = agent_decision();
        d.changes = vec![change(
            "a",
            vec![
                hunk(0, 0, 1, 2),  // inserted 2 lines at the top
                hunk(1, 1, 3, 1),  // replaced old line 1
                hunk(5, 0, 8, 3),  // inserted after old line 5 (new line 7)
                hunk(6, 2, 10, 0), // deleted old lines 6-7
            ],
        )];
        assert_eq!(problems(&d), Vec::<String>::new());
    }

    #[test]
    fn rejects_two_insertions_at_the_same_spot() {
        let mut d = agent_decision();
        d.changes = vec![change("a", vec![hunk(5, 0, 6, 1), hunk(5, 0, 7, 1)])];
        assert!(d.validate().is_err());
    }

    #[test]
    fn rejects_empty_hunk_and_zero_start_with_lines() {
        let mut d = agent_decision();
        d.changes = vec![change("a", vec![hunk(5, 0, 5, 0)])];
        assert!(d.validate().is_err());
        d.changes = vec![change("a", vec![hunk(0, 1, 1, 1)])];
        assert!(d.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_paths_and_captures() {
        let mut d = agent_decision();
        d.changes.push(change("src/a.cs", vec![]));
        assert!(d.validate().is_err());

        let mut d = agent_decision();
        let c = crate::id::CaptureId::new_at(SystemTime::now());
        d.captures = vec![c, c];
        assert!(d.validate().is_err());
    }

    #[test]
    fn missing_blob_only_allowed_for_file_deletion() {
        let mut d = agent_decision();
        d.changes[0].after_blob = None;
        assert!(d.validate().is_err());
        d.changes[0].hunks = vec![hunk(1, 20, 0, 0)];
        assert!(d.validate().is_ok());
    }

    #[test]
    fn rejects_blank_alternative() {
        let mut d = agent_decision();
        d.alternatives = vec![Alternative {
            option: "".into(),
            rejected_because: "x".into(),
        }];
        assert!(d.validate().is_err());
    }

    #[test]
    fn event_and_capture_rules() {
        let id = crate::id::DecisionId::new_at(SystemTime::now());
        assert!(
            Event::review(id, "robbert", ReviewStatus::Approved, None)
                .validate()
                .is_ok()
        );
        assert!(
            Event::review(id, " ", ReviewStatus::Approved, None)
                .validate()
                .is_err()
        );
        assert!(
            Event::review(id, "r", ReviewStatus::Rejected, Some("".into()))
                .validate()
                .is_err()
        );

        let agent = AgentInfo {
            tool: "claude-code".into(),
            model: None,
            session: None,
        };
        let capture = Capture::new(agent, "Edit", change("a", vec![hunk(1, 1, 1, 1)]));
        assert!(capture.validate().is_ok());
        let mut bad = capture.clone();
        bad.tool_name.clear();
        assert!(bad.validate().is_err());
    }
}

//! Queries over a loaded decision log: per-file and per-line history, review state, the
//! inbox, and which captures a new decision should claim (ADR 0005, ADR 0007).
//!
//! Line ranges here are the ranges as recorded. Mapping them onto the current file when
//! the code has moved since (line drift, ADR 0006) is layered on top using the
//! `ChangeStore`.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::id::{CaptureId, DecisionId};
use crate::model::{
    Capture, CommitId, Decision, Event, EventKind, Hunk, LineRange, RepoPath, ReviewStatus,
};

/// An in-memory view of every record in `.concede/`.
#[derive(Debug, Clone, Default)]
pub struct Log {
    /// Keyed by id, so iteration is in creation order.
    decisions: BTreeMap<DecisionId, Decision>,
    /// Sorted by id, i.e. creation order, which is the order state is folded in.
    events: Vec<Event>,
    captures: BTreeMap<CaptureId, Capture>,
}

/// The current state of a decision, folded from its events.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecisionState {
    /// Status from the latest review event, or `Unreviewed`.
    pub review: ReviewStatus,
    /// The latest review event, if any (who reviewed, when, and their note).
    pub last_review: Option<Event>,
    /// Every commit the decision was linked to, in link order, without duplicates.
    pub commits: Vec<CommitId>,
}

/// A decision that touched the queried lines, with the hunks that did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionHit<'a> {
    pub decision: &'a Decision,
    pub hunks: Vec<Hunk>,
}

/// An unclaimed capture that touched the queried lines, with the hunks that did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureHit<'a> {
    pub capture: &'a Capture,
    pub hunks: Vec<Hunk>,
}

/// Unreviewed decisions from one agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGroup<'a> {
    /// The session id, or `None` for decisions without one.
    pub session: Option<&'a str>,
    pub decisions: Vec<&'a Decision>,
}

/// How a change story was grouped (ADR 0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryKey<'a> {
    /// Decisions sharing a recorded `intent` label — grouped even across sessions.
    Intent(&'a str),
    /// No intent recorded: grouped by the agent session instead.
    Session(&'a str),
    /// Neither an intent nor a session.
    Ungrouped,
}

/// The aggregate review state of a story's decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryStatus {
    /// Every decision is still unreviewed.
    Unreviewed,
    /// Some decisions reviewed, some not.
    Mixed,
    /// Every decision has been reviewed (approved or rejected).
    Reviewed,
}

/// How many of a story's decisions sit in each review state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoryReview {
    pub unreviewed: usize,
    pub approved: usize,
    pub rejected: usize,
}

impl StoryReview {
    /// Total decisions counted (always ≥ 1 for a story).
    pub fn total(&self) -> usize {
        self.unreviewed + self.approved + self.rejected
    }

    /// The aggregate status: all-unreviewed, all-reviewed, or mixed.
    pub fn status(&self) -> StoryStatus {
        if self.unreviewed == 0 {
            StoryStatus::Reviewed
        } else if self.unreviewed == self.total() {
            StoryStatus::Unreviewed
        } else {
            StoryStatus::Mixed
        }
    }
}

/// A change story: decisions that serve one intent, with the files they touched and
/// their aggregate review state (ADR 0011).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Story<'a> {
    pub key: StoryKey<'a>,
    /// In creation order.
    pub decisions: Vec<&'a Decision>,
    /// Union of touched paths, sorted and deduplicated.
    pub files: Vec<RepoPath>,
    pub review: StoryReview,
}

impl Story<'_> {
    /// The aggregate review status of the story.
    pub fn status(&self) -> StoryStatus {
        self.review.status()
    }
}

/// What still needs a human: unreviewed decisions, and changes nobody explained.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inbox<'a> {
    /// Grouped by session, oldest session first.
    pub sessions: Vec<SessionGroup<'a>>,
    /// Captures no decision claims, oldest first.
    pub unexplained: Vec<&'a Capture>,
}

impl Inbox<'_> {
    /// Whether there is nothing to review.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.unexplained.is_empty()
    }
}

impl Log {
    /// Build a log from loaded records (in any order).
    pub fn new(decisions: Vec<Decision>, mut events: Vec<Event>, captures: Vec<Capture>) -> Self {
        events.sort_by_key(|e| e.id);
        Self {
            decisions: decisions.into_iter().map(|d| (d.id, d)).collect(),
            events,
            captures: captures.into_iter().map(|c| (c.id, c)).collect(),
        }
    }

    /// One decision by id.
    pub fn decision(&self, id: &DecisionId) -> Option<&Decision> {
        self.decisions.get(id)
    }

    /// All decisions, oldest first.
    pub fn decisions(&self) -> impl Iterator<Item = &Decision> {
        self.decisions.values()
    }

    /// One capture by id.
    pub fn capture(&self, id: &CaptureId) -> Option<&Capture> {
        self.captures.get(id)
    }

    /// All captures, oldest first.
    pub fn captures(&self) -> impl Iterator<Item = &Capture> {
        self.captures.values()
    }

    /// All events, oldest first.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Events whose decision is not in the log (e.g. it arrives in a later merge).
    pub fn orphan_events(&self) -> impl Iterator<Item = &Event> {
        self.events
            .iter()
            .filter(|e| !self.decisions.contains_key(&e.decision))
    }

    /// Fold a decision's events into its current state.
    pub fn state(&self, id: &DecisionId) -> DecisionState {
        let mut state = DecisionState::default();
        for event in self.events.iter().filter(|e| &e.decision == id) {
            match &event.kind {
                EventKind::Review { status, .. } => {
                    state.review = *status;
                    state.last_review = Some(event.clone());
                }
                EventKind::Link { commit } => {
                    if !state.commits.contains(commit) {
                        state.commits.push(commit.clone());
                    }
                }
            }
        }
        state
    }

    /// Decisions that touched `path`, oldest first.
    pub fn decisions_for_file(&self, path: &RepoPath) -> Vec<&Decision> {
        self.decisions()
            .filter(|d| d.change_for(path).is_some())
            .collect()
    }

    /// Decisions whose recorded hunks in `path` touch `lines`, oldest first.
    pub fn decisions_for_lines(&self, path: &RepoPath, lines: LineRange) -> Vec<DecisionHit<'_>> {
        self.decisions()
            .filter_map(|decision| {
                let hunks = touching(&decision.change_for(path)?.hunks, lines);
                (!hunks.is_empty()).then_some(DecisionHit { decision, hunks })
            })
            .collect()
    }

    /// The ids of every capture some decision claims.
    pub fn claimed_captures(&self) -> BTreeSet<CaptureId> {
        self.decisions()
            .flat_map(|d| d.captures.iter().copied())
            .collect()
    }

    /// The decision that claims a capture, if any.
    pub fn claimed_by(&self, capture: &CaptureId) -> Option<&Decision> {
        self.decisions().find(|d| d.captures.contains(capture))
    }

    /// Captures no decision claims, oldest first.
    pub fn unclaimed_captures(&self) -> Vec<&Capture> {
        let claimed = self.claimed_captures();
        self.captures()
            .filter(|c| !claimed.contains(&c.id))
            .collect()
    }

    /// Unclaimed captures of `path`, oldest first. Claimed ones are reached through the
    /// decision that claims them.
    pub fn unexplained_for_file(&self, path: &RepoPath) -> Vec<&Capture> {
        self.unclaimed_captures()
            .into_iter()
            .filter(|c| &c.change.path == path)
            .collect()
    }

    /// Unclaimed captures whose hunks in `path` touch `lines`, oldest first.
    pub fn unexplained_for_lines(&self, path: &RepoPath, lines: LineRange) -> Vec<CaptureHit<'_>> {
        self.unexplained_for_file(path)
            .into_iter()
            .filter_map(|capture| {
                let hunks = touching(&capture.change.hunks, lines);
                (!hunks.is_empty()).then_some(CaptureHit { capture, hunks })
            })
            .collect()
    }

    /// The captures a new decision touching `paths` should claim (ADR 0007).
    ///
    /// For each path, among unclaimed captures of that path:
    /// - captures from the same session (when `session` is given) are always claimed;
    /// - captures with no session id, or any capture when `session` is `None`, are claimed
    ///   only if they are newer than the last decision that touched the path.
    ///
    /// Captures from a *different* known session are never claimed when `session` is
    /// given, so two agents working on the same file don't take each other's edits.
    pub fn claimable_captures(&self, paths: &[RepoPath], session: Option<&str>) -> Vec<CaptureId> {
        let paths: BTreeSet<&RepoPath> = paths.iter().collect();
        let last_decision: BTreeMap<&RepoPath, DateTime<Utc>> = paths
            .iter()
            .filter_map(|&path| {
                let latest = self
                    .decisions_for_file(path)
                    .iter()
                    .map(|d| d.timestamp)
                    .max()?;
                Some((path, latest))
            })
            .collect();

        self.unclaimed_captures()
            .into_iter()
            .filter(|c| paths.contains(&c.change.path))
            .filter(|c| {
                let newer = last_decision
                    .get(&c.change.path)
                    .is_none_or(|&last| c.timestamp > last);
                match (session, c.agent.session.as_deref()) {
                    (Some(wanted), Some(theirs)) => wanted == theirs,
                    _ => newer,
                }
            })
            .map(|c| c.id)
            .collect()
    }

    /// Unreviewed decisions grouped by session, plus unexplained captures.
    pub fn inbox(&self) -> Inbox<'_> {
        let mut groups: Vec<SessionGroup<'_>> = Vec::new();
        // Decisions iterate oldest first, so groups come out ordered by their oldest
        // unreviewed decision, and decisions within a group stay in order.
        for decision in self.decisions() {
            if self.state(&decision.id).review != ReviewStatus::Unreviewed {
                continue;
            }
            let session = decision.session();
            match groups.iter_mut().find(|g| g.session == session) {
                Some(group) => group.decisions.push(decision),
                None => groups.push(SessionGroup {
                    session,
                    decisions: vec![decision],
                }),
            }
        }
        Inbox {
            sessions: groups,
            unexplained: self.unclaimed_captures(),
        }
    }

    /// All decisions grouped into change stories (ADR 0011): by recorded `intent`
    /// when present, else by session, else ungrouped. Decisions sharing an intent
    /// group together even across sessions. Stories come out ordered by their
    /// earliest decision (first-touch), since decisions iterate oldest first, and
    /// decisions within a story stay in creation order.
    pub fn stories(&self) -> Vec<Story<'_>> {
        let mut stories: Vec<Story<'_>> = Vec::new();
        for decision in self.decisions() {
            let key = match (decision.intent(), decision.session()) {
                (Some(intent), _) => StoryKey::Intent(intent),
                (None, Some(session)) => StoryKey::Session(session),
                (None, None) => StoryKey::Ungrouped,
            };
            // Index-based lookup so the (immutable) search borrow is released before
            // a push — the first-seen order of keys gives the earliest-touch order.
            let index = match stories.iter().position(|s| s.key == key) {
                Some(index) => index,
                None => {
                    stories.push(Story {
                        key,
                        decisions: Vec::new(),
                        files: Vec::new(),
                        review: StoryReview::default(),
                    });
                    stories.len() - 1
                }
            };
            let story = &mut stories[index];
            story.decisions.push(decision);
            match self.state(&decision.id).review {
                ReviewStatus::Unreviewed => story.review.unreviewed += 1,
                ReviewStatus::Approved => story.review.approved += 1,
                ReviewStatus::Rejected => story.review.rejected += 1,
            }
        }

        for story in &mut stories {
            let files: BTreeSet<RepoPath> = story
                .decisions
                .iter()
                .flat_map(|d| d.changes.iter().map(|c| c.path.clone()))
                .collect();
            story.files = files.into_iter().collect();
        }
        stories
    }
}

fn touching(hunks: &[Hunk], lines: LineRange) -> Vec<Hunk> {
    hunks.iter().filter(|h| h.touches(lines)).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentInfo, BlobId, Change, Source};
    use chrono::TimeDelta;

    fn path(p: &str) -> RepoPath {
        RepoPath::new(p).unwrap()
    }

    fn agent(session: Option<&str>) -> AgentInfo {
        AgentInfo {
            tool: "claude-code".into(),
            model: None,
            session: session.map(Into::into),
        }
    }

    fn change(p: &str, hunks: &[(u32, u32)]) -> Change {
        Change {
            path: path(p),
            after_blob: Some(BlobId::new("a".repeat(40)).unwrap()),
            hunks: hunks
                .iter()
                .map(|&(new_start, new_lines)| Hunk {
                    old_start: new_start,
                    old_lines: 1,
                    new_start,
                    new_lines,
                })
                .collect(),
        }
    }

    fn decision(session: Option<&str>, changes: Vec<Change>) -> Decision {
        let mut d = Decision::new(Source::Agent, "task", "unconfigured");
        d.agent = Some(agent(session));
        d.reasoning = "because".into();
        d.changes = changes;
        d
    }

    fn capture(session: Option<&str>, p: &str, hunks: &[(u32, u32)]) -> Capture {
        Capture::new(agent(session), "Edit", change(p, hunks))
    }

    fn commit(c: char) -> CommitId {
        CommitId::new(c.to_string().repeat(40)).unwrap()
    }

    #[test]
    fn queries_by_file_and_line_range() {
        let a = decision(None, vec![change("src/a.cs", &[(10, 5), (40, 2)])]); // 10-14, 40-41
        let b = decision(
            None,
            vec![
                change("src/a.cs", &[(12, 1)]),
                change("src/b.cs", &[(1, 1)]),
            ],
        );
        let c = decision(None, vec![change("src/b.cs", &[(100, 3)])]);
        let log = Log::new(vec![c.clone(), a.clone(), b.clone()], vec![], vec![]);

        let ids =
            |hits: Vec<DecisionHit<'_>>| hits.iter().map(|h| h.decision.id).collect::<Vec<_>>();
        let lines = |s, e| LineRange::new(s, e).unwrap();

        let for_a: Vec<_> = log
            .decisions_for_file(&path("src/a.cs"))
            .iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(for_a, vec![a.id, b.id]);
        assert!(log.decisions_for_file(&path("src/none.cs")).is_empty());

        assert_eq!(
            ids(log.decisions_for_lines(&path("src/a.cs"), lines(12, 12))),
            vec![a.id, b.id]
        );
        assert_eq!(
            ids(log.decisions_for_lines(&path("src/a.cs"), lines(14, 39))),
            vec![a.id]
        );
        assert!(
            log.decisions_for_lines(&path("src/a.cs"), lines(15, 39))
                .is_empty()
        );

        // Only the touching hunks are returned.
        let hits = log.decisions_for_lines(&path("src/a.cs"), lines(41, 41));
        assert_eq!(hits[0].hunks, vec![a.changes[0].hunks[1]]);
    }

    #[test]
    fn review_flow_latest_review_wins_and_links_accumulate() {
        let d = decision(None, vec![]);
        let id = d.id;
        let mut log = Log::new(vec![d.clone()], vec![], vec![]);
        assert_eq!(log.state(&id), DecisionState::default());

        let approve = Event::review(id, "robbert", ReviewStatus::Approved, None);
        let link1 = Event::link(id, "robbert", commit('1'));
        let reject = Event::review(
            id,
            "robbert",
            ReviewStatus::Rejected,
            Some("broke tests".into()),
        );
        let link2 = Event::link(id, "robbert", commit('2'));
        let link1_again = Event::link(id, "robbert", commit('1'));
        // Shuffled input: folding must follow id order, not input order.
        let events = vec![
            link1_again.clone(),
            reject.clone(),
            link2.clone(),
            approve.clone(),
            link1.clone(),
        ];
        log = Log::new(vec![d], events, vec![]);

        let state = log.state(&id);
        assert_eq!(state.review, ReviewStatus::Rejected);
        assert_eq!(state.last_review.as_ref().map(|e| e.id), Some(reject.id));
        assert_eq!(state.commits, vec![commit('1'), commit('2')]);
    }

    #[test]
    fn orphan_events_are_reported() {
        let ghost = decision(None, vec![]);
        let event = Event::review(ghost.id, "robbert", ReviewStatus::Approved, None);
        let log = Log::new(vec![], vec![event.clone()], vec![]);
        assert_eq!(
            log.orphan_events().map(|e| e.id).collect::<Vec<_>>(),
            vec![event.id]
        );
    }

    #[test]
    fn inbox_groups_unreviewed_by_session_and_lists_unexplained_captures() {
        let s1a = decision(Some("s1"), vec![]);
        let s2 = decision(Some("s2"), vec![]);
        let s1b = decision(Some("s1"), vec![]);
        let none = decision(None, vec![]);
        let reviewed = decision(Some("s1"), vec![]);
        let claimed = capture(Some("s1"), "a", &[(1, 1)]);
        let loose = capture(Some("s3"), "b", &[(1, 1)]);
        let mut claimer = decision(Some("s1"), vec![]);
        claimer.captures = vec![claimed.id];

        let events = vec![
            Event::review(reviewed.id, "r", ReviewStatus::Approved, None),
            Event::review(claimer.id, "r", ReviewStatus::Rejected, None),
        ];
        let log = Log::new(
            vec![
                s1a.clone(),
                s2.clone(),
                s1b.clone(),
                none.clone(),
                reviewed,
                claimer,
            ],
            events,
            vec![claimed, loose.clone()],
        );

        let inbox = log.inbox();
        let shape: Vec<(Option<&str>, Vec<DecisionId>)> = inbox
            .sessions
            .iter()
            .map(|g| (g.session, g.decisions.iter().map(|d| d.id).collect()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (Some("s1"), vec![s1a.id, s1b.id]),
                (Some("s2"), vec![s2.id]),
                (None, vec![none.id]),
            ]
        );
        assert_eq!(
            inbox.unexplained.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![loose.id]
        );
        assert!(!inbox.is_empty());
        assert!(Log::default().inbox().is_empty());
    }

    #[test]
    fn unexplained_changes_are_queryable_by_lines() {
        let loose = capture(None, "src/a.cs", &[(20, 3)]);
        let log = Log::new(vec![], vec![], vec![loose.clone()]);
        let hits = log.unexplained_for_lines(&path("src/a.cs"), LineRange::line(22).unwrap());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].capture.id, loose.id);
        assert!(
            log.unexplained_for_lines(&path("src/a.cs"), LineRange::line(23).unwrap())
                .is_empty()
        );
    }

    #[test]
    fn claims_same_session_captures_and_never_another_sessions() {
        let mine = capture(Some("s1"), "a", &[(1, 1)]);
        let theirs = capture(Some("s2"), "a", &[(5, 1)]);
        let other_file = capture(Some("s1"), "b", &[(1, 1)]);
        let log = Log::new(vec![], vec![], vec![mine.clone(), theirs, other_file]);

        assert_eq!(
            log.claimable_captures(&[path("a")], Some("s1")),
            vec![mine.id]
        );
    }

    #[test]
    fn without_session_claims_only_captures_since_the_last_decision_on_that_file() {
        let mut old = capture(None, "a", &[(1, 1)]);
        let mut previous = decision(None, vec![change("a", &[(1, 1)])]);
        let mut fresh = capture(Some("s9"), "a", &[(3, 1)]);
        let mut sessionless = capture(None, "a", &[(4, 1)]);
        let t0 = previous.timestamp;
        old.timestamp = t0 - TimeDelta::seconds(10);
        previous.timestamp = t0;
        fresh.timestamp = t0 + TimeDelta::seconds(10);
        sessionless.timestamp = t0 + TimeDelta::seconds(20);
        let log = Log::new(
            vec![previous],
            vec![],
            vec![old, fresh.clone(), sessionless.clone()],
        );

        // No session on the decision: anything newer than the last decision on the file.
        assert_eq!(
            log.claimable_captures(&[path("a")], None),
            vec![fresh.id, sessionless.id]
        );
        // With a session: its own captures, plus newer captures that have no session id.
        assert_eq!(
            log.claimable_captures(&[path("a")], Some("s1")),
            vec![sessionless.id]
        );
    }

    #[test]
    fn already_claimed_captures_are_not_claimed_again() {
        let cap = capture(Some("s1"), "a", &[(1, 1)]);
        let mut d = decision(Some("s1"), vec![]);
        d.captures = vec![cap.id];
        let log = Log::new(vec![d], vec![], vec![cap]);
        assert!(log.claimable_captures(&[path("a")], Some("s1")).is_empty());
    }

    /// Set the intent on a decision (the test helper leaves it unset).
    fn with_intent(mut d: Decision, intent: &str) -> Decision {
        d.intent = Some(intent.to_owned());
        d
    }

    #[test]
    fn stories_group_by_intent_then_session_then_ungrouped() {
        // Same intent in two different sessions → one story; a sessionful decision
        // without intent → its own story; neither → the ungrouped story.
        let login_a = with_intent(
            decision(Some("s1"), vec![change("src/Login.cs", &[(1, 2)])]),
            "login",
        );
        let login_b = with_intent(
            decision(Some("s2"), vec![change("src/Auth.cs", &[(5, 1)])]),
            "login",
        );
        let tidy = decision(Some("s1"), vec![change("src/Login.cs", &[(9, 1)])]);
        let orphan = decision(None, vec![change("src/Misc.cs", &[(1, 1)])]);

        // Shuffled into the log; stories() re-derives creation order internally.
        let log = Log::new(
            vec![
                tidy.clone(),
                orphan.clone(),
                login_b.clone(),
                login_a.clone(),
            ],
            vec![],
            vec![],
        );
        let stories = log.stories();

        assert_eq!(stories.len(), 3);
        // Ordered by earliest-touch: login (login_a) first, then s1 (tidy), then orphan.
        assert_eq!(stories[0].key, StoryKey::Intent("login"));
        assert_eq!(
            stories[0]
                .decisions
                .iter()
                .map(|d| d.id)
                .collect::<Vec<_>>(),
            vec![login_a.id, login_b.id]
        );
        // Files are the de-duplicated, sorted union across the story's decisions.
        assert_eq!(
            stories[0].files,
            vec![path("src/Auth.cs"), path("src/Login.cs")]
        );

        assert_eq!(stories[1].key, StoryKey::Session("s1"));
        assert_eq!(stories[1].decisions, vec![&tidy]);

        assert_eq!(stories[2].key, StoryKey::Ungrouped);
        assert_eq!(stories[2].decisions, vec![&orphan]);
    }

    #[test]
    fn story_status_aggregates_decision_reviews() {
        let first = with_intent(decision(Some("s1"), vec![]), "cleanup");
        let second = with_intent(decision(Some("s1"), vec![]), "cleanup");

        // Both unreviewed → Unreviewed.
        let log = Log::new(vec![first.clone(), second.clone()], vec![], vec![]);
        assert_eq!(log.stories()[0].status(), StoryStatus::Unreviewed);

        // One approved, one not → Mixed.
        let approve = Event::review(first.id, "me", ReviewStatus::Approved, None);
        let log = Log::new(
            vec![first.clone(), second.clone()],
            vec![approve.clone()],
            vec![],
        );
        let story = &log.stories()[0];
        assert_eq!(story.status(), StoryStatus::Mixed);
        assert_eq!(story.review.approved, 1);
        assert_eq!(story.review.unreviewed, 1);

        // Both reviewed (one approved, one rejected) → Reviewed.
        let reject = Event::review(second.id, "me", ReviewStatus::Rejected, None);
        let log = Log::new(vec![first, second], vec![approve, reject], vec![]);
        let story = &log.stories()[0];
        assert_eq!(story.status(), StoryStatus::Reviewed);
        assert_eq!(story.review.total(), 2);
    }
}

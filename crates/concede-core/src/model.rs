//! The decision log's record types (ADR 0005, ADR 0007).
//!
//! These types are the on-disk format. Field order here is the field order in the JSON
//! files, so keep it stable to keep diffs small.

use std::borrow::Cow;
use std::fmt;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::id::{CaptureId, DecisionId, EventId};

/// The schema version this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// The current time, truncated to milliseconds so timestamps match the ULID precision and
/// stay short in the JSON files.
pub(crate) fn now() -> (DateTime<Utc>, SystemTime) {
    let now = Utc::now();
    let truncated = DateTime::from_timestamp_millis(now.timestamp_millis()).unwrap_or(now);
    (truncated, truncated.into())
}

// ---------------------------------------------------------------------------------------
// Paths and git object ids
// ---------------------------------------------------------------------------------------

/// A path relative to the repository root, always with `/` separators.
///
/// Validated on construction and on deserialisation, so a `RepoPath` in memory is always
/// well-formed: relative, no `..` or empty segments, no backslashes, no drive letters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepoPath(String);

/// Error returned for a malformed [`RepoPath`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid repository path {path:?}: {reason}")]
pub struct InvalidPath {
    pub path: String,
    pub reason: &'static str,
}

impl RepoPath {
    /// Validate and wrap a repository-relative path.
    pub fn new(path: impl Into<String>) -> Result<Self, InvalidPath> {
        let path = path.into();
        let reason = if path.is_empty() {
            Some("path is empty")
        } else if path.starts_with('/') {
            Some("path must be relative to the repository root")
        } else if path.contains('\\') {
            Some("use '/' as the separator")
        } else if path.as_bytes().get(1) == Some(&b':') {
            Some("path must not contain a drive letter")
        } else if path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
        {
            Some("path must not contain empty, '.' or '..' segments")
        } else {
            None
        };
        match reason {
            Some(reason) => Err(InvalidPath { path, reason }),
            None => Ok(Self(path)),
        }
    }

    /// The path as a `/`-separated string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RepoPath {
    type Error = InvalidPath;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RepoPath> for String {
    fn from(value: RepoPath) -> Self {
        value.0
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl JsonSchema for RepoPath {
    fn schema_name() -> Cow<'static, str> {
        "RepoPath".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "description": "Path relative to the repository root, '/' separated, no '..' segments.",
            "minLength": 1
        })
    }
}

/// Error returned for a malformed git object id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid object id {0:?}: expected 40 or 64 lowercase hex characters")]
pub struct InvalidObjectId(pub String);

fn is_full_object_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

macro_rules! define_object_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// Always a full id (40 hex characters for SHA-1, 64 for SHA-256), never abbreviated,
        /// so it stays unambiguous as the repository grows.
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap a full object id. Upper-case hex is normalised to lower-case.
            pub fn new(id: impl Into<String>) -> Result<Self, InvalidObjectId> {
                let id = id.into().to_ascii_lowercase();
                if is_full_object_id(&id) { Ok(Self(id)) } else { Err(InvalidObjectId(id)) }
            }

            /// The id as a hex string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidObjectId;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "description": "Full git object id (SHA-1 or SHA-256), lowercase hex.",
                    "pattern": "^([0-9a-f]{40}|[0-9a-f]{64})$"
                })
            }
        }
    };
}

define_object_id!(
    /// A git commit id.
    CommitId
);
define_object_id!(
    /// A git blob id: the content of one file at one moment.
    BlobId
);

// ---------------------------------------------------------------------------------------
// Hunks and line ranges
// ---------------------------------------------------------------------------------------

/// One changed region of a file, with unified-diff (`git diff -U0`) semantics.
///
/// `old_*` refers to the file before the change, `new_*` to the file after it. Lines are
/// 1-based. A pure insertion has `old_lines == 0` and `old_start` is the line it was
/// inserted after; a pure deletion has `new_lines == 0` and `new_start` is the line after
/// which the removed lines used to be. Either start may be `0` for a change at the very
/// top of the file.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

impl Hunk {
    /// The lines this hunk occupies in the new file, as an inclusive range.
    ///
    /// A pure deletion occupies no lines, so it is treated as touching the two lines on
    /// either side of where the code used to be. That way `why file:line` on either
    /// neighbour finds the decision that removed code there.
    pub fn new_side(&self) -> LineRange {
        if self.new_lines > 0 {
            LineRange {
                start: self.new_start,
                end: self.new_start + self.new_lines - 1,
            }
        } else {
            LineRange {
                start: self.new_start.max(1),
                end: self.new_start + 1,
            }
        }
    }

    /// Whether this hunk touches any line in `range` (new-file line numbers).
    pub fn touches(&self, range: LineRange) -> bool {
        self.new_side().overlaps(range)
    }
}

/// An inclusive, 1-based range of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

/// Error returned for an invalid [`LineRange`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid line range {start}-{end}: lines start at 1 and the end must not be before the start"
)]
pub struct InvalidLineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    /// A validated range from `start` to `end`, both inclusive.
    pub fn new(start: u32, end: u32) -> Result<Self, InvalidLineRange> {
        if start >= 1 && start <= end {
            Ok(Self { start, end })
        } else {
            Err(InvalidLineRange { start, end })
        }
    }

    /// A range of exactly one line.
    pub fn line(line: u32) -> Result<Self, InvalidLineRange> {
        Self::new(line, line)
    }

    /// Whether two ranges share at least one line.
    pub fn overlaps(&self, other: LineRange) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

impl fmt::Display for LineRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            write!(f, "{}", self.start)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}

// ---------------------------------------------------------------------------------------
// Decision
// ---------------------------------------------------------------------------------------

/// Who produced a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// An AI agent, typically through the MCP `record_decision` tool.
    Agent,
    /// An editor hook; carries hunks but no reasoning.
    Hook,
    /// A person, through the CLI or the app.
    Human,
}

/// The agent (tool, model, session) behind a record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentInfo {
    /// The agent tool, e.g. `claude-code`.
    pub tool: String,
    /// The model id, when the source knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The agent session id, used to group work and to match captures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// An option the agent considered and rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Alternative {
    pub option: String,
    pub rejected_because: String,
}

/// The changes a record made to one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Change {
    pub path: RepoPath,
    /// Blob of the file right after the change; absent when the change deleted the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_blob: Option<BlobId>,
    #[serde(default)]
    pub hunks: Vec<Hunk>,
}

/// A decision: what an agent or person changed, and why. Immutable once written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Decision {
    /// Schema version; always [`SCHEMA_VERSION`] for records this build writes.
    pub schema: u32,
    pub id: DecisionId,
    pub timestamp: DateTime<Utc>,
    pub source: Source,
    /// Required unless `source` is `human`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentInfo>,
    /// The prompt or task summary.
    pub task: String,
    /// The intent (change story) this decision belongs to: a short, free-text label
    /// grouping decisions that serve one purpose. Absent when the change wasn't
    /// tagged; stories then fall back to grouping by session (ADR 0011).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// Why the change was made. May be empty only for `source: hook`.
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
    /// The rule that allowed the action. Free text until the rules engine exists.
    pub allowed_by: String,
    /// HEAD when the change was made; absent in a repository with no commits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<CommitId>,
    #[serde(default)]
    pub changes: Vec<Change>,
    /// Hook captures this decision explains (ADR 0007).
    #[serde(default)]
    pub captures: Vec<CaptureId>,
}

impl Decision {
    /// A new decision stamped with a fresh id and the current time. Fill in the rest of
    /// the fields, then [`validate`](Self::validate) or write it through the store.
    pub fn new(source: Source, task: impl Into<String>, allowed_by: impl Into<String>) -> Self {
        let (timestamp, at) = now();
        Self {
            schema: SCHEMA_VERSION,
            id: DecisionId::new_at(at),
            timestamp,
            source,
            agent: None,
            task: task.into(),
            intent: None,
            reasoning: String::new(),
            alternatives: Vec::new(),
            allowed_by: allowed_by.into(),
            base_commit: None,
            changes: Vec::new(),
            captures: Vec::new(),
        }
    }

    /// The change to `path`, if this decision touched it.
    pub fn change_for(&self, path: &RepoPath) -> Option<&Change> {
        self.changes.iter().find(|c| &c.path == path)
    }

    /// The agent session this decision belongs to, if known.
    pub fn session(&self) -> Option<&str> {
        self.agent.as_ref().and_then(|a| a.session.as_deref())
    }

    /// The intent (change story) label this decision was tagged with, if any.
    pub fn intent(&self) -> Option<&str> {
        self.intent.as_deref()
    }
}

// ---------------------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------------------

/// The review status of a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    #[default]
    Unreviewed,
    Approved,
    Rejected,
}

impl fmt::Display for ReviewStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unreviewed => "unreviewed",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        })
    }
}

/// What an event records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// The decision was reviewed. The latest review event wins.
    Review {
        status: ReviewStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// The decision ended up in this commit. A decision may be linked to several.
    Link { commit: CommitId },
}

/// A state change for a decision (ADR 0005). Immutable once written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub schema: u32,
    pub id: EventId,
    pub decision: DecisionId,
    pub timestamp: DateTime<Utc>,
    /// Who did it (git `user.name` by default).
    pub actor: String,
    #[serde(flatten)]
    pub kind: EventKind,
}

impl Event {
    /// A new event stamped with a fresh id and the current time.
    pub fn new(decision: DecisionId, actor: impl Into<String>, kind: EventKind) -> Self {
        let (timestamp, at) = now();
        Self {
            schema: SCHEMA_VERSION,
            id: EventId::new_at(at),
            decision,
            timestamp,
            actor: actor.into(),
            kind,
        }
    }

    /// A review event.
    pub fn review(
        decision: DecisionId,
        actor: impl Into<String>,
        status: ReviewStatus,
        note: Option<String>,
    ) -> Self {
        Self::new(decision, actor, EventKind::Review { status, note })
    }

    /// A link-to-commit event.
    pub fn link(decision: DecisionId, actor: impl Into<String>, commit: CommitId) -> Self {
        Self::new(decision, actor, EventKind::Link { commit })
    }
}

// ---------------------------------------------------------------------------------------
// Captures
// ---------------------------------------------------------------------------------------

/// A raw change observed by an editor hook, with no reasoning (ADR 0007).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Capture {
    pub schema: u32,
    pub id: CaptureId,
    pub timestamp: DateTime<Utc>,
    pub agent: AgentInfo,
    /// The editor tool that made the change, e.g. `Edit` or `Write`.
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<CommitId>,
    pub change: Change,
}

impl Capture {
    /// A new capture stamped with a fresh id and the current time.
    pub fn new(agent: AgentInfo, tool_name: impl Into<String>, change: Change) -> Self {
        let (timestamp, at) = now();
        Self {
            schema: SCHEMA_VERSION,
            id: CaptureId::new_at(at),
            timestamp,
            agent,
            tool_name: tool_name.into(),
            base_commit: None,
            change,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_path_accepts_normal_relative_paths() {
        for ok in [
            "src/OrderService.cs",
            "README.md",
            "a/b/c.d.e",
            ".github/ci.yml",
        ] {
            assert!(RepoPath::new(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn repo_path_rejects_escaping_or_platform_paths() {
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a//b",
            "a/",
            "./a",
            "src\\x.cs",
            "C:/x",
        ] {
            assert!(RepoPath::new(bad).is_err(), "{bad:?}");
        }
        assert!(serde_json::from_str::<RepoPath>("\"../x\"").is_err());
    }

    #[test]
    fn object_ids_must_be_full_and_are_lowercased() {
        let sha1 = "A".repeat(40);
        assert_eq!(CommitId::new(&*sha1).unwrap().as_str(), "a".repeat(40));
        assert!(BlobId::new("b".repeat(64)).is_ok());
        assert!(CommitId::new("abc123").is_err());
        assert!(CommitId::new("g".repeat(40)).is_err());
    }

    #[test]
    fn line_range_rejects_zero_and_reversed() {
        assert!(LineRange::new(0, 3).is_err());
        assert!(LineRange::new(5, 4).is_err());
        assert_eq!(LineRange::new(4, 9).unwrap().to_string(), "4-9");
        assert_eq!(LineRange::line(7).unwrap().to_string(), "7");
    }

    fn hunk(new_start: u32, new_lines: u32) -> Hunk {
        Hunk {
            old_start: new_start,
            old_lines: 1,
            new_start,
            new_lines,
        }
    }

    #[test]
    fn hunk_touches_its_new_side_lines_inclusive() {
        let h = hunk(40, 11); // lines 40..=50
        let r = |a, b| LineRange::new(a, b).unwrap();
        assert!(!h.touches(r(1, 39)));
        assert!(h.touches(r(39, 40)));
        assert!(h.touches(r(45, 45)));
        assert!(h.touches(r(50, 60)));
        assert!(!h.touches(r(51, 60)));
        assert!(h.touches(r(1, 100)));
    }

    #[test]
    fn deletion_touches_both_neighbouring_lines() {
        let h = hunk(10, 0); // code removed between lines 10 and 11
        let line = |n| LineRange::line(n).unwrap();
        assert!(!h.touches(line(9)));
        assert!(h.touches(line(10)));
        assert!(h.touches(line(11)));
        assert!(!h.touches(line(12)));
        // Deletion at the top of the file.
        assert!(hunk(0, 0).touches(line(1)));
    }

    #[test]
    fn event_kind_is_flattened_into_the_event() {
        let decision = DecisionId::new_at(SystemTime::now());
        let event = Event::review(decision, "robbert", ReviewStatus::Approved, None);
        let json: serde_json::Value = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "review");
        assert_eq!(json["status"], "approved");
        assert!(json.get("note").is_none());
        let back: Event = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}

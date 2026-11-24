//! Error types for `concede-core`.

use std::fmt;
use std::path::PathBuf;

/// Errors returned by `concede-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file or directory could not be read or written.
    #[error("I/O error at {}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A record file is not valid JSON or doesn't match the schema.
    #[error("invalid record in {}", path.display())]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// A record was written by a newer (or unknown) schema version.
    #[error(
        "{} uses schema version {found}, but this build reads version {}",
        path.display(),
        crate::model::SCHEMA_VERSION
    )]
    UnsupportedSchema { path: PathBuf, found: String },

    /// A record failed validation.
    #[error(transparent)]
    Invalid(#[from] ValidationError),

    /// A record with this id already exists. Records are never overwritten.
    #[error("{} already exists; records are never overwritten", path.display())]
    AlreadyExists { path: PathBuf },

    /// The `git` executable could not be found on PATH.
    #[error("git was not found on PATH; Concede needs git to be installed")]
    GitNotFound,

    /// A git command failed.
    #[error("`{command}` failed: {stderr}")]
    Git { command: String, stderr: String },

    /// The directory is not inside a git work tree.
    #[error("{} is not inside a git repository", path.display())]
    NotARepository { path: PathBuf },

    /// A file path points outside the repository.
    #[error("{} is outside the repository", path.display())]
    OutsideRepository { path: PathBuf },

    /// The operation needs a commit, but the repository has none yet.
    #[error("the repository has no commits yet")]
    NoCommits,

    /// No decision or capture matches the given id.
    #[error("no decision or capture matches {0:?}")]
    NotFound(String),

    /// An id prefix matches more than one record.
    #[error("{prefix:?} matches {count} records; use more characters of the id")]
    Ambiguous { prefix: String, count: usize },

    /// A capture is already explained by a decision, so it can't be claimed or reviewed
    /// on its own.
    #[error("capture {capture} is already explained by decision {decision}")]
    CaptureExplained {
        capture: crate::id::CaptureId,
        decision: crate::id::DecisionId,
    },
}

/// Convenience alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// One or more validation problems with a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// Human-readable problems, one per violated rule.
    pub problems: Vec<String>,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid record: {}", self.problems.join("; "))
    }
}

impl std::error::Error for ValidationError {}

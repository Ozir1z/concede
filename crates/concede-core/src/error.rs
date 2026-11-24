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

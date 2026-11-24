//! Concede core library.
//!
//! All domain logic (decision log, rules, criticality, change store) lives here.
//! The CLI and the app are thin layers on top of this crate.
//!
//! The decision log (ADR 0005) is a directory of immutable JSON records:
//! [`Decision`]s explain changes, [`Event`]s record reviews and commit links, and
//! [`Capture`]s are raw changes seen by editor hooks (ADR 0007). [`Store`] reads and
//! writes them; [`Log`] answers questions about them. [`Concede`] is the entry point
//! that ties them to version control through a [`ChangeStore`] (ADR 0006).

pub mod change_store;
mod concede;
pub mod error;
pub mod git;
pub mod id;
pub mod linemap;
pub mod log;
pub mod model;
pub mod schema;
pub mod store;
mod validate;

pub use change_store::ChangeStore;
pub use concede::{
    Concede, FileRef, LinkOutcome, MappedHunk, NewCapture, NewDecision, ReviewOutcome, Target,
    UNCONFIGURED_RULE, Why, WhyEntry, WhyRecord,
};
pub use error::{Error, Result, ValidationError};
pub use git::GitCli;
pub use id::{CaptureId, DecisionId, EventId};
pub use linemap::{MappedRange, Mapping};
pub use log::{CaptureHit, DecisionHit, DecisionState, Inbox, Log, SessionGroup};
pub use model::{
    AgentInfo, Alternative, BlobId, Capture, Change, CommitId, Decision, Event, EventKind, Hunk,
    LineRange, RepoPath, ReviewStatus, SCHEMA_VERSION, Source,
};
pub use store::{LoadWarning, Loaded, Store};

/// The product name shown in the UI and CLI.
pub const PRODUCT_NAME: &str = "Concede";

/// The version of the core library, taken from the workspace version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A short identification string, e.g. `"Concede 0.1.0"`.
pub fn product_banner() -> String {
    format!("{PRODUCT_NAME} {VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_contains_name_and_version() {
        let banner = product_banner();
        assert!(banner.starts_with(PRODUCT_NAME));
        assert!(banner.ends_with(VERSION));
    }
}

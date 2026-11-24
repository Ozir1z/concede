//! Reading and writing `.concede/` (ADR 0005).
//!
//! Every record is its own immutable JSON file. Writes go to a temporary file in the same
//! directory and are then renamed into place, so readers never see a half-written record.
//! Loading is tolerant: a broken file becomes a warning instead of hiding every other
//! record.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};
use crate::log::Log;
use crate::model::{Capture, Decision, Event, SCHEMA_VERSION};

/// Name of the data directory at the repository root.
pub const DATA_DIR: &str = ".concede";

const DECISIONS: &str = "decisions";
const EVENTS: &str = "events";
const CAPTURES: &str = "captures";
const CAPTURE_ERRORS: &str = "capture-errors.log";

/// The on-disk decision log of one repository.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

/// A record file that could not be loaded. The rest of the log still loads.
#[derive(Debug)]
pub struct LoadWarning {
    pub path: PathBuf,
    pub error: Error,
}

impl fmt::Display for LoadWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Include the cause chain so the message says *what* is wrong with the file.
        write!(f, "{}", self.error)?;
        let mut source = std::error::Error::source(&self.error);
        while let Some(cause) = source {
            write!(f, ": {cause}")?;
            source = cause.source();
        }
        Ok(())
    }
}

/// The result of [`Store::load`].
#[derive(Debug)]
pub struct Loaded {
    pub log: Log,
    pub warnings: Vec<LoadWarning>,
}

impl Store {
    /// The store for the repository rooted at `repo_root` (its `.concede/` directory).
    /// Nothing is created until the first write.
    pub fn for_repo(repo_root: impl AsRef<Path>) -> Self {
        Self {
            dir: repo_root.as_ref().join(DATA_DIR),
        }
    }

    /// The `.concede/` directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Validate and write a decision. Returns the path written.
    pub fn write_decision(&self, decision: &Decision) -> Result<PathBuf> {
        decision.validate()?;
        self.write_record(DECISIONS, &decision.id.to_string(), decision)
    }

    /// Validate and write an event. Returns the path written.
    pub fn write_event(&self, event: &Event) -> Result<PathBuf> {
        event.validate()?;
        self.write_record(EVENTS, &event.id.to_string(), event)
    }

    /// Validate and write a capture. Returns the path written.
    pub fn write_capture(&self, capture: &Capture) -> Result<PathBuf> {
        capture.validate()?;
        self.write_record(CAPTURES, &capture.id.to_string(), capture)
    }

    /// Load every record. A missing `.concede/` directory is an empty log, not an error.
    pub fn load(&self) -> Result<Loaded> {
        let mut warnings = Vec::new();
        let decisions = self.load_kind(DECISIONS, &mut warnings, |d: &Decision| {
            (d.id.to_string(), d.validate())
        })?;
        let events = self.load_kind(EVENTS, &mut warnings, |e: &Event| {
            (e.id.to_string(), e.validate())
        })?;
        let captures = self.load_kind(CAPTURES, &mut warnings, |c: &Capture| {
            (c.id.to_string(), c.validate())
        })?;
        Ok(Loaded {
            log: Log::new(decisions, events, captures),
            warnings,
        })
    }

    /// Append a line to `.concede/capture-errors.log`, which is kept out of git by a
    /// `.concede/.gitignore` created on first use. Hooks must never fail the agent, so
    /// capture problems land here instead (ADR 0007).
    pub fn log_capture_error(&self, message: &str) -> Result<PathBuf> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| Error::Io { path, source }
        };
        fs::create_dir_all(&self.dir).map_err(io(&self.dir))?;

        let ignore = self.dir.join(".gitignore");
        if !ignore.exists() {
            fs::write(&ignore, format!("{CAPTURE_ERRORS}\n")).map_err(io(&ignore))?;
        }

        let path = self.dir.join(CAPTURE_ERRORS);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(io(&path))?;
        let line = format!(
            "{} {}\n",
            chrono::Utc::now().to_rfc3339(),
            message.replace('\n', " ")
        );
        file.write_all(line.as_bytes()).map_err(io(&path))?;
        Ok(path)
    }

    fn write_record<T: Serialize>(&self, kind: &str, id: &str, record: &T) -> Result<PathBuf> {
        let dir = self.dir.join(kind);
        fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;

        let path = dir.join(format!("{id}.json"));
        if path.exists() {
            return Err(Error::AlreadyExists { path });
        }

        let mut json = serde_json::to_string_pretty(record).map_err(|source| Error::Json {
            path: path.clone(),
            source,
        })?;
        json.push('\n');

        // Write to a dot-prefixed temp file (ignored by `load`), flush it to disk, then
        // rename. Rename within one directory is atomic on every platform we target.
        let tmp = dir.join(format!(".{id}.json.tmp"));
        let write = || -> std::io::Result<()> {
            let mut file = fs::File::create_new(&tmp)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp, &path)
        };
        write().map_err(|source| {
            // Best effort: don't leave temp files behind. The original error matters more.
            let _ = fs::remove_file(&tmp);
            Error::Io {
                path: path.clone(),
                source,
            }
        })?;
        Ok(path)
    }

    /// Load all records of one kind. `check` returns the record's id (to compare with the
    /// filename) and its validation result.
    fn load_kind<T, F>(
        &self,
        kind: &str,
        warnings: &mut Vec<LoadWarning>,
        check: F,
    ) -> Result<Vec<T>>
    where
        T: DeserializeOwned,
        F: Fn(
            &T,
        ) -> (
            String,
            std::result::Result<(), crate::error::ValidationError>,
        ),
    {
        let dir = self.dir.join(kind);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(Error::Io { path: dir, source }),
        };

        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| Error::Io {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            let name = entry.file_name();
            let is_record = path.extension() == Some(OsStr::new("json"))
                && !name.to_string_lossy().starts_with('.');
            if is_record {
                paths.push(path);
            }
        }
        // Filenames are ULIDs, so sorting by name is sorting by creation time.
        paths.sort();

        let mut records = Vec::with_capacity(paths.len());
        for path in paths {
            match read_record::<T>(&path) {
                Ok(record) => {
                    let (id, validation) = check(&record);
                    let stem = path.file_stem().and_then(OsStr::to_str).unwrap_or_default();
                    if stem != id {
                        let error = crate::error::ValidationError {
                            problems: vec![format!("file name does not match record id {id}")],
                        };
                        warnings.push(LoadWarning {
                            path,
                            error: error.into(),
                        });
                    } else if let Err(error) = validation {
                        warnings.push(LoadWarning {
                            path,
                            error: error.into(),
                        });
                    } else {
                        records.push(record);
                    }
                }
                Err(error) => warnings.push(LoadWarning { path, error }),
            }
        }
        Ok(records)
    }
}

/// Read one record, checking the schema version before mapping it onto `T` so a newer
/// file gives a clear "upgrade Concede" error instead of a confusing field error.
fn read_record<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.into(),
        source,
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|source| Error::Json {
        path: path.into(),
        source,
    })?;
    match value.get("schema") {
        Some(serde_json::Value::Number(n)) if n.as_u64() == Some(u64::from(SCHEMA_VERSION)) => {}
        Some(other) => {
            return Err(Error::UnsupportedSchema {
                path: path.into(),
                found: other.to_string(),
            });
        }
        None => {
            return Err(Error::UnsupportedSchema {
                path: path.into(),
                found: "none".into(),
            });
        }
    }
    serde_json::from_value(value).map_err(|source| Error::Json {
        path: path.into(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentInfo, BlobId, Change, Hunk, RepoPath, ReviewStatus, Source};

    fn decision() -> Decision {
        let mut d = Decision::new(Source::Agent, "Add retry", "unconfigured");
        d.agent = Some(AgentInfo {
            tool: "claude-code".into(),
            model: Some("claude-opus-5-5".into()),
            session: Some("s1".into()),
        });
        d.reasoning = "Transient failures".into();
        d.changes = vec![Change {
            path: RepoPath::new("src/a.cs").unwrap(),
            after_blob: Some(BlobId::new("a".repeat(40)).unwrap()),
            hunks: vec![Hunk {
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 2,
            }],
        }];
        d
    }

    #[test]
    fn capture_errors_are_appended_and_git_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::for_repo(tmp.path());
        store.log_capture_error("first\nproblem").unwrap();
        let path = store.log_capture_error("second").unwrap();
        let log = std::fs::read_to_string(path).unwrap();
        assert_eq!(log.lines().count(), 2);
        assert!(log.lines().next().unwrap().ends_with("first problem"));
        let ignore = std::fs::read_to_string(store.dir().join(".gitignore")).unwrap();
        assert_eq!(ignore, "capture-errors.log\n");
        // The log file is not a record and must not disturb loading.
        assert!(store.load().unwrap().warnings.is_empty());
    }

    #[test]
    fn missing_directory_is_an_empty_log() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = Store::for_repo(tmp.path()).load().unwrap();
        assert!(loaded.log.decisions().next().is_none());
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn records_round_trip_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::for_repo(tmp.path());
        let d = decision();
        let path = store.write_decision(&d).unwrap();
        assert_eq!(
            path,
            tmp.path()
                .join(".concede/decisions")
                .join(format!("{}.json", d.id))
        );
        assert!(std::fs::read_to_string(&path).unwrap().ends_with("}\n"));

        let e = Event::review(d.id, "robbert", ReviewStatus::Approved, None);
        store.write_event(&e).unwrap();

        let loaded = store.load().unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.log.decision(&d.id), Some(&d));
        assert_eq!(loaded.log.state(&d.id).review, ReviewStatus::Approved);
    }

    #[test]
    fn never_overwrites_a_record() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::for_repo(tmp.path());
        let d = decision();
        store.write_decision(&d).unwrap();
        assert!(matches!(
            store.write_decision(&d),
            Err(Error::AlreadyExists { .. })
        ));
    }

    #[test]
    fn refuses_to_write_invalid_records() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::for_repo(tmp.path());
        let mut d = decision();
        d.reasoning.clear();
        assert!(matches!(store.write_decision(&d), Err(Error::Invalid(_))));
        assert!(!store.dir().join("decisions").exists());
    }

    #[test]
    fn broken_files_become_warnings_and_the_rest_still_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::for_repo(tmp.path());
        let good = decision();
        store.write_decision(&good).unwrap();
        let dir = store.dir().join("decisions");

        std::fs::write(dir.join("01ARZ3NDEKTSV4RRFFQ69G5FAV.json"), "{ not json").unwrap();
        std::fs::write(
            dir.join("01ARZ3NDEKTSV4RRFFQ69G5FAW.json"),
            r#"{"schema": 2}"#,
        )
        .unwrap();
        // Valid record under the wrong file name.
        let moved = decision();
        let json = serde_json::to_string(&moved).unwrap();
        std::fs::write(dir.join("01ARZ3NDEKTSV4RRFFQ69G5FAX.json"), json).unwrap();
        // Ignored entirely: temp files and non-JSON files.
        std::fs::write(dir.join(".01ARZ3NDEKTSV4RRFFQ69G5FAY.json.tmp"), "junk").unwrap();
        std::fs::write(dir.join("notes.txt"), "junk").unwrap();

        let loaded = store.load().unwrap();
        let ids: Vec<_> = loaded.log.decisions().map(|d| d.id).collect();
        assert_eq!(ids, vec![good.id]);
        assert_eq!(loaded.warnings.len(), 3, "{:#?}", loaded.warnings);
        let messages: Vec<String> = loaded.warnings.iter().map(ToString::to_string).collect();
        assert!(
            messages.iter().any(|m| m.contains("schema version 2")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("does not match record id")),
            "{messages:?}"
        );
    }
}

//! Turning editor hook payloads into captures (ADR 0007).
//!
//! Only Claude Code for now. The payload format was checked against real PostToolUse
//! runs (fixtures in `tests/fixtures/claude-code/`), not assumed:
//!
//! - `tool_input.file_path` (or `notebook_path`) is the absolute path that was edited.
//! - `tool_response.structuredPatch` holds the applied patch **with context lines**, so
//!   the exact changed lines are recovered from the `-`/`+` markers.
//! - `Write` creating a new file sends an empty `structuredPatch`; the caller falls back
//!   to diffing with git.
//! - The payload carries `session_id` but no model name.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::concede::NewCapture;
use crate::model::{AgentInfo, Hunk};

/// The agent tool name recorded for Claude Code captures.
pub const CLAUDE_CODE: &str = "claude-code";

/// The parts of a Claude Code PostToolUse payload we use. Unknown fields are ignored,
/// so new fields in future Claude Code versions don't break capturing.
#[derive(Debug, Deserialize)]
struct PostToolUse {
    #[serde(default)]
    session_id: Option<String>,
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
    #[serde(default)]
    tool_response: Value,
}

/// One hunk of `structuredPatch` (unified diff with context).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchHunk {
    old_start: u32,
    new_start: u32,
    lines: Vec<String>,
}

/// Parse a Claude Code PostToolUse payload. Returns `Ok(None)` for tools that don't
/// edit a file (nothing to capture).
pub fn claude_code_capture(payload: &str) -> Result<Option<NewCapture>, serde_json::Error> {
    let event: PostToolUse = serde_json::from_str(payload)?;
    let path = ["file_path", "notebook_path"]
        .iter()
        .find_map(|key| event.tool_input.get(key).and_then(Value::as_str))
        .filter(|p| !p.is_empty());
    let Some(path) = path else { return Ok(None) };

    let hunks = event
        .tool_response
        .get("structuredPatch")
        .cloned()
        .and_then(|patch| serde_json::from_value::<Vec<PatchHunk>>(patch).ok())
        .map(|patch| {
            patch
                .iter()
                .flat_map(zero_context_hunks)
                .collect::<Vec<_>>()
        })
        .filter(|hunks| !hunks.is_empty());

    Ok(Some(NewCapture {
        agent: AgentInfo {
            tool: CLAUDE_CODE.into(),
            model: None,
            session: event.session_id,
        },
        tool_name: event.tool_name,
        path: PathBuf::from(path),
        hunks,
    }))
}

/// Split one context hunk into `-U0` hunks: each run of consecutive `-`/`+` lines is one
/// hunk, positioned with unified-diff conventions (a zero-length side points at the line
/// it follows).
fn zero_context_hunks(patch: &PatchHunk) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let (mut old, mut new) = (patch.old_start, patch.new_start);
    let mut current: Option<Hunk> = None;

    let flush = |current: &mut Option<Hunk>, hunks: &mut Vec<Hunk>| {
        if let Some(mut h) = current.take() {
            if h.old_lines == 0 {
                h.old_start = h.old_start.saturating_sub(1);
            }
            if h.new_lines == 0 {
                h.new_start = h.new_start.saturating_sub(1);
            }
            hunks.push(h);
        }
    };

    for line in &patch.lines {
        let start = || Hunk {
            old_start: old,
            old_lines: 0,
            new_start: new,
            new_lines: 0,
        };
        match line.chars().next() {
            Some('-') => {
                current.get_or_insert_with(start).old_lines += 1;
                old += 1;
            }
            Some('+') => {
                current.get_or_insert_with(start).new_lines += 1;
                new += 1;
            }
            // "\ No newline at end of file" refers to the previous line; it isn't a line.
            Some('\\') => {}
            // Context (' ') or anything unexpected: the run of changes ends here.
            _ => {
                flush(&mut current, &mut hunks);
                old += 1;
                new += 1;
            }
        }
    }
    flush(&mut current, &mut hunks);
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDIT: &str = include_str!("../tests/fixtures/claude-code/post-tool-use-edit.json");
    const WRITE_CREATE: &str =
        include_str!("../tests/fixtures/claude-code/post-tool-use-write-create.json");

    fn patch(old_start: u32, new_start: u32, lines: &[&str]) -> PatchHunk {
        PatchHunk {
            old_start,
            new_start,
            lines: lines.iter().map(|l| l.to_string()).collect(),
        }
    }

    fn h(old_start: u32, old_lines: u32, new_start: u32, new_lines: u32) -> Hunk {
        Hunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
        }
    }

    #[test]
    fn real_edit_payload_becomes_a_one_line_hunk() {
        let capture = claude_code_capture(EDIT).unwrap().unwrap();
        assert_eq!(capture.tool_name, "Edit");
        assert_eq!(capture.path, PathBuf::from("/repo/notes.txt"));
        assert_eq!(capture.agent.tool, "claude-code");
        assert_eq!(
            capture.agent.session.as_deref(),
            Some("00000000-0000-4000-8000-000000000001")
        );
        assert_eq!(capture.agent.model, None);
        assert_eq!(capture.hunks, Some(vec![h(3, 1, 3, 1)]));
    }

    #[test]
    fn real_write_create_payload_has_no_hunks_so_git_decides() {
        let capture = claude_code_capture(WRITE_CREATE).unwrap().unwrap();
        assert_eq!(capture.tool_name, "Write");
        assert_eq!(capture.path, PathBuf::from("/repo/hello.txt"));
        assert_eq!(capture.hunks, None);
    }

    #[test]
    fn tools_without_a_file_are_ignored_and_garbage_is_an_error() {
        let bash = r#"{"session_id":"s","tool_name":"Bash","tool_input":{"command":"ls"}}"#;
        assert!(claude_code_capture(bash).unwrap().is_none());
        assert!(claude_code_capture("not json").is_err());
    }

    #[test]
    fn notebook_edits_use_notebook_path() {
        let nb = r#"{"tool_name":"NotebookEdit","tool_input":{"notebook_path":"/r/a.ipynb"}}"#;
        let capture = claude_code_capture(nb).unwrap().unwrap();
        assert_eq!(capture.path, PathBuf::from("/r/a.ipynb"));
        assert_eq!(capture.agent.session, None);
    }

    #[test]
    fn splits_context_hunks_into_runs_of_changes() {
        // Replace line 3, insert 2 lines after old line 5, delete old line 7.
        let p = patch(
            1,
            1,
            &[
                " 1", " 2", "-3", "+three", " 4", " 5", "+a", "+b", " 6", "-7", " 8",
            ],
        );
        assert_eq!(
            zero_context_hunks(&p),
            vec![h(3, 1, 3, 1), h(5, 0, 6, 2), h(7, 1, 8, 0)]
        );
    }

    #[test]
    fn changes_at_the_top_and_missing_final_newline() {
        let top = patch(1, 1, &["+header", " 1", " 2"]);
        assert_eq!(zero_context_hunks(&top), vec![h(0, 0, 1, 1)]);

        let no_newline = patch(
            4,
            4,
            &[
                " 4",
                "-5",
                "\\ No newline at end of file",
                "+five",
                "\\ No newline at end of file",
            ],
        );
        assert_eq!(zero_context_hunks(&no_newline), vec![h(5, 1, 5, 1)]);

        let delete_first = patch(1, 1, &["-1", " 2"]);
        assert_eq!(zero_context_hunks(&delete_first), vec![h(1, 1, 0, 0)]);
    }
}

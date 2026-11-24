// Test helpers outside #[test] functions aren't covered by allow-unwrap-in-tests;
// panicking is the right failure mode for test setup.
#![allow(clippy::unwrap_used)]

//! End-to-end tests: run the real `concede` binary against a throwaway git repository.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const CONCEDE: &str = env!("CARGO_BIN_EXE_concede");

struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new() -> Self {
        let repo = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "Test User"],
            &["config", "user.email", "test@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
        ] {
            repo.git(args);
        }
        repo
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn write(&self, path: &str, content: &str) {
        std::fs::write(self.path().join(path), content).unwrap();
    }

    fn run(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut child = Command::new(CONCEDE)
            .args(args)
            .current_dir(self.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        {
            use std::io::Write;
            let mut pipe = child.stdin.take().unwrap();
            pipe.write_all(stdin.unwrap_or("").as_bytes()).unwrap();
        }
        child.wait_with_output().unwrap()
    }

    /// Run and expect success; return stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args, None);
        assert!(
            out.status.success(),
            "concede {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(args)).unwrap()
    }
}

fn numbered(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

#[test]
fn capture_log_review_link_and_why() {
    let repo = Repo::new();
    repo.write("Order.cs", &numbered(10));
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "-m", "base"]);

    // An agent edits line 3 and the hook reports it, in Claude Code's payload shape.
    repo.write("Order.cs", &numbered(10).replace("line 3\n", "retry()\n"));
    let file = repo.path().join("Order.cs");
    let payload = json!({
        "session_id": "s1",
        "hook_event_name": "PostToolUse",
        "tool_name": "Edit",
        "tool_input": { "file_path": file, "old_string": "line 3\n", "new_string": "retry()\n" },
        "tool_response": { "structuredPatch": [
            { "oldStart": 2, "oldLines": 3, "newStart": 2, "newLines": 3,
              "lines": [" line 2", "-line 3", "+retry()", " line 4"] }
        ]}
    });
    let out = repo.run(&["capture"], Some(&payload.to_string()));
    assert!(out.status.success());

    let inbox = repo.json(&["inbox", "--json"]);
    assert_eq!(inbox["unexplained"].as_array().unwrap().len(), 1);
    assert_eq!(
        inbox["unexplained"][0]["change"]["hunks"][0]["new_start"],
        3
    );

    // The agent explains it; the decision claims the capture.
    let decision = repo.json(&[
        "log",
        "--json",
        "--task",
        "Add retry",
        "--reasoning",
        "Transient failures",
        "--file",
        "Order.cs",
        "--agent",
        "claude-code",
        "--session",
        "s1",
        "--alternative",
        "Polly: new dependency",
    ]);
    let id = decision["id"].as_str().unwrap().to_owned();
    assert_eq!(decision["captures"].as_array().unwrap().len(), 1);
    let inbox = repo.json(&["inbox", "--json"]);
    assert!(inbox["unexplained"].as_array().unwrap().is_empty());
    assert_eq!(inbox["sessions"][0]["session"], "s1");

    // Review by prefix, commit, link.
    assert!(
        repo.ok(&["review", &id[..14], "--approve"])
            .contains("approved")
    );
    repo.git(&["commit", "-qam", "retry"]);
    assert!(repo.ok(&["link"]).contains(&format!("linked {id}")));
    assert!(repo.ok(&["inbox"]).contains("Inbox zero"));

    // Insert two lines at the top: line 3 is now line 5, and why still finds it.
    repo.write(
        "Order.cs",
        &format!("a\nb\n{}", numbered(10).replace("line 3\n", "retry()\n")),
    );
    let why = repo.json(&["why", "Order.cs:5", "--json"]);
    let entry = &why["entries"][0];
    assert_eq!(entry["kind"], "decision");
    assert_eq!(entry["state"]["review"], "approved");
    assert_eq!(
        entry["hunks"][0]["current"],
        json!({ "start": 5, "end": 5 })
    );
    assert_eq!(entry["hunks"][0]["mapping"], "unchanged");
    assert!(
        repo.json(&["why", "Order.cs:3", "--json"])["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let text = repo.ok(&["why", "Order.cs:5"]);
    assert!(
        text.contains("Add retry") && text.contains("5 (was 3)") && text.contains("Polly"),
        "{text}"
    );
}

#[test]
fn capture_never_fails_the_agent() {
    let repo = Repo::new();
    for payload in [
        "not json",
        r#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#,
        "",
    ] {
        let out = repo.run(&["capture"], Some(payload));
        assert!(out.status.success(), "payload {payload:?}");
    }
    // A file outside any repository is ignored quietly.
    let elsewhere = tempfile::tempdir().unwrap();
    let payload = json!({ "tool_name": "Write", "tool_input": { "file_path": elsewhere.path().join("x.txt") } });
    assert!(
        repo.run(&["capture"], Some(&payload.to_string()))
            .status
            .success()
    );
}

#[test]
fn log_reads_json_from_stdin_and_reports_bad_input() {
    let repo = Repo::new();
    repo.write("a.txt", "one\n");
    let input =
        json!({ "task": "Seed", "reasoning": "Needed a file", "files": [{ "path": "a.txt" }] });
    let out = repo.run(&["log", "--stdin", "--json"], Some(&input.to_string()));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let decision: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(decision["source"], "human");
    assert_eq!(decision["changes"][0]["hunks"][0]["new_lines"], 1);

    let bad = repo.run(&["log", "--stdin"], Some(r#"{"task": "x"}"#));
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("invalid decision JSON"));

    let missing = repo.run(&["review", "ZZZZZZZZ", "--approve"], None);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no decision or capture matches"));
}

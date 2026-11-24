//! Text and JSON output.

use std::collections::BTreeSet;

use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use concede_core::{
    Capture, Decision, DecisionState, LinkOutcome, Log, MappedHunk, Mapping, ReviewStatus, Why,
    WhyRecord,
};
use serde::Serialize;
use serde_json::{Value, json};

pub fn print_json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Short ids
// ---------------------------------------------------------------------------------------

/// Shortest unique prefixes for every record id, at least 12 characters (ULIDs share
/// their first 10 characters within a millisecond, so shorter prefixes collide often).
struct ShortIds(Vec<String>);

impl ShortIds {
    const MIN: usize = 12;

    fn new(log: &Log) -> Self {
        let mut ids: Vec<String> = log
            .decisions()
            .map(|d| d.id.to_string())
            .chain(log.captures().map(|c| c.id.to_string()))
            .collect();
        ids.sort();
        Self(ids)
    }

    fn get(&self, id: &str) -> String {
        let common = |other: &String| {
            other
                .bytes()
                .zip(id.bytes())
                .take_while(|(a, b)| a == b)
                .count()
        };
        let Ok(i) = self.0.binary_search_by(|probe| probe.as_str().cmp(id)) else {
            return id.to_owned();
        };
        let before = i.checked_sub(1).map(|j| common(&self.0[j])).unwrap_or(0);
        let after = self.0.get(i + 1).map(common).unwrap_or(0);
        let len = (before.max(after) + 1).max(Self::MIN).min(id.len());
        id[..len].to_owned()
    }
}

fn local(ts: &DateTime<Utc>) -> String {
    ts.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn status_label(status: ReviewStatus) -> &'static str {
    match status {
        ReviewStatus::Unreviewed => "UNREVIEWED",
        ReviewStatus::Approved => "approved",
        ReviewStatus::Rejected => "REJECTED",
    }
}

fn agent_label(decision: &Decision) -> String {
    match &decision.agent {
        Some(a) => match &a.model {
            Some(model) => format!("{} ({model})", a.tool),
            None => a.tool.clone(),
        },
        None => "human".to_owned(),
    }
}

// ---------------------------------------------------------------------------------------
// why
// ---------------------------------------------------------------------------------------

fn mapped_label(hunk: &MappedHunk) -> String {
    let recorded = hunk.recorded.new_side();
    match (hunk.current.mapping, hunk.current.range) {
        (Mapping::Gone, _) | (_, None) => format!("{recorded} (since deleted)"),
        (Mapping::Approximate, Some(_)) => {
            format!("{recorded} (approximate: original content no longer available)")
        }
        (mapping, Some(now)) => {
            let mut label = now.to_string();
            if now != recorded {
                label.push_str(&format!(" (was {recorded})"));
            }
            if mapping == Mapping::Modified {
                label.push_str(" (modified since)");
            }
            label
        }
    }
}

fn lines_label(hunks: &[MappedHunk]) -> String {
    if hunks.is_empty() {
        return "no changed lines recorded".to_owned();
    }
    hunks
        .iter()
        .map(mapped_label)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn print_why(why: &Why, log: &Log) {
    let short = ShortIds::new(log);
    let target = match why.lines {
        Some(lines) => format!("{}:{lines}", why.path),
        None => why.path.to_string(),
    };
    if why.entries.is_empty() {
        println!("No recorded decisions touch {target}.");
        return;
    }
    println!("{target}: {} record(s), oldest first\n", why.entries.len());
    for entry in &why.entries {
        match &entry.record {
            WhyRecord::Decision { decision, state } => {
                println!(
                    "{}  {}  {}  {}",
                    short.get(&decision.id.to_string()),
                    local(&decision.timestamp),
                    status_label(state.review),
                    agent_label(decision),
                );
                println!("  Task:     {}", decision.task);
                if !decision.reasoning.is_empty() {
                    println!("  Why:      {}", decision.reasoning);
                }
                for alt in &decision.alternatives {
                    println!("  Rejected: {} ({})", alt.option, alt.rejected_because);
                }
                println!("  Lines:    {}", lines_label(&entry.hunks));
                print_state(state);
            }
            WhyRecord::Unexplained { capture } => {
                println!(
                    "{}  {}  UNEXPLAINED  {} by {}",
                    short.get(&capture.id.to_string()),
                    local(&capture.timestamp),
                    capture.tool_name,
                    capture.agent.tool,
                );
                println!("  No decision explains this change yet.");
                println!("  Lines:    {}", lines_label(&entry.hunks));
            }
        }
        println!();
    }
}

fn print_state(state: &DecisionState) {
    if let Some(review) = &state.last_review {
        let note = match &review.kind {
            concede_core::EventKind::Review {
                note: Some(note), ..
            } => format!(": {note}"),
            _ => String::new(),
        };
        println!(
            "  Review:   {} by {} on {}{note}",
            state.review,
            review.actor,
            local(&review.timestamp)
        );
    }
    if !state.commits.is_empty() {
        let commits: Vec<&str> = state.commits.iter().map(|c| &c.as_str()[..12]).collect();
        println!("  Commits:  {}", commits.join(", "));
    }
}

fn hunk_json(hunk: &MappedHunk) -> Value {
    let mapping = match hunk.current.mapping {
        Mapping::Unchanged => "unchanged",
        Mapping::Modified => "modified",
        Mapping::Gone => "gone",
        Mapping::Approximate => "approximate",
    };
    let recorded = hunk.recorded.new_side();
    json!({
        "recorded": { "start": recorded.start, "end": recorded.end },
        "current": hunk.current.range.map(|r| json!({ "start": r.start, "end": r.end })),
        "mapping": mapping,
        "hunk": hunk.recorded,
    })
}

fn state_json(state: &DecisionState) -> Value {
    json!({
        "review": state.review,
        "last_review": state.last_review,
        "commits": state.commits,
    })
}

pub fn why_json(why: &Why) -> Value {
    let entries: Vec<Value> = why
        .entries
        .iter()
        .map(|entry| {
            let hunks: Vec<Value> = entry.hunks.iter().map(hunk_json).collect();
            match &entry.record {
                WhyRecord::Decision { decision, state } => json!({
                    "kind": "decision",
                    "decision": decision,
                    "state": state_json(state),
                    "hunks": hunks,
                }),
                WhyRecord::Unexplained { capture } => json!({
                    "kind": "unexplained",
                    "capture": capture,
                    "hunks": hunks,
                }),
            }
        })
        .collect();
    json!({
        "path": why.path,
        "lines": why.lines.map(|r| json!({ "start": r.start, "end": r.end })),
        "entries": entries,
    })
}

// ---------------------------------------------------------------------------------------
// inbox
// ---------------------------------------------------------------------------------------

fn files_label(paths: &[String]) -> String {
    match paths {
        [] => "no files".to_owned(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first} (+{} more)", rest.len()),
    }
}

fn capture_label(capture: &Capture) -> String {
    let lines: Vec<String> = capture
        .change
        .hunks
        .iter()
        .map(|h| h.new_side().to_string())
        .collect();
    format!(
        "{} {} lines {}",
        capture.tool_name,
        capture.change.path,
        lines.join(", ")
    )
}

pub fn print_inbox(log: &Log) {
    let inbox = log.inbox();
    if inbox.is_empty() {
        println!("Inbox zero: nothing to review.");
        return;
    }
    let short = ShortIds::new(log);
    let unreviewed: usize = inbox.sessions.iter().map(|g| g.decisions.len()).sum();
    if unreviewed > 0 {
        println!("Unreviewed decisions ({unreviewed})");
        for group in &inbox.sessions {
            let tools: BTreeSet<String> = group.decisions.iter().map(|d| agent_label(d)).collect();
            let tools = tools.into_iter().collect::<Vec<_>>().join(", ");
            match group.session {
                Some(session) => println!("\n  Session {session}  ({tools})"),
                None => println!("\n  No session  ({tools})"),
            }
            for d in &group.decisions {
                let paths: Vec<String> = d.changes.iter().map(|c| c.path.to_string()).collect();
                println!(
                    "    {}  {}  {}",
                    short.get(&d.id.to_string()),
                    local(&d.timestamp),
                    d.task
                );
                println!("    {:12}  {}", "", files_label(&paths));
            }
        }
        println!();
    }
    if !inbox.unexplained.is_empty() {
        println!("Unexplained changes ({})", inbox.unexplained.len());
        for c in &inbox.unexplained {
            let session = c.agent.session.as_deref().unwrap_or("no session");
            println!(
                "    {}  {}  {}  ({}, {session})",
                short.get(&c.id.to_string()),
                local(&c.timestamp),
                capture_label(c),
                c.agent.tool,
            );
        }
        println!();
    }
    println!("Review with: concede review <id> --approve   (or --reject --note \"...\")");
    println!("Details:     concede why <file>");
}

pub fn inbox_json(log: &Log) -> Value {
    let inbox = log.inbox();
    let sessions: Vec<Value> = inbox
        .sessions
        .iter()
        .map(|g| json!({ "session": g.session, "decisions": g.decisions }))
        .collect();
    json!({ "sessions": sessions, "unexplained": inbox.unexplained })
}

pub fn link_json(outcome: &LinkOutcome) -> Value {
    json!({
        "commit": outcome.commit,
        "linked": outcome.linked,
        "already_linked": outcome.already_linked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use concede_core::{AgentInfo, Source};

    #[test]
    fn short_ids_are_unique_and_at_least_twelve_characters() {
        let mut decisions = Vec::new();
        for _ in 0..50 {
            let mut d = Decision::new(Source::Agent, "t", "r");
            d.agent = Some(AgentInfo {
                tool: "x".into(),
                model: None,
                session: None,
            });
            d.reasoning = "r".into();
            decisions.push(d);
        }
        let log = Log::new(decisions.clone(), vec![], vec![]);
        let short = ShortIds::new(&log);
        let prefixes: BTreeSet<String> = decisions
            .iter()
            .map(|d| short.get(&d.id.to_string()))
            .collect();
        assert_eq!(prefixes.len(), decisions.len());
        assert!(prefixes.iter().all(|p| p.len() >= 12));
        for d in &decisions {
            let full = d.id.to_string();
            assert!(full.starts_with(&short.get(&full)));
        }
    }
}

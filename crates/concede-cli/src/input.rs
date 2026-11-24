//! Parsing user input: `path:lines` targets, `--alternative` strings and the JSON form of
//! `concede log --stdin`.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use concede_core::{AgentInfo, Alternative, CaptureId, FileRef, LineRange, NewDecision, Source};
use serde::Deserialize;

use crate::args::LogArgs;

/// Split `path`, `path:LINE` or `path:START-END`. The suffix only counts as lines when it
/// parses as a range, so Windows drive letters (`C:\x`) and odd file names still work.
pub fn parse_target(text: &str) -> Result<(PathBuf, Option<LineRange>)> {
    if let Some((path, suffix)) = text.rsplit_once(':')
        && !path.is_empty()
        && let Some(lines) = parse_lines(suffix)
    {
        return Ok((PathBuf::from(path), Some(lines?)));
    }
    Ok((PathBuf::from(text), None))
}

/// `Some(Ok)` for a valid range, `Some(Err)` for something that looks like a range but
/// isn't valid (e.g. `0` or `9-3`), `None` if it doesn't look like lines at all.
fn parse_lines(text: &str) -> Option<Result<LineRange>> {
    let (start, end) = match text.split_once('-') {
        Some((a, b)) => (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?),
        None => {
            let n = text.trim().parse::<u32>().ok()?;
            (n, n)
        }
    };
    Some(LineRange::new(start, end).map_err(|e| anyhow!(e)))
}

fn parse_file(text: &str) -> Result<FileRef> {
    let (path, lines) = parse_target(text)?;
    Ok(FileRef {
        path,
        lines: lines.into_iter().collect(),
    })
}

fn parse_alternative(text: &str) -> Result<Alternative> {
    match text.split_once(':') {
        Some((option, reason)) if !option.trim().is_empty() => Ok(Alternative {
            option: option.trim().to_owned(),
            rejected_because: reason.trim().to_owned(),
        }),
        _ => bail!("--alternative must look like \"OPTION: REASON\", got {text:?}"),
    }
}

fn parse_claim(text: &str) -> Result<CaptureId> {
    text.trim()
        .parse()
        .with_context(|| format!("--claim needs a full capture id, got {text:?}"))
}

/// Build a decision from `concede log` flags.
pub fn decision_from_args(args: LogArgs) -> Result<NewDecision> {
    let agent = args.agent.map(|tool| AgentInfo {
        tool,
        model: args.model,
        session: args.session,
    });
    Ok(NewDecision {
        source: if agent.is_some() {
            Source::Agent
        } else {
            Source::Human
        },
        agent,
        task: args.task.unwrap_or_default(),
        reasoning: args.reasoning.unwrap_or_default(),
        alternatives: args
            .alternatives
            .iter()
            .map(|a| parse_alternative(a))
            .collect::<Result<_>>()?,
        allowed_by: args.allowed_by,
        files: args
            .files
            .iter()
            .map(|f| parse_file(f))
            .collect::<Result<_>>()?,
        claim: args
            .claims
            .iter()
            .map(|c| parse_claim(c))
            .collect::<Result<_>>()?,
    })
}

/// The JSON accepted by `concede log --stdin`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogInput {
    task: String,
    reasoning: String,
    #[serde(default)]
    files: Vec<FileInput>,
    #[serde(default)]
    alternatives: Vec<Alternative>,
    #[serde(default)]
    allowed_by: Option<String>,
    #[serde(default)]
    agent: Option<AgentInfo>,
    #[serde(default)]
    claim: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileInput {
    path: PathBuf,
    /// "LINE" or "START-END" strings, the same syntax as on the command line.
    #[serde(default)]
    lines: Vec<String>,
}

/// Build a decision from the JSON form of `concede log --stdin`.
pub fn decision_from_json(text: &str) -> Result<NewDecision> {
    let input: LogInput = serde_json::from_str(text).context("invalid decision JSON on stdin")?;
    let files = input
        .files
        .into_iter()
        .map(|f| {
            let lines = f
                .lines
                .iter()
                .map(|l| parse_lines(l).unwrap_or_else(|| Err(anyhow!("invalid line range {l:?}"))))
                .collect::<Result<_>>()?;
            Ok(FileRef {
                path: f.path,
                lines,
            })
        })
        .collect::<Result<_>>()?;
    Ok(NewDecision {
        source: if input.agent.is_some() {
            Source::Agent
        } else {
            Source::Human
        },
        agent: input.agent,
        task: input.task,
        reasoning: input.reasoning,
        alternatives: input.alternatives,
        allowed_by: input.allowed_by,
        files,
        claim: input
            .claim
            .iter()
            .map(|c| parse_claim(c))
            .collect::<Result<_>>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(a: u32, b: u32) -> Option<LineRange> {
        Some(LineRange::new(a, b).unwrap())
    }

    #[test]
    fn targets_with_and_without_lines() {
        assert_eq!(
            parse_target("src/a.cs").unwrap(),
            (PathBuf::from("src/a.cs"), None)
        );
        assert_eq!(
            parse_target("src/a.cs:42").unwrap(),
            (PathBuf::from("src/a.cs"), r(42, 42))
        );
        assert_eq!(
            parse_target("src/a.cs:40-60").unwrap(),
            (PathBuf::from("src/a.cs"), r(40, 60))
        );
        assert_eq!(
            parse_target(r"C:\repo\a.cs").unwrap(),
            (PathBuf::from(r"C:\repo\a.cs"), None)
        );
        assert_eq!(
            parse_target(r"C:\repo\a.cs:7").unwrap(),
            (PathBuf::from(r"C:\repo\a.cs"), r(7, 7))
        );
        assert_eq!(
            parse_target("weird:name").unwrap(),
            (PathBuf::from("weird:name"), None)
        );
        assert!(parse_target("a.cs:0").is_err());
        assert!(parse_target("a.cs:9-3").is_err());
    }

    #[test]
    fn alternatives_need_option_and_reason() {
        let alt = parse_alternative("Polly policy: new dependency: overkill").unwrap();
        assert_eq!(alt.option, "Polly policy");
        assert_eq!(alt.rejected_because, "new dependency: overkill");
        assert!(parse_alternative("no separator").is_err());
        assert!(parse_alternative(": reason only").is_err());
    }

    #[test]
    fn json_input_matches_the_flags() {
        let json = r#"{
            "task": "Add retry", "reasoning": "flaky network",
            "files": [{"path": "src/a.cs", "lines": ["40-52", "60"]}],
            "alternatives": [{"option": "Polly", "rejected_because": "dependency"}],
            "agent": {"tool": "claude-code", "model": "claude-opus-5-5", "session": "s1"}
        }"#;
        let d = decision_from_json(json).unwrap();
        assert_eq!(d.source, Source::Agent);
        assert_eq!(
            d.files[0].lines,
            vec![r(40, 52).unwrap(), r(60, 60).unwrap()]
        );
        assert_eq!(d.agent.unwrap().session.as_deref(), Some("s1"));

        assert!(decision_from_json(r#"{"task": "t", "reasoning": "r", "typo": 1}"#).is_err());
        assert!(
            decision_from_json(
                r#"{"task": "t", "reasoning": "r", "files": [{"path": "a", "lines": ["x"]}]}"#
            )
            .is_err()
        );
    }
}

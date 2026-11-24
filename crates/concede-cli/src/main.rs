//! `concede` command line interface. A thin layer over `concede-core`: argument parsing,
//! input files, and output formatting. Every rule lives in the core crate.

mod args;
mod input;
mod output;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;
use concede_core::{Concede, NewCapture, ReviewStatus, hooks};

use crate::args::{Cli, Command, LogArgs};

fn main() -> ExitCode {
    let cli = Cli::parse();
    // The hook entry point must never fail the agent: it handles its own errors.
    if let Command::Capture = cli.command {
        capture(cli.repo.as_deref());
        return ExitCode::SUCCESS;
    }
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let cwd = std::env::current_dir().context("can't read the current directory")?;
    let concede = Concede::discover(cli.repo.as_deref().unwrap_or(&cwd))?;
    match cli.command {
        Command::Log(args) => log(&concede, &cwd, args),
        Command::Why { target, json } => {
            let (path, lines) = input::parse_target(&target)?;
            let why = concede.why(&absolute(&cwd, &path), lines)?;
            let log = concede.load()?.log;
            if json {
                output::print_json(&output::why_json(&why))
            } else {
                output::print_why(&why, &log);
                Ok(())
            }
        }
        Command::Inbox { json } => {
            let loaded = concede.load()?;
            for warning in &loaded.warnings {
                eprintln!("warning: {warning}");
            }
            if json {
                output::print_json(&output::inbox_json(&loaded.log))
            } else {
                output::print_inbox(&loaded.log);
                Ok(())
            }
        }
        Command::Review {
            id,
            approve,
            reject,
            note,
            actor,
        } => {
            let status = match (approve, reject) {
                (true, false) => ReviewStatus::Approved,
                (false, true) => ReviewStatus::Rejected,
                _ => bail!("pass exactly one of --approve or --reject"),
            };
            let outcome = concede.review(&id, status, note, actor)?;
            if let Some(capture) = outcome.wrapped_capture {
                println!(
                    "Recorded unexplained change {capture} as decision {}.",
                    outcome.decision
                );
            }
            println!("{status} {}", outcome.decision);
            Ok(())
        }
        Command::Link { ids, actor, json } => {
            let outcome = concede.link(&ids, actor)?;
            if json {
                return output::print_json(&output::link_json(&outcome));
            }
            let short = &outcome.commit.as_str()[..12];
            if outcome.linked.is_empty() && outcome.already_linked.is_empty() {
                println!("Nothing to link: no approved decisions are waiting for a commit.");
            }
            for id in &outcome.linked {
                println!("linked {id} -> {short}");
            }
            for id in &outcome.already_linked {
                println!("already linked {id} -> {short}");
            }
            Ok(())
        }
        Command::Capture => unreachable!("handled before run()"),
    }
}

fn log(concede: &Concede, cwd: &Path, args: LogArgs) -> Result<()> {
    let json_out = args.json;
    let new = if args.stdin {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("can't read stdin")?;
        input::decision_from_json(&text)?
    } else {
        input::decision_from_args(args)?
    };
    let mut new = new;
    for file in &mut new.files {
        file.path = absolute(cwd, &file.path);
    }
    let decision = concede.record_decision(new)?;
    if json_out {
        return output::print_json(&decision);
    }
    println!("recorded decision {}", decision.id);
    for change in &decision.changes {
        let lines: Vec<String> = change
            .hunks
            .iter()
            .map(|h| h.new_side().to_string())
            .collect();
        let lines = if lines.is_empty() {
            "no changed lines".to_owned()
        } else {
            lines.join(", ")
        };
        println!("  {}: {lines}", change.path);
    }
    if !decision.captures.is_empty() {
        println!("  explains {} captured edit(s)", decision.captures.len());
    }
    Ok(())
}

/// `concede capture`: read a Claude Code PostToolUse payload from stdin and record it.
/// Any problem is written to stderr and, when the repository is known, to
/// `.concede/capture-errors.log`, then swallowed.
fn capture(repo: Option<&Path>) {
    let mut payload = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut payload) {
        eprintln!("concede capture: can't read stdin: {e}");
        return;
    }
    let input = match hooks::claude_code_capture(&payload) {
        Ok(Some(input)) => input,
        Ok(None) => return,
        Err(e) => {
            eprintln!("concede capture: not a hook payload: {e}");
            return;
        }
    };
    // The repository is the one containing the edited file, which may differ from the
    // agent's working directory.
    let start = repo
        .map(Path::to_path_buf)
        .unwrap_or_else(|| existing_parent(&input.path));
    let concede = match Concede::discover(&start) {
        Ok(concede) => concede,
        // Not in a git repository: nothing for us to record.
        Err(_) => return,
    };
    if let Err(e) = record_capture(&concede, input) {
        let message = format!("{e:#}");
        eprintln!("concede capture: {message}");
        let _ = concede.store().log_capture_error(&message);
    }
}

fn record_capture(concede: &Concede, input: NewCapture) -> Result<()> {
    concede.record_capture(input)?;
    Ok(())
}

/// The nearest existing ancestor directory of `path` (the file may have been deleted).
fn existing_parent(path: &Path) -> PathBuf {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d.is_dir() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    PathBuf::from(".")
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

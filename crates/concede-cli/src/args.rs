//! Command-line arguments.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Decision log for AI-assisted coding: why every change was made.
#[derive(Debug, Parser)]
#[command(name = "concede", version, about, long_about = None)]
pub struct Cli {
    /// Repository to use (default: the one containing the current directory).
    #[arg(long, global = true, value_name = "DIR")]
    pub repo: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Record a decision: what changed and why.
    Log(LogArgs),

    /// Show which decisions shaped a file or line range.
    #[command(
        after_help = "Examples:\n  concede why src/Order.cs\n  concede why src/Order.cs:42\n  concede why src/Order.cs:40-60"
    )]
    Why {
        /// FILE, FILE:LINE or FILE:START-END (current line numbers).
        target: String,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },

    /// List unreviewed decisions and changes nobody explained.
    Inbox {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },

    /// Group decisions into change stories, by intent (then session).
    Stories {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },

    /// Approve or reject a decision, or an unexplained change.
    Review {
        /// Decision or capture id; a unique prefix is enough.
        id: String,
        /// Approve it.
        #[arg(long, conflicts_with = "reject")]
        approve: bool,
        /// Reject it.
        #[arg(long)]
        reject: bool,
        /// A note stored with the review.
        #[arg(long)]
        note: Option<String>,
        /// Who is reviewing (default: git user.name).
        #[arg(long)]
        actor: Option<String>,
    },

    /// Link decisions to the HEAD commit (default: every approved, unlinked decision).
    Link {
        /// Decision ids (unique prefixes are enough).
        ids: Vec<String>,
        /// Who is linking (default: git user.name).
        #[arg(long)]
        actor: Option<String>,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },

    /// Record an edit from a Claude Code PostToolUse hook (reads the hook JSON on stdin).
    ///
    /// Always exits 0 so a problem never interrupts the agent; failures are written to
    /// .concede/capture-errors.log.
    Capture,

    /// Run the MCP server over stdio, exposing `record_decision` and `why` to agents.
    Mcp,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Example:\n  concede log --task \"Add retry to Submit\" \\\n    --reasoning \"Submit fails on transient network errors\" \\\n    --file src/Order.cs:40-52 \\\n    --alternative \"Polly policy: new dependency for one call site\"\n\nWith --stdin, pass the same fields as JSON:\n  {\"task\": \"...\", \"reasoning\": \"...\", \"files\": [{\"path\": \"src/Order.cs\", \"lines\": [\"40-52\"]}],\n   \"alternatives\": [{\"option\": \"...\", \"rejected_because\": \"...\"}], \"allowed_by\": \"...\",\n   \"agent\": {\"tool\": \"claude-code\", \"model\": \"...\", \"session\": \"...\"}, \"claim\": [\"<capture id>\"]}"
)]
pub struct LogArgs {
    /// The task or prompt this change served.
    #[arg(long, required_unless_present = "stdin")]
    pub task: Option<String>,

    /// The intent (change story) this change belongs to: a short label grouping
    /// decisions that serve one purpose, e.g. "order-validation".
    #[arg(long)]
    pub intent: Option<String>,

    /// Why the change was made.
    #[arg(long, required_unless_present = "stdin")]
    pub reasoning: Option<String>,

    /// A file the change touched: PATH, PATH:LINE or PATH:START-END. Repeatable.
    #[arg(long = "file", value_name = "PATH[:LINES]")]
    pub files: Vec<String>,

    /// A rejected option as "OPTION: REASON". Repeatable.
    #[arg(long = "alternative", value_name = "OPTION: REASON")]
    pub alternatives: Vec<String>,

    /// The rule that allowed the change (default: "unconfigured").
    #[arg(long)]
    pub allowed_by: Option<String>,

    /// The agent tool that made the change, e.g. claude-code. Omit for a human decision.
    #[arg(long)]
    pub agent: Option<String>,

    /// The model id, when an agent made the change.
    #[arg(long, requires = "agent")]
    pub model: Option<String>,

    /// The agent session id, used to group work and match captured edits.
    #[arg(long, requires = "agent")]
    pub session: Option<String>,

    /// An unexplained capture this decision explains. Repeatable.
    #[arg(long = "claim", value_name = "CAPTURE_ID")]
    pub claims: Vec<String>,

    /// Read the decision as JSON from stdin instead of from flags.
    #[arg(long, conflicts_with_all = ["task", "intent", "reasoning", "files", "alternatives", "agent", "claims"])]
    pub stdin: bool,

    /// Print the recorded decision as JSON.
    #[arg(long)]
    pub json: bool,
}

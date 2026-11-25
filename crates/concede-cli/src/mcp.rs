//! `concede mcp`: an MCP server over stdio for agents (ADR 0007).
//!
//! Tools:
//! - `record_decision`: log what was changed and why, right after making the change.
//! - `why`: read the decision history of a file or line range before changing it.
//!
//! The session id comes from the `CLAUDE_CODE_SESSION_ID` environment variable, which
//! Claude Code sets for MCP servers and which matches the `session_id` in its hook
//! payloads (checked against a real session). That lets a decision claim exactly its
//! own session's captured edits.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use concede_core::{
    AgentInfo, Alternative, Concede, FileRef, LineRange, NewDecision, Source, hooks,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::{input, output};

/// Environment variable Claude Code sets for MCP servers it starts.
const SESSION_ENV: &str = "CLAUDE_CODE_SESSION_ID";

const INSTRUCTIONS: &str = "Concede keeps a decision log in this repository: why each change was made. \
After each logical change (one intent, possibly several files), call record_decision with the task, \
your reasoning, the files you changed and the alternatives you rejected. Before changing code you \
didn't write, call why on it to see earlier decisions. Edits are also captured automatically by a \
hook, but only you can say why.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FileParam {
    /// Path of a file you changed, relative to the repository root (or absolute).
    pub path: String,
    /// Optional line ranges you changed, in current line numbers: "42" or "40-52".
    /// Omit to let Concede work out the changed lines from your captured edits.
    #[serde(default)]
    pub lines: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AlternativeParam {
    /// An approach you considered.
    pub option: String,
    /// Why you didn't take it.
    pub rejected_because: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecordDecisionParams {
    /// The task or user request this change served, in one sentence.
    pub task: String,
    /// The intent (change story) this change belongs to: a short, stable label you
    /// reuse across the decisions that serve one purpose, e.g. "order-validation".
    /// Changes sharing an intent are grouped into one story for review.
    #[serde(default)]
    pub intent: Option<String>,
    /// Why you made this change this way: the reasoning a reviewer needs.
    pub reasoning: String,
    /// The files this change touched.
    pub files: Vec<FileParam>,
    /// Approaches you considered and rejected.
    #[serde(default)]
    pub alternatives: Vec<AlternativeParam>,
    /// The rule or permission that allowed this action, if you were given one.
    #[serde(default)]
    pub allowed_by: Option<String>,
    /// Your model id, e.g. "claude-opus-5-5".
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WhyParams {
    /// File path, relative to the repository root (or absolute).
    pub path: String,
    /// Optional line range in current line numbers: "42" or "40-52".
    #[serde(default)]
    pub lines: Option<String>,
}

#[derive(Clone)]
pub struct ConcedeServer {
    concede: Arc<Concede>,
    root: PathBuf,
    session: Option<String>,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl ConcedeServer {
    fn new(concede: Concede, session: Option<String>) -> Self {
        let root = concede
            .store()
            .dir()
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        Self {
            concede: Arc::new(concede),
            root,
            session,
            tool_router: Self::tool_router(),
        }
    }

    /// Record why you made a change. Call this after each logical change, once the edits
    /// are done. The changed lines are taken from your captured edits where possible.
    #[tool(name = "record_decision")]
    async fn record_decision(
        &self,
        Parameters(params): Parameters<RecordDecisionParams>,
    ) -> Result<String, String> {
        let new = self.to_new_decision(params)?;
        let concede = Arc::clone(&self.concede);
        // Core operations spawn git and touch the disk: keep them off the async runtime.
        let decision = tokio::task::spawn_blocking(move || concede.record_decision(new))
            .await
            .map_err(|e| format!("internal error: {e}"))?
            .map_err(|e| format!("{e}"))?;

        let mut reply = format!("Recorded decision {}.", decision.id);
        for change in &decision.changes {
            let lines: Vec<String> = change
                .hunks
                .iter()
                .map(|h| h.new_side().to_string())
                .collect();
            let lines = if lines.is_empty() {
                "no changed lines found".into()
            } else {
                lines.join(", ")
            };
            reply.push_str(&format!("\n- {}: lines {lines}", change.path));
        }
        if !decision.captures.is_empty() {
            reply.push_str(&format!(
                "\nIt explains {} captured edit(s).",
                decision.captures.len()
            ));
        }
        reply.push_str("\nA human will review it with `concede inbox`.");
        Ok(reply)
    }

    /// Show the recorded decisions (task, reasoning, rejected alternatives, review status)
    /// behind a file or line range. Use it before changing code you didn't write.
    #[tool(name = "why")]
    async fn why(&self, Parameters(params): Parameters<WhyParams>) -> Result<String, String> {
        let lines = match params
            .lines
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            Some(text) => Some(parse_lines(text)?),
            None => None,
        };
        let path = self.absolute(&params.path);
        let concede = Arc::clone(&self.concede);
        let why = tokio::task::spawn_blocking(move || concede.why(&path, lines))
            .await
            .map_err(|e| format!("internal error: {e}"))?
            .map_err(|e| format!("{e}"))?;
        serde_json::to_string_pretty(&output::why_json(&why)).map_err(|e| e.to_string())
    }
}

impl ConcedeServer {
    fn absolute(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        }
    }

    fn to_new_decision(&self, params: RecordDecisionParams) -> Result<NewDecision, String> {
        let files = params
            .files
            .into_iter()
            .map(|f| {
                let lines = f
                    .lines
                    .iter()
                    .map(|l| parse_lines(l))
                    .collect::<Result<_, _>>()?;
                Ok(FileRef {
                    path: self.absolute(&f.path),
                    lines,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(NewDecision {
            source: Source::Agent,
            agent: Some(AgentInfo {
                tool: hooks::CLAUDE_CODE.into(),
                model: params.model.filter(|m| !m.trim().is_empty()),
                session: self.session.clone(),
            }),
            task: params.task,
            intent: params.intent.filter(|i| !i.trim().is_empty()),
            reasoning: params.reasoning,
            alternatives: params
                .alternatives
                .into_iter()
                .map(|a| Alternative {
                    option: a.option,
                    rejected_because: a.rejected_because,
                })
                .collect(),
            allowed_by: params.allowed_by.filter(|a| !a.trim().is_empty()),
            files,
            claim: Vec::new(),
        })
    }
}

fn parse_lines(text: &str) -> Result<LineRange, String> {
    match input::parse_target(&format!("x:{text}")) {
        Ok((_, Some(lines))) => Ok(lines),
        _ => Err(format!(
            "invalid line range {text:?}; use \"42\" or \"40-52\""
        )),
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ConcedeServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("concede", concede_core::VERSION))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Run the MCP server on stdin/stdout until the client disconnects.
pub fn serve(concede: Concede) -> anyhow::Result<()> {
    let session = std::env::var(SESSION_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty());
    let server = ConcedeServer::new(concede, session);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("can't start the async runtime")?;
    runtime.block_on(async move {
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .context("MCP handshake failed")?;
        service
            .waiting()
            .await
            .context("MCP server stopped unexpectedly")?;
        Ok(())
    })
}

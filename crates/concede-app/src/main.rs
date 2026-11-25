//! Concede desktop app. Thin GPUI layer over `concede-core`.
//!
//! Phase 2:
//! - step 2: the story inbox — unreviewed decisions grouped by agent session.
//! - step 3: a detail pane — selecting a decision shows its reasoning, the
//!   alternatives it rejected, and the touched code read-only with the changed
//!   lines highlighted (ranges mapped onto the current file via `why`).
//!
//! Selection is by click for now; keyboard navigation comes in a later step.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};
use concede_core::{ChangeStore, Concede, Decision, MappedHunk, Mapping, PRODUCT_NAME, WhyRecord};
use gpui::{
    App, Bounds, Context, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};

// A small dark palette. Colours live here so later steps (and a light theme) have
// one place to change.
const BG: u32 = 0x1e1e22;
const PANEL: u32 = 0x26262b;
const DIVIDER: u32 = 0x33333a;
const ROW_SELECTED: u32 = 0x33406a;
const ROW_HOVER: u32 = 0x2e2e34;
const CODE_BG: u32 = 0x202024;
const CODE_HIGHLIGHT: u32 = 0x3a3b2a;
const TEXT: u32 = 0xf0f0f0;
const MUTED: u32 = 0x9a9aa2;
const FAINT: u32 = 0x6c6c74;
const ACCENT: u32 = 0x8ab4ff;

/// One unreviewed decision in the list, flattened to owned strings.
#[derive(Debug, Clone)]
struct DecisionRow {
    short_id: String,
    when: String,
    task: String,
    files: String,
}

/// One agent session's unreviewed decisions.
#[derive(Debug, Clone)]
struct SessionRow {
    heading: String,
    decisions: Vec<DecisionRow>,
}

/// One line of source in the detail pane's code view.
#[derive(Debug, Clone)]
struct CodeLine {
    number: u32,
    text: String,
    highlighted: bool,
}

/// One touched file in the detail pane: its path, a human label for the changed
/// lines, and the current file content (empty when the file can't be read).
#[derive(Debug, Clone)]
struct FileDetail {
    path: String,
    lines_label: String,
    code: Vec<CodeLine>,
    note: Option<String>,
}

/// The full, owned detail for one decision, shown on the right when selected.
#[derive(Debug, Clone)]
struct DecisionDetail {
    task: String,
    meta: String,
    reasoning: String,
    alternatives: Vec<(String, String)>,
    files: Vec<FileDetail>,
}

/// An owned snapshot of the inbox. The GPUI view holds this across frames, so it
/// cannot borrow from the `Log` the way `concede_core::Inbox` does.
#[derive(Debug, Clone, Default)]
struct AppData {
    sessions: Vec<SessionRow>,
    /// Flat, in list order, so the selected index addresses both.
    details: Vec<DecisionDetail>,
    unexplained: usize,
    /// Set when there's nothing to show; the body shows this instead of the panes.
    message: Option<String>,
}

impl AppData {
    fn total(&self) -> usize {
        self.details.len()
    }
}

/// Read the decision log at `root` and build both the list rows and the per-decision
/// detail. Any failure, or an empty/absent log, becomes a `message` rather than a
/// panic, so a plain folder just shows an explained state.
fn load_app(root: &Path) -> AppData {
    let concede = match Concede::discover(root) {
        Ok(concede) => concede,
        Err(error) => {
            return AppData {
                message: Some(format!("Not a git repository here: {error}")),
                ..Default::default()
            };
        }
    };

    // Distinguish "no decision log at all" from "a log with nothing to review".
    let repo_root = concede.vcs().root().to_path_buf();
    if !repo_root.join(".concede").exists() {
        return AppData {
            message: Some(format!(
                "No decision log in {}.\nRun a Claude Code session here (see examples/try-it.sh) to create one.",
                repo_root.display()
            )),
            ..Default::default()
        };
    }

    let loaded = match concede.load() {
        Ok(loaded) => loaded,
        Err(error) => {
            return AppData {
                message: Some(format!("Couldn't read the decision log: {error}")),
                ..Default::default()
            };
        }
    };

    let inbox = loaded.log.inbox();
    let mut sessions = Vec::with_capacity(inbox.sessions.len());
    let mut details = Vec::new();

    for group in &inbox.sessions {
        let tools: BTreeSet<String> = group.decisions.iter().map(|d| agent_label(d)).collect();
        let tools = tools.into_iter().collect::<Vec<_>>().join(", ");
        let heading = match group.session {
            Some(session) => format!("Session {session}  ({tools})"),
            None => format!("No session  ({tools})"),
        };

        let mut rows = Vec::with_capacity(group.decisions.len());
        for d in &group.decisions {
            rows.push(DecisionRow {
                short_id: short_id(&d.id.to_string()),
                when: local(&d.timestamp),
                task: d.task.clone(),
                files: files_label(d),
            });
            details.push(build_detail(&concede, &repo_root, d));
        }

        sessions.push(SessionRow {
            heading,
            decisions: rows,
        });
    }

    AppData {
        sessions,
        details,
        unexplained: inbox.unexplained.len(),
        message: None,
    }
}

/// Build the detail for one decision: reasoning, alternatives, and each touched
/// file's current content with the decision's changed lines highlighted.
fn build_detail(
    concede: &Concede<concede_core::GitCli>,
    root: &Path,
    d: &Decision,
) -> DecisionDetail {
    let alternatives = d
        .alternatives
        .iter()
        .map(|a| (a.option.clone(), a.rejected_because.clone()))
        .collect();

    let files = d
        .changes
        .iter()
        .map(|change| {
            let path = change.path.to_string();
            let absolute = root.join(&path);

            // Ask `why` where this decision's recorded hunks are in the file now.
            // (why() reloads the log; fine for the small logs we have today.)
            let mapped: Vec<MappedHunk> = concede
                .why(&absolute, None)
                .ok()
                .and_then(|why| {
                    why.entries
                        .into_iter()
                        .find_map(|entry| match entry.record {
                            WhyRecord::Decision { decision, .. } if decision.id == d.id => {
                                Some(entry.hunks)
                            }
                            _ => None,
                        })
                })
                .unwrap_or_default();

            let highlights = highlight_lines(&mapped);
            let (code, note) = match std::fs::read_to_string(&absolute) {
                Ok(content) => (code_lines(&content, &highlights), None),
                Err(_) => (Vec::new(), Some("(file not found on disk)".to_owned())),
            };

            FileDetail {
                path,
                lines_label: lines_label(&mapped),
                code,
                note,
            }
        })
        .collect();

    DecisionDetail {
        task: d.task.clone(),
        meta: format!("{} · {} · unreviewed", agent_label(d), local(&d.timestamp)),
        reasoning: d.reasoning.clone(),
        alternatives,
        files,
    }
}

/// The set of current line numbers this decision's hunks cover (for highlighting).
fn highlight_lines(hunks: &[MappedHunk]) -> BTreeSet<u32> {
    let mut lines = BTreeSet::new();
    for hunk in hunks {
        if let Some(range) = hunk.current.range {
            for line in range.start..=range.end {
                lines.insert(line);
            }
        }
    }
    lines
}

/// Split file content into numbered lines, marking the highlighted ones.
fn code_lines(content: &str, highlights: &BTreeSet<u32>) -> Vec<CodeLine> {
    content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let number = i as u32 + 1;
            CodeLine {
                number,
                // Keep the line non-empty so blank lines still take a row's height.
                text: if line.is_empty() {
                    " ".to_owned()
                } else {
                    line.to_owned()
                },
                highlighted: highlights.contains(&number),
            }
        })
        .collect()
}

/// A short label for a decision's changed lines, e.g. `52-64, 71` or `whole file`.
fn lines_label(hunks: &[MappedHunk]) -> String {
    if hunks.is_empty() {
        return "no changed lines recorded".to_owned();
    }
    let parts: Vec<String> = hunks
        .iter()
        .map(|hunk| match (hunk.current.mapping, hunk.current.range) {
            (Mapping::Gone, _) | (_, None) => "since deleted".to_owned(),
            (_, Some(range)) if range.start == range.end => range.start.to_string(),
            (_, Some(range)) => format!("{}-{}", range.start, range.end),
        })
        .collect();
    parts.join(", ")
}

/// `tool (model)` or just `tool`, matching the CLI's inbox.
fn agent_label(decision: &Decision) -> String {
    match &decision.agent {
        Some(agent) => match &agent.model {
            Some(model) => format!("{} ({model})", agent.tool),
            None => agent.tool.clone(),
        },
        None => "human".to_owned(),
    }
}

/// The local-time stamp the inbox shows, same format as the CLI.
fn local(ts: &DateTime<Utc>) -> String {
    ts.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// A fixed 12-char id prefix for display (ULIDs are unique well within 12 chars;
/// the CLI computes the shortest unique prefix, which the app doesn't need yet).
fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

/// `src/Order.cs`, `src/Order.cs (+3 more)`, or `no files`.
fn files_label(decision: &Decision) -> String {
    let mut paths = decision.changes.iter().map(|c| c.path.to_string());
    match paths.next() {
        None => "no files".to_owned(),
        Some(first) => {
            let rest = paths.count();
            if rest == 0 {
                first
            } else {
                format!("{first} (+{rest} more)")
            }
        }
    }
}

/// Root view: the story inbox (left) and the selected decision's detail (right).
struct ConcedeRoot {
    title: SharedString,
    data: AppData,
    /// Flat index of the selected decision across all sessions.
    selected: usize,
}

impl ConcedeRoot {
    fn new(data: AppData) -> Self {
        Self {
            title: PRODUCT_NAME.into(),
            data,
            selected: 0,
        }
    }
}

impl Render for ConcedeRoot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .flex()
            .items_baseline()
            .gap_3()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .child(
                div()
                    .text_xl()
                    .text_color(rgb(TEXT))
                    .child(self.title.clone()),
            )
            .child(div().text_sm().text_color(rgb(MUTED)).child(format!(
                "{} unreviewed · {} unexplained",
                self.data.total(),
                self.data.unexplained
            )));

        let body = if let Some(message) = &self.data.message {
            centered_message(message.clone())
        } else if self.data.total() == 0 {
            centered_message("Inbox zero: nothing to review.".to_owned())
        } else {
            div()
                .flex()
                .flex_1()
                .child(self.inbox_list(cx))
                .child(self.detail_pane())
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .child(header)
            .child(body)
    }
}

impl ConcedeRoot {
    /// The scrollable, grouped list of unreviewed decisions (left pane).
    fn inbox_list(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let selected = self.selected;
        let mut flat = 0usize;
        let mut sessions = Vec::with_capacity(self.data.sessions.len());

        for session in &self.data.sessions {
            let mut rows = Vec::with_capacity(session.decisions.len());
            for row in &session.decisions {
                let index = flat;
                flat += 1;
                rows.push(
                    div()
                        .id(("decision", index))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(if index == selected {
                            ROW_SELECTED
                        } else {
                            PANEL
                        }))
                        .hover(|style| style.bg(rgb(ROW_HOVER)))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.selected = index;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(div().text_color(rgb(ACCENT)).child(row.short_id.clone()))
                                .child(div().text_color(rgb(MUTED)).child(row.when.clone())),
                        )
                        .child(div().child(row.task.clone()))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .child(row.files.clone()),
                        ),
                );
            }

            sessions.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(session.heading.clone()),
                    )
                    .child(div().flex().flex_col().gap_1().children(rows)),
            );
        }

        div()
            .id("inbox-list")
            .flex()
            .flex_col()
            .w(px(380.))
            .h_full()
            .gap_4()
            .p_4()
            .border_r_1()
            .border_color(rgb(DIVIDER))
            .overflow_y_scroll()
            .children(sessions)
            .into_any_element()
    }

    /// The detail of the selected decision (right pane).
    fn detail_pane(&self) -> gpui::AnyElement {
        let Some(detail) = self.data.details.get(self.selected) else {
            return div().flex_1().into_any_element();
        };

        let mut sections = Vec::new();

        sections.push(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xl().child(detail.task.clone()))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(detail.meta.clone()),
                )
                .into_any_element(),
        );

        if !detail.reasoning.is_empty() {
            sections.push(section("Reasoning", div().child(detail.reasoning.clone())));
        }

        if !detail.alternatives.is_empty() {
            let items = detail.alternatives.iter().map(|(option, because)| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_color(rgb(TEXT)).child(option.clone()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(format!("rejected — {because}")),
                    )
            });
            sections.push(section(
                "Rejected alternatives",
                div().flex().flex_col().gap_3().children(items),
            ));
        }

        for file in &detail.files {
            sections.push(file_section(file));
        }

        div()
            .id("detail")
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .gap_5()
            .p_5()
            .overflow_y_scroll()
            .children(sections)
            .into_any_element()
    }
}

/// A titled section in the detail pane.
fn section(title: &str, body: impl IntoElement) -> gpui::AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(rgb(FAINT))
                .child(title.to_uppercase()),
        )
        .child(body)
        .into_any_element()
}

/// A touched file: its path and changed-line label, then the code with the
/// changed lines highlighted (or a note when the file can't be read).
fn file_section(file: &FileDetail) -> gpui::AnyElement {
    let heading = div()
        .flex()
        .gap_3()
        .items_baseline()
        .child(div().text_color(rgb(ACCENT)).child(file.path.clone()))
        .child(
            div()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(format!("lines {}", file.lines_label)),
        );

    let body = if let Some(note) = &file.note {
        div().text_sm().text_color(rgb(MUTED)).child(note.clone())
    } else {
        let rows = file.code.iter().map(|line| {
            div()
                .flex()
                .gap_3()
                .when(line.highlighted, |row| row.bg(rgb(CODE_HIGHLIGHT)))
                .child(
                    div()
                        .w(px(44.))
                        .text_color(rgb(FAINT))
                        .child(format!("{:>4}", line.number)),
                )
                .child(div().text_color(rgb(TEXT)).child(line.text.clone()))
        });
        div()
            .flex()
            .flex_col()
            .p_3()
            .rounded_md()
            .bg(rgb(CODE_BG))
            .text_sm()
            .children(rows)
    };

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(heading)
        .child(body)
        .into_any_element()
}

/// A single centred block of text (empty state / message), filling the body.
fn centered_message(text: String) -> gpui::AnyElement {
    div()
        .flex()
        .flex_1()
        .w_full()
        .justify_center()
        .items_center()
        .p_8()
        .text_color(rgb(MUTED))
        .child(text)
        .into_any_element()
}

fn main() {
    // The repo to open: the first CLI argument, or the current directory.
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));

    // Load once, before the first frame. The log is a handful of small JSON files,
    // so this is fast; live reloading will move to a background task when we add
    // file-watching in a later step.
    let data = load_app(&root);

    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(PRODUCT_NAME.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        // Failing to open the main window at startup is unrecoverable, so a panic with a
        // clear message is the right outcome here.
        cx.open_window(options, |_, cx| cx.new(|_| ConcedeRoot::new(data)))
            .expect("failed to open the main window");
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use concede_core::{AgentInfo, Change, RepoPath, Source};

    fn decision_with_paths(paths: &[&str]) -> Decision {
        let mut decision = Decision::new(Source::Agent, "task", "rule");
        decision.changes = paths
            .iter()
            .map(|p| Change {
                path: RepoPath::new(*p).expect("valid path"),
                after_blob: None,
                hunks: Vec::new(),
            })
            .collect();
        decision
    }

    #[test]
    fn files_label_covers_none_one_and_many() {
        assert_eq!(files_label(&decision_with_paths(&[])), "no files");
        assert_eq!(
            files_label(&decision_with_paths(&["src/Order.cs"])),
            "src/Order.cs"
        );
        assert_eq!(
            files_label(&decision_with_paths(&["src/Order.cs", "a", "b"])),
            "src/Order.cs (+2 more)"
        );
    }

    #[test]
    fn agent_label_uses_tool_model_or_human() {
        let mut with_model = decision_with_paths(&[]);
        with_model.agent = Some(AgentInfo {
            tool: "claude-code".to_owned(),
            model: Some("claude-haiku-5-5".to_owned()),
            session: None,
        });
        assert_eq!(agent_label(&with_model), "claude-code (claude-haiku-5-5)");

        let mut tool_only = decision_with_paths(&[]);
        tool_only.agent = Some(AgentInfo {
            tool: "claude-code".to_owned(),
            model: None,
            session: None,
        });
        assert_eq!(agent_label(&tool_only), "claude-code");

        let human = decision_with_paths(&[]);
        assert_eq!(agent_label(&human), "human");
    }

    #[test]
    fn short_id_takes_twelve_chars() {
        assert_eq!(short_id("01M87W8RM73WPD4TMG7C54V6X1"), "01M87W8RM73W");
        assert_eq!(short_id("abc"), "abc");
    }

    #[test]
    fn code_lines_number_and_highlight() {
        let highlights = BTreeSet::from([2u32]);
        let lines = code_lines("a\n\nc", &highlights);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].number, 1);
        assert!(!lines[0].highlighted);
        assert_eq!(lines[1].text, " "); // blank line kept non-empty
        assert!(lines[1].highlighted);
        assert_eq!(lines[2].text, "c");
    }
}

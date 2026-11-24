//! Concede desktop app. Thin GPUI layer over `concede-core`.
//!
//! Phase 2, step 2: the story inbox. On launch we open the repo in the current
//! directory, read the decision log once, and render the unreviewed decisions
//! grouped by agent session — the visual twin of `concede inbox`. Selection is
//! by click for now; keyboard navigation and a detail pane come in later steps.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, Local, Utc};
use concede_core::{Concede, Decision, PRODUCT_NAME};
use gpui::{
    App, Bounds, Context, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};

// A small dark palette. Colours live here so later steps (and a light theme) have
// one place to change.
const BG: u32 = 0x1e1e22;
const PANEL: u32 = 0x26262b;
const ROW_SELECTED: u32 = 0x33406a;
const ROW_HOVER: u32 = 0x2e2e34;
const TEXT: u32 = 0xf0f0f0;
const MUTED: u32 = 0x9a9aa2;
const ACCENT: u32 = 0x8ab4ff;

/// One unreviewed decision, flattened to owned, display-ready strings.
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

/// An owned snapshot of the inbox. The GPUI view holds this across frames, so it
/// cannot borrow from the `Log` the way `concede_core::Inbox` does.
#[derive(Debug, Clone, Default)]
struct InboxView {
    sessions: Vec<SessionRow>,
    unexplained: usize,
    /// Set when the repo or log couldn't be read; shown instead of the list.
    error: Option<String>,
}

impl InboxView {
    fn total(&self) -> usize {
        self.sessions.iter().map(|s| s.decisions.len()).sum()
    }
}

/// Read the decision log at `root` and flatten its inbox into owned rows. Any
/// failure (not a repo, unreadable log) becomes `error` rather than a panic, so a
/// plain folder just shows an empty, explained state.
fn load_inbox(root: &Path) -> InboxView {
    let concede = match Concede::discover(root) {
        Ok(concede) => concede,
        Err(error) => {
            return InboxView {
                error: Some(format!("Not a Concede repository here: {error}")),
                ..Default::default()
            };
        }
    };
    let loaded = match concede.load() {
        Ok(loaded) => loaded,
        Err(error) => {
            return InboxView {
                error: Some(format!("Couldn't read the decision log: {error}")),
                ..Default::default()
            };
        }
    };

    let inbox = loaded.log.inbox();
    let sessions = inbox
        .sessions
        .iter()
        .map(|group| {
            let tools: BTreeSet<String> = group.decisions.iter().map(|d| agent_label(d)).collect();
            let tools = tools.into_iter().collect::<Vec<_>>().join(", ");
            let heading = match group.session {
                Some(session) => format!("Session {session}  ({tools})"),
                None => format!("No session  ({tools})"),
            };
            let decisions = group
                .decisions
                .iter()
                .map(|d| DecisionRow {
                    short_id: short_id(&d.id.to_string()),
                    when: local(&d.timestamp),
                    task: d.task.clone(),
                    files: files_label(d),
                })
                .collect();
            SessionRow { heading, decisions }
        })
        .collect();

    InboxView {
        sessions,
        unexplained: inbox.unexplained.len(),
        error: None,
    }
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

/// Root view: the story inbox.
struct ConcedeRoot {
    title: SharedString,
    inbox: InboxView,
    /// Flat index of the selected decision across all sessions.
    selected: usize,
}

impl ConcedeRoot {
    fn new(inbox: InboxView) -> Self {
        Self {
            title: PRODUCT_NAME.into(),
            inbox,
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
            .border_color(rgb(PANEL))
            .child(
                div()
                    .text_xl()
                    .text_color(rgb(TEXT))
                    .child(self.title.clone()),
            )
            .child(div().text_sm().text_color(rgb(MUTED)).child(format!(
                "{} unreviewed · {} unexplained",
                self.inbox.total(),
                self.inbox.unexplained
            )));

        let body = if let Some(error) = &self.inbox.error {
            centered_message(error.clone(), MUTED)
        } else if self.inbox.total() == 0 {
            centered_message("Inbox zero: nothing to review.".to_owned(), MUTED)
        } else {
            self.inbox_list(cx)
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
    /// The scrollable, grouped list of unreviewed decisions.
    fn inbox_list(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let selected = self.selected;
        let mut flat = 0usize;
        let mut sessions = Vec::with_capacity(self.inbox.sessions.len());

        for session in &self.inbox.sessions {
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
            .flex_1()
            .gap_4()
            .p_4()
            .overflow_y_scroll()
            .children(sessions)
            .into_any_element()
    }
}

/// A single centred line of text (empty state / error), filling the body.
fn centered_message(text: String, color: u32) -> gpui::AnyElement {
    div()
        .flex()
        .flex_1()
        .w_full()
        .justify_center()
        .items_center()
        .text_color(rgb(color))
        .child(text)
        .into_any_element()
}

fn main() {
    // Load the inbox once, before the first frame. The log is a handful of small
    // JSON files, so this is sub-millisecond; live reloading will move to a
    // background task when we add file-watching in a later step.
    let root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let inbox = load_inbox(&root);

    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);
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
        cx.open_window(options, |_, cx| cx.new(|_| ConcedeRoot::new(inbox)))
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

        let mut human = decision_with_paths(&[]);
        human.agent = None;
        assert_eq!(agent_label(&human), "human");
    }

    #[test]
    fn short_id_takes_twelve_chars() {
        assert_eq!(short_id("01M87W8RM73WPD4TMG7C54V6X1"), "01M87W8RM73W");
        assert_eq!(short_id("abc"), "abc");
    }
}

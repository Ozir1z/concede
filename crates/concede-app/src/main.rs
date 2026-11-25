//! Concede desktop app. Thin GPUI layer over `concede-core`.
//!
//! Phase 2:
//! - step 2: the story inbox — unreviewed decisions grouped by agent session.
//! - step 3: a detail pane — selecting a decision shows its reasoning, the
//!   alternatives it rejected, and the touched code read-only with the changed
//!   lines highlighted (ranges mapped onto the current file via `why`).
//!   Draggable scrollbars: vertical on the list and detail panes, horizontal on
//!   each code block (lines don't wrap).
//!
//! Selection is by click for now; keyboard navigation comes in a later step.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};
use concede_core::{
    ChangeStore, Concede, Decision, MappedHunk, Mapping, PRODUCT_NAME, ReviewStatus, WhyRecord,
};
use gpui::{
    App, Bounds, Context, FocusHandle, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ScrollHandle, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions,
    actions, div, point, prelude::*, px, rgb, size,
};

// Keyboard actions, dispatched to the root view. Names become e.g.
// `concede::SelectNext`; the keymap binds keystrokes to them.
actions!(
    concede,
    [
        SelectNext,
        SelectPrev,
        FocusDetail,
        FocusList,
        Approve,
        Reject
    ]
);

/// Which pane the keyboard drives: moving the selection, or scrolling the detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    List,
    Detail,
}

/// Keyboard configuration: which preset, plus any per-action overrides from the
/// config file. See [`load_key_config`].
#[derive(Debug, Clone, Default)]
struct KeyConfig {
    vim: bool,
    /// `(action name, keystroke)` pairs added on top of the preset.
    overrides: Vec<(String, String)>,
}

/// Load the keymap config. Looked up at `$CONCEDE_KEYS`, else
/// `$HOME/.config/concede/keymap.conf`. Missing file → built-in defaults.
///
/// Format (one per line, `#` comments):
///   mode = vim | default
///   bind <action> = <keystroke>     e.g. `bind approve = shift-y`
/// Actions: select_next, select_prev, focus_detail, focus_list, approve, reject.
fn load_key_config() -> KeyConfig {
    let path = std::env::var_os("CONCEDE_KEYS")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".config/concede/keymap.conf"))
        });
    let Some(path) = path else {
        return KeyConfig::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return KeyConfig::default();
    };

    let mut config = KeyConfig::default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("bind ") {
            if let Some((action, key)) = rest.split_once('=') {
                config
                    .overrides
                    .push((action.trim().to_owned(), key.trim().to_owned()));
            }
        } else if let Some((key, value)) = line.split_once('=')
            && key.trim() == "mode"
        {
            config.vim = value.trim().eq_ignore_ascii_case("vim");
        }
    }
    config
}

/// Build the key bindings from the config: a preset (default or vim) plus any
/// overrides. Keystrokes must be `'static`, so overridden ones are leaked (they
/// live for the whole app run anyway).
fn key_bindings(config: &KeyConfig) -> Vec<KeyBinding> {
    const CONTEXT: Option<&str> = Some("ConcedeRoot");
    let mut binds: Vec<KeyBinding> = if config.vim {
        vec![
            KeyBinding::new("j", SelectNext, CONTEXT),
            KeyBinding::new("k", SelectPrev, CONTEXT),
            KeyBinding::new("down", SelectNext, CONTEXT),
            KeyBinding::new("up", SelectPrev, CONTEXT),
            KeyBinding::new("l", FocusDetail, CONTEXT),
            KeyBinding::new("enter", FocusDetail, CONTEXT),
            KeyBinding::new("h", FocusList, CONTEXT),
            KeyBinding::new("escape", FocusList, CONTEXT),
            KeyBinding::new("a", Approve, CONTEXT),
            KeyBinding::new("r", Reject, CONTEXT),
        ]
    } else {
        vec![
            KeyBinding::new("down", SelectNext, CONTEXT),
            KeyBinding::new("up", SelectPrev, CONTEXT),
            KeyBinding::new("enter", FocusDetail, CONTEXT),
            KeyBinding::new("escape", FocusList, CONTEXT),
            KeyBinding::new("shift-a", Approve, CONTEXT),
            KeyBinding::new("shift-r", Reject, CONTEXT),
        ]
    };

    for (action, key) in &config.overrides {
        let key: &'static str = Box::leak(key.clone().into_boxed_str());
        let bind = match action.as_str() {
            "select_next" => KeyBinding::new(key, SelectNext, CONTEXT),
            "select_prev" => KeyBinding::new(key, SelectPrev, CONTEXT),
            "focus_detail" => KeyBinding::new(key, FocusDetail, CONTEXT),
            "focus_list" => KeyBinding::new(key, FocusList, CONTEXT),
            "approve" => KeyBinding::new(key, Approve, CONTEXT),
            "reject" => KeyBinding::new(key, Reject, CONTEXT),
            _ => continue,
        };
        binds.push(bind);
    }
    binds
}

// A small dark palette. Colours live here so later steps (and a light theme) have
// one place to change.
const BG: u32 = 0x1e1e22;
const PANEL: u32 = 0x26262b;
const DIVIDER: u32 = 0x33333a;
const ROW_SELECTED: u32 = 0x33406a;
const ROW_HOVER: u32 = 0x2e2e34;
const CODE_BG: u32 = 0x202024;
const CODE_HIGHLIGHT: u32 = 0x3a3b2a;
const SCROLL_THUMB: u32 = 0x55555f;
const TEXT: u32 = 0xf0f0f0;
const MUTED: u32 = 0x9a9aa2;
const FAINT: u32 = 0x6c6c74;
const ACCENT: u32 = 0x8ab4ff;

/// Smallest scrollbar thumb, in pixels, so it stays grabbable on long content.
const MIN_THUMB: f32 = 28.0;

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
    /// The decision's full id, for the review call.
    id: String,
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

/// An in-progress scrollbar drag: which scroll handle, along which axis, and
/// where the grab started (so moves map to a new offset). `ScrollHandle` is an
/// `Rc`, so cloning it is cheap and shares the same scroll state.
#[derive(Clone)]
struct Drag {
    handle: ScrollHandle,
    vertical: bool,
    /// Mouse position along the axis when the grab started, in pixels.
    start_mouse: f32,
    /// The handle's offset along the axis when the grab started (<= 0).
    start_offset: f32,
    /// Track length minus thumb length: how far the thumb can travel.
    usable: f32,
    /// Maximum scroll distance (content length minus viewport).
    max: f32,
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
        id: d.id.to_string(),
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
    /// The repository root, kept so reviews can reopen the log and reload.
    root: PathBuf,
    data: AppData,
    /// Flat index of the selected decision across all sessions.
    selected: usize,
    /// Which pane the keyboard drives.
    focus_area: FocusArea,
    /// Whether the vim keymap is active (for the header hint).
    vim: bool,
    focus: FocusHandle,
    focused_once: bool,
    list_scroll: ScrollHandle,
    detail_scroll: ScrollHandle,
    /// One per file of the selected decision (for horizontal code scrolling).
    code_scrolls: Vec<ScrollHandle>,
    /// The scrollbar drag in progress, if any.
    drag: Option<Drag>,
    /// Scroll geometry is only known after the first layout, so we request one
    /// extra frame on startup to draw the scrollbars at their true sizes.
    primed: bool,
}

impl ConcedeRoot {
    fn new(root: PathBuf, data: AppData, vim: bool, cx: &mut Context<Self>) -> Self {
        Self {
            title: PRODUCT_NAME.into(),
            root,
            code_scrolls: code_scroll_pool(&data),
            data,
            selected: 0,
            focus_area: FocusArea::List,
            vim,
            focus: cx.focus_handle(),
            focused_once: false,
            list_scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            drag: None,
            primed: false,
        }
    }

    /// Reset the per-file scroll handles and the detail scroll when the selection
    /// changes, so the new decision starts scrolled to the top.
    fn select(&mut self, index: usize) {
        if index >= self.data.total() {
            return;
        }
        self.selected = index;
        // Keep the selected row visible in the list.
        self.list_scroll
            .scroll_to_item(self.list_child_index(index));
        // Reset scroll positions to the top/left, reusing the stable handles.
        self.detail_scroll.set_offset(point(px(0.0), px(0.0)));
        for handle in &self.code_scrolls {
            handle.set_offset(point(px(0.0), px(0.0)));
        }
        // The newly shown files have no measured geometry until the next layout,
        // so draw one more frame after it (as on startup) to size their
        // horizontal scrollbars correctly.
        self.primed = false;
    }

    fn select_next(&mut self) {
        match self.focus_area {
            FocusArea::List => self.select(self.selected.saturating_add(1)),
            FocusArea::Detail => self.scroll_detail(60.0),
        }
    }

    fn select_prev(&mut self) {
        match self.focus_area {
            FocusArea::List => self.select(self.selected.saturating_sub(1)),
            FocusArea::Detail => self.scroll_detail(-60.0),
        }
    }

    /// The scroll-container child index of decision `flat`, given the list is a
    /// flat sequence of `[heading, rows…, heading, rows…]`.
    fn list_child_index(&self, flat: usize) -> usize {
        let mut remaining = flat;
        let mut child = 0;
        for session in &self.data.sessions {
            child += 1; // the session heading
            if remaining < session.decisions.len() {
                return child + remaining;
            }
            remaining -= session.decisions.len();
            child += session.decisions.len();
        }
        child
    }

    /// Scroll the detail pane by `delta` pixels (positive is down).
    fn scroll_detail(&self, delta: f32) {
        let max = f32::from(self.detail_scroll.max_offset().y);
        let current = -f32::from(self.detail_scroll.offset().y);
        let next = (current + delta).clamp(0.0, max);
        self.detail_scroll.set_offset(point(px(0.0), px(-next)));
    }

    /// Record a review of the selected decision, then reload (it drops from the
    /// inbox). `actor = None` lets the core use the repo's git user.
    fn review(&mut self, status: ReviewStatus) {
        let Some(detail) = self.data.details.get(self.selected) else {
            return;
        };
        let id = detail.id.clone();
        if let Ok(concede) = Concede::discover(&self.root) {
            let _ = concede.review(&id, status, None, None);
        }
        self.data = load_app(&self.root);
        self.code_scrolls = code_scroll_pool(&self.data);
        self.selected = self.selected.min(self.data.total().saturating_sub(1));
        self.focus_area = FocusArea::List;
        self.primed = false;
    }
}

/// A stable pool of code scroll handles, one per file slot, sized to the decision
/// with the most files. Reused across selections (never recreated) so GPUI keeps
/// each slot's measured geometry — recreating a handle under the same element id
/// loses it, which breaks the horizontal scrollbar.
fn code_scroll_pool(data: &AppData) -> Vec<ScrollHandle> {
    let max_files = data
        .details
        .iter()
        .map(|d| d.files.len())
        .max()
        .unwrap_or(0);
    (0..max_files).map(|_| ScrollHandle::new()).collect()
}

impl Render for ConcedeRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Draw once more after the first layout, when scroll geometry is known.
        if !self.primed {
            self.primed = true;
            cx.notify();
        }
        // Keyboard-first: focus the root so key bindings reach it from the start.
        if !self.focused_once {
            self.focused_once = true;
            window.focus(&self.focus, cx);
        }

        let hint = if self.vim {
            "j/k move · l detail · h list · a/r review"
        } else {
            "↑/↓ move · enter detail · esc list · ⇧A/⇧R review"
        };
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
            )))
            .child(div().flex_1())
            .child(div().text_sm().text_color(rgb(FAINT)).child(hint));

        let body = if let Some(message) = &self.data.message {
            centered_message(message.clone())
        } else if self.data.total() == 0 {
            centered_message("Inbox zero: nothing to review.".to_owned())
        } else {
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(self.inbox_list(cx))
                .child(self.detail_pane(cx))
                .into_any_element()
        };

        div()
            .track_focus(&self.focus)
            .key_context("ConcedeRoot")
            .on_action(cx.listener(|this, _: &SelectNext, _window, cx| {
                this.select_next();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, _window, cx| {
                this.select_prev();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusDetail, _window, cx| {
                this.focus_area = FocusArea::Detail;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusList, _window, cx| {
                this.focus_area = FocusArea::List;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Approve, _window, cx| {
                this.review(ReviewStatus::Approved);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Reject, _window, cx| {
                this.review(ReviewStatus::Rejected);
                cx.notify();
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            // Dragging a scrollbar thumb: track the move anywhere in the window,
            // and end the drag on mouse up.
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                let Some(drag) = this.drag.clone() else {
                    return;
                };
                let pos = if drag.vertical {
                    f32::from(ev.position.y)
                } else {
                    f32::from(ev.position.x)
                };
                let delta = pos - drag.start_mouse;
                let started = -drag.start_offset;
                let scrolled = if drag.usable > 0.0 {
                    (started + delta / drag.usable * drag.max).clamp(0.0, drag.max)
                } else {
                    0.0
                };
                let offset = if drag.vertical {
                    point(px(0.0), px(-scrolled))
                } else {
                    point(px(-scrolled), px(0.0))
                };
                drag.handle.set_offset(offset);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _ev: &MouseUpEvent, _window, cx| {
                    if this.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(header)
            .child(body)
    }
}

impl ConcedeRoot {
    /// The scrollable, grouped list of unreviewed decisions (left pane).
    fn inbox_list(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let selected = self.selected;
        let mut flat = 0usize;
        // Flat, interleaved children (a heading, then its rows) so each decision row
        // is a direct child of the scroll container and `scroll_to_item` can bring
        // the selected row into view (see `list_child_index`).
        let mut children: Vec<gpui::AnyElement> = Vec::new();

        for (s, session) in self.data.sessions.iter().enumerate() {
            children.push(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .when(s > 0, |d| d.pt_2())
                    .child(session.heading.clone())
                    .into_any_element(),
            );
            for row in &session.decisions {
                let index = flat;
                flat += 1;
                children.push(
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
                            this.select(index);
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
                        )
                        .into_any_element(),
                );
            }
        }

        let list = div()
            .id("inbox-list")
            .flex()
            .flex_col()
            .size_full()
            .gap_2()
            .p_4()
            .overflow_y_scroll()
            .track_scroll(&self.list_scroll)
            .on_scroll(cx.listener(|_this, _ev, _window, cx| cx.notify()))
            .children(children);

        let focused = self.focus_area == FocusArea::List;
        div()
            .relative()
            .w(px(380.0))
            .h_full()
            .border_r_1()
            .border_color(rgb(DIVIDER))
            .child(list)
            .children(vertical_scrollbar(&self.list_scroll, cx))
            .children(focus_strip(focused))
            .into_any_element()
    }

    /// The detail of the selected decision (right pane).
    fn detail_pane(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(detail) = self.data.details.get(self.selected) else {
            return div().flex_1().into_any_element();
        };

        let mut sections = Vec::new();

        sections.push(
            div()
                .flex()
                .flex_col()
                .min_w_0()
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

        for (i, file) in detail.files.iter().enumerate() {
            sections.push(file_section(i, file, self.code_scrolls.get(i), cx));
        }

        let content = div()
            .id("detail")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .gap_5()
            .p_5()
            .overflow_y_scroll()
            .track_scroll(&self.detail_scroll)
            .on_scroll(cx.listener(|_this, _ev, _window, cx| cx.notify()))
            .children(sections);

        let focused = self.focus_area == FocusArea::Detail;
        div()
            .relative()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(content)
            .children(vertical_scrollbar(&self.detail_scroll, cx))
            .children(focus_strip(focused))
            .into_any_element()
    }
}

/// A titled section in the detail pane.
fn section(title: &str, body: impl IntoElement) -> gpui::AnyElement {
    div()
        .flex()
        .flex_col()
        .min_w_0()
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
/// changed lines highlighted, horizontally scrollable (or a note when the file
/// can't be read).
fn file_section(
    i: usize,
    file: &FileDetail,
    scroll: Option<&ScrollHandle>,
    cx: &mut Context<ConcedeRoot>,
) -> gpui::AnyElement {
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
        div()
            .text_sm()
            .text_color(rgb(MUTED))
            .child(note.clone())
            .into_any_element()
    } else {
        let rows = file.code.iter().map(|line| {
            div()
                .flex()
                .gap_3()
                .when(line.highlighted, |row| row.bg(rgb(CODE_HIGHLIGHT)))
                .child(
                    div()
                        .flex_none()
                        .w(px(44.0))
                        .text_color(rgb(FAINT))
                        .child(format!("{:>4}", line.number)),
                )
                .child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_color(rgb(TEXT))
                        .child(line.text.clone()),
                )
        });

        // Content sized to the widest line: flex_none stops it shrinking to the
        // scroller's width, and items_start stops rows stretching, so the widest
        // line sets the width and the block can overflow and scroll horizontally.
        let code_inner = div()
            .flex()
            .flex_col()
            .flex_none()
            .items_start()
            .p_3()
            .children(rows);

        // A flex row so its flex_none child keeps its content width instead of
        // being sized to the (bounded) scroller width.
        let scroller = div()
            .id(("code", i))
            .flex()
            .w_full()
            .min_w_0()
            .overflow_x_scroll()
            .child(code_inner);
        let scroller = match scroll {
            Some(handle) => scroller
                .track_scroll(handle)
                .on_scroll(cx.listener(|_this, _ev, _window, cx| cx.notify())),
            None => scroller,
        };

        // The horizontal scrollbar is a sibling of the scroll area (not inside
        // it), so it stays put instead of scrolling with the code.
        let mut wrapper = div()
            .relative()
            .min_w_0()
            .rounded_md()
            .bg(rgb(CODE_BG))
            .text_sm()
            .child(scroller);
        if let Some(handle) = scroll {
            wrapper = wrapper.children(horizontal_scrollbar(handle, cx));
        }
        wrapper.into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap_2()
        .child(heading)
        .child(body)
        .into_any_element()
}

/// A draggable vertical scrollbar overlay for a scroll handle, or `None` when the
/// content fits. Position it inside a `relative` wrapper beside the scroll area.
fn vertical_scrollbar(
    handle: &ScrollHandle,
    cx: &mut Context<ConcedeRoot>,
) -> Option<gpui::AnyElement> {
    let max = f32::from(handle.max_offset().y);
    let viewport = f32::from(handle.bounds().size.height);
    if max <= 1.0 || viewport <= 0.0 {
        return None;
    }
    let thumb = (viewport * viewport / (viewport + max))
        .max(MIN_THUMB)
        .min(viewport);
    let usable = (viewport - thumb).max(0.0);
    let scrolled = (-f32::from(handle.offset().y)).clamp(0.0, max);
    let top = if max > 0.0 {
        scrolled / max * usable
    } else {
        0.0
    };

    let handle = handle.clone();
    let thumb_el = div()
        .absolute()
        .top(px(top))
        .right_0()
        .w(px(8.0))
        .h(px(thumb))
        .rounded_full()
        .bg(rgb(SCROLL_THUMB))
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
                this.drag = Some(Drag {
                    handle: handle.clone(),
                    vertical: true,
                    start_mouse: f32::from(ev.position.y),
                    start_offset: f32::from(handle.offset().y),
                    usable,
                    max,
                });
                cx.notify();
            }),
        );

    Some(
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(10.0))
            .child(thumb_el)
            .into_any_element(),
    )
}

/// A draggable horizontal scrollbar overlay for a scroll handle, or `None` when
/// the content fits.
fn horizontal_scrollbar(
    handle: &ScrollHandle,
    cx: &mut Context<ConcedeRoot>,
) -> Option<gpui::AnyElement> {
    let max = f32::from(handle.max_offset().x);
    let viewport = f32::from(handle.bounds().size.width);
    if max <= 1.0 || viewport <= 0.0 {
        return None;
    }
    let thumb = (viewport * viewport / (viewport + max))
        .max(MIN_THUMB)
        .min(viewport);
    let usable = (viewport - thumb).max(0.0);
    let scrolled = (-f32::from(handle.offset().x)).clamp(0.0, max);
    let left = if max > 0.0 {
        scrolled / max * usable
    } else {
        0.0
    };

    let handle = handle.clone();
    let thumb_el = div()
        .absolute()
        .left(px(left))
        .bottom_0()
        .h(px(8.0))
        .w(px(thumb))
        .rounded_full()
        .bg(rgb(SCROLL_THUMB))
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
                this.drag = Some(Drag {
                    handle: handle.clone(),
                    vertical: false,
                    start_mouse: f32::from(ev.position.x),
                    start_offset: f32::from(handle.offset().x),
                    usable,
                    max,
                });
                cx.notify();
            }),
        );

    Some(
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(10.0))
            .child(thumb_el)
            .into_any_element(),
    )
}

/// A 2px accent strip across the top of the keyboard-focused pane.
fn focus_strip(focused: bool) -> Option<gpui::AnyElement> {
    focused.then(|| {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(2.0))
            .bg(rgb(ACCENT))
            .into_any_element()
    })
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
    let key_config = load_key_config();

    gpui_platform::application().run(move |cx: &mut App| {
        cx.bind_keys(key_bindings(&key_config));

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
        let vim = key_config.vim;
        cx.open_window(options, |_, cx| {
            cx.new(|cx| ConcedeRoot::new(root, data, vim, cx))
        })
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

//! Concede desktop app. Thin GPUI layer over `concede-core`.
//!
//! Phase 2:
//! - step 2: the story inbox — unreviewed decisions grouped by agent session.
//! - step 3: a detail pane — selecting a decision shows its reasoning, the
//!   alternatives it rejected, and the touched code read-only with the changed
//!   lines highlighted (ranges mapped onto the current file via `why`).
//!   Draggable scrollbars: vertical on the list and detail panes, horizontal on
//!   each code block (lines don't wrap).
//! - step 4: keyboard-first navigation, review, and a configurable keymap.
//! - step 5: code mode — one key opens the selected decision's file in a basic
//!   editable view; edits save to disk explicitly, Esc discards and returns to the
//!   same visual-mode spot. See ADR 0009.
//!
//! Phase 3:
//! - step 3: the inbox is grouped into change stories (by recorded intent, falling
//!   back to session), each with an aggregate-status chip, and the whole story can
//!   be approved or rejected in one key. See ADR 0011.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use concede_core::{
    ChangeStore, Concede, Decision, MappedHunk, Mapping, PRODUCT_NAME, ReviewStatus, Story,
    StoryKey, StoryStatus, WhyRecord,
};
use gpui::{
    App, Bounds, Context, FocusHandle, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ScrollHandle, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowOptions, actions, div, point, prelude::*, px, rgb, size,
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
        Reject,
        ApproveStory,
        RejectStory,
        EnterCode,
        ExitCode,
        Save,
        Paste
    ]
);

/// Which area the keyboard drives: moving the selection, scrolling the detail, or
/// editing the file in code mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusArea {
    List,
    Detail,
    Code,
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
/// Actions: select_next, select_prev, focus_detail, focus_list, approve, reject,
/// approve_story, reject_story (review the whole story), code (enter code mode),
/// save (write the file in code mode).
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
            KeyBinding::new("shift-a", ApproveStory, CONTEXT),
            KeyBinding::new("shift-r", RejectStory, CONTEXT),
            KeyBinding::new("c", EnterCode, CONTEXT),
        ]
    } else {
        vec![
            KeyBinding::new("down", SelectNext, CONTEXT),
            KeyBinding::new("up", SelectPrev, CONTEXT),
            KeyBinding::new("enter", FocusDetail, CONTEXT),
            KeyBinding::new("escape", FocusList, CONTEXT),
            KeyBinding::new("shift-a", Approve, CONTEXT),
            KeyBinding::new("shift-r", Reject, CONTEXT),
            KeyBinding::new("ctrl-a", ApproveStory, CONTEXT),
            KeyBinding::new("ctrl-r", RejectStory, CONTEXT),
            KeyBinding::new("c", EnterCode, CONTEXT),
        ]
    };

    // Code mode runs under its own key context so the single-letter visual-mode
    // binds above (a, j/k, …) don't fire while typing; see ADR 0009. Only these
    // few keys are bound there — every other key reaches the editor as text input.
    const CODE: Option<&str> = Some("ConcedeCode");
    binds.extend([
        KeyBinding::new("escape", ExitCode, CODE),
        KeyBinding::new("cmd-s", Save, CODE),
        KeyBinding::new("ctrl-s", Save, CODE),
        KeyBinding::new("cmd-v", Paste, CODE),
        KeyBinding::new("ctrl-v", Paste, CODE),
    ]);

    for (action, key) in &config.overrides {
        let key: &'static str = Box::leak(key.clone().into_boxed_str());
        let bind = match action.as_str() {
            "select_next" => KeyBinding::new(key, SelectNext, CONTEXT),
            "select_prev" => KeyBinding::new(key, SelectPrev, CONTEXT),
            "focus_detail" => KeyBinding::new(key, FocusDetail, CONTEXT),
            "focus_list" => KeyBinding::new(key, FocusList, CONTEXT),
            "approve" => KeyBinding::new(key, Approve, CONTEXT),
            "reject" => KeyBinding::new(key, Reject, CONTEXT),
            "approve_story" => KeyBinding::new(key, ApproveStory, CONTEXT),
            "reject_story" => KeyBinding::new(key, RejectStory, CONTEXT),
            "code" => KeyBinding::new(key, EnterCode, CONTEXT),
            "save" => KeyBinding::new(key, Save, CODE),
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

/// The snappiness budget (CLAUDE.md): a frame's UI-thread build over this is a bug.
const DEFAULT_FRAME_BUDGET: Duration = Duration::from_millis(16);
/// Over-budget frames log at most this often (with a count of those suppressed).
const FRAME_LOG_INTERVAL: Duration = Duration::from_secs(1);

/// Frame-time instrumentation (ADR 0010). Times our `render` build each frame and
/// logs to stderr when it exceeds the budget. Breaches are throttled to roughly
/// once a second with a count of the ones suppressed between, so a sustained slow
/// state doesn't flood the console and the fast path stays a subtract-and-compare.
///
/// Env: `CONCEDE_FRAME_BUDGET_MS` tunes the budget; `CONCEDE_FRAME_TRACE=1` logs
/// every frame's time (for active profiling).
#[derive(Debug)]
struct FrameStats {
    budget: Duration,
    trace: bool,
    last_log: Option<Instant>,
    suppressed: u32,
}

impl FrameStats {
    fn new(budget: Duration, trace: bool) -> Self {
        Self {
            budget,
            trace,
            last_log: None,
            suppressed: 0,
        }
    }

    fn from_env() -> Self {
        let budget = std::env::var("CONCEDE_FRAME_BUDGET_MS")
            .ok()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|ms| *ms > 0.0)
            .map(|ms| Duration::from_secs_f64(ms / 1000.0))
            .unwrap_or(DEFAULT_FRAME_BUDGET);
        let trace = matches!(
            std::env::var("CONCEDE_FRAME_TRACE").ok().as_deref(),
            Some("1") | Some("true") | Some("TRUE")
        );
        Self::new(budget, trace)
    }

    /// Decide what, if anything, to log for a frame that took `elapsed`, as of
    /// `now`. Pure (no stderr, no clock), so the policy is unit-testable.
    fn record_at(&mut self, elapsed: Duration, now: Instant) -> Option<String> {
        if self.trace {
            return Some(format!("concede: frame {:.1}ms", millis(elapsed)));
        }
        if elapsed <= self.budget {
            return None;
        }
        let due = self
            .last_log
            .is_none_or(|last| now.saturating_duration_since(last) >= FRAME_LOG_INTERVAL);
        if !due {
            self.suppressed += 1;
            return None;
        }
        let message = if self.suppressed > 0 {
            format!(
                "concede: slow frame {:.1}ms (budget {:.0}ms, +{} more since)",
                millis(elapsed),
                millis(self.budget),
                self.suppressed
            )
        } else {
            format!(
                "concede: slow frame {:.1}ms (budget {:.0}ms)",
                millis(elapsed),
                millis(self.budget)
            )
        };
        self.last_log = Some(now);
        self.suppressed = 0;
        Some(message)
    }

    /// Record a frame's build time and print any resulting log line.
    fn record(&mut self, elapsed: Duration) {
        if let Some(line) = self.record_at(elapsed, Instant::now()) {
            eprintln!("{line}");
        }
    }
}

fn millis(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// One unreviewed decision in the list, flattened to owned strings.
#[derive(Debug, Clone)]
struct DecisionRow {
    short_id: String,
    when: String,
    task: String,
    files: String,
}

/// One change story's unreviewed decisions (ADR 0011).
#[derive(Debug, Clone)]
struct StoryRow {
    heading: String,
    /// Aggregate review state, e.g. `unreviewed` or `2 of 5 reviewed`.
    status: String,
    decisions: Vec<DecisionRow>,
    /// Full ids of the unreviewed decisions here, for whole-story review.
    ids: Vec<String>,
}

/// The heading for a story's group (its intent, or the session/ungrouped fallback).
fn story_heading(key: StoryKey) -> String {
    match key {
        StoryKey::Intent(intent) => format!("Intent: {intent}"),
        StoryKey::Session(session) => format!("Session {session}"),
        StoryKey::Ungrouped => "Ungrouped".to_owned(),
    }
}

/// A short aggregate-status label over a story's decisions (reviewed ones included).
fn story_status_label(story: &Story) -> String {
    match story.status() {
        StoryStatus::Unreviewed => "unreviewed".to_owned(),
        StoryStatus::Reviewed => "reviewed".to_owned(),
        StoryStatus::Mixed => {
            let done = story.review.approved + story.review.rejected;
            format!("{done} of {} reviewed", story.review.total())
        }
    }
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
    stories: Vec<StoryRow>,
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

/// A minimal editable text buffer for code mode (ADR 0009): the file split into
/// `lines`, plus a caret at (`row`, `col`). `col` is a **character** index within
/// the row (not a byte offset), so edits stay correct on multi-byte UTF-8 lines.
/// All editing logic lives here, pure and unit-tested; the view only renders it
/// and feeds it key events.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditBuffer {
    lines: Vec<String>,
    row: usize,
    col: usize,
}

impl EditBuffer {
    /// Split text into lines. `"a\nb"` → `["a", "b"]`; `"a\n"` → `["a", ""]`;
    /// `""` → `[""]`. Always holds at least one line, so `lines[row]` is safe.
    fn from_str(text: &str) -> Self {
        let lines = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(str::to_owned).collect()
        };
        Self {
            lines,
            row: 0,
            col: 0,
        }
    }

    /// Reassemble the buffer into file text (the inverse of [`from_str`]).
    fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Characters in the given row.
    fn row_len(&self, row: usize) -> usize {
        self.lines[row].chars().count()
    }

    /// The byte offset of character index `col` within `line` (its end if `col`
    /// is past the last character), for splicing into the `String`.
    fn byte_of(line: &str, col: usize) -> usize {
        line.char_indices()
            .nth(col)
            .map(|(b, _)| b)
            .unwrap_or(line.len())
    }

    /// Keep `col` within the current row (after a vertical move onto a shorter row).
    fn clamp_col(&mut self) {
        self.col = self.col.min(self.row_len(self.row));
    }

    /// Place the caret, clamped to the buffer.
    fn place_at(&mut self, row: usize, col: usize) {
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.row_len(self.row));
    }

    fn insert_char(&mut self, ch: char) {
        let at = Self::byte_of(&self.lines[self.row], self.col);
        self.lines[self.row].insert(at, ch);
        self.col += 1;
    }

    /// Insert arbitrary text (e.g. a clipboard paste), honouring embedded newlines
    /// and dropping `\r` so pasted CRLF text doesn't leave stray carriage returns.
    fn insert_str(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => self.insert_newline(),
                '\r' => {}
                _ => self.insert_char(ch),
            }
        }
    }

    fn insert_newline(&mut self) {
        let at = Self::byte_of(&self.lines[self.row], self.col);
        let rest = self.lines[self.row].split_off(at);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.col = 0;
    }

    /// Delete the character before the caret, or merge with the previous line.
    fn backspace(&mut self) {
        if self.col > 0 {
            let prev = self.col - 1;
            let at = Self::byte_of(&self.lines[self.row], prev);
            self.lines[self.row].remove(at);
            self.col = prev;
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.row_len(self.row);
            self.lines[self.row].push_str(&line);
        }
    }

    /// Delete the character at the caret, or merge the next line onto this one.
    fn delete(&mut self) {
        if self.col < self.row_len(self.row) {
            let at = Self::byte_of(&self.lines[self.row], self.col);
            self.lines[self.row].remove(at);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.row_len(self.row);
        }
    }

    fn move_right(&mut self) {
        if self.col < self.row_len(self.row) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.clamp_col();
        }
    }

    fn move_down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.clamp_col();
        }
    }

    fn move_home(&mut self) {
        self.col = 0;
    }

    fn move_end(&mut self) {
        self.col = self.row_len(self.row);
    }
}

/// Code mode: one file open in the editable buffer. The buffer is the whole file
/// (ADR 0009); `highlights` are the decision's changed line numbers, still marked.
struct CodeMode {
    /// Absolute path, written on save.
    path: PathBuf,
    /// Repo-relative path, for the header.
    rel: String,
    buffer: EditBuffer,
    highlights: BTreeSet<u32>,
    /// Unsaved edits since the last save (or since opening).
    dirty: bool,
    /// Last save result, shown in the header (e.g. `saved 14:03:07`).
    status: Option<String>,
    /// The visual area to return to on Esc.
    return_area: FocusArea,
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

    // Group all decisions into change stories (ADR 0011), but keep the review-queue
    // behaviour: show only stories with unreviewed work, listing their unreviewed
    // decisions, so reviewing empties the inbox as before.
    let log = &loaded.log;
    let mut stories = Vec::new();
    let mut details = Vec::new();

    for story in &log.stories() {
        let unreviewed: Vec<&Decision> = story
            .decisions
            .iter()
            .copied()
            .filter(|d| log.state(&d.id).review == ReviewStatus::Unreviewed)
            .collect();
        if unreviewed.is_empty() {
            continue;
        }

        let mut rows = Vec::with_capacity(unreviewed.len());
        let mut ids = Vec::with_capacity(unreviewed.len());
        for d in &unreviewed {
            rows.push(DecisionRow {
                short_id: short_id(&d.id.to_string()),
                when: local(&d.timestamp),
                task: d.task.clone(),
                files: files_label(d),
            });
            ids.push(d.id.to_string());
            details.push(build_detail(&concede, &repo_root, d));
        }

        stories.push(StoryRow {
            heading: story_heading(story.key),
            status: story_status_label(story),
            decisions: rows,
            ids,
        });
    }

    AppData {
        stories,
        details,
        unexplained: log.inbox().unexplained.len(),
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
    /// Which area the keyboard drives.
    focus_area: FocusArea,
    /// Present while editing a file in code mode (ADR 0009).
    code: Option<CodeMode>,
    /// Whether the vim keymap is active (for the header hint).
    vim: bool,
    focus: FocusHandle,
    /// Focused while in code mode, so the editor receives key events.
    code_focus: FocusHandle,
    focused_once: bool,
    list_scroll: ScrollHandle,
    detail_scroll: ScrollHandle,
    /// The code-mode editor's vertical scroll (reused across entries, per the
    /// no-recreate-under-same-id gotcha).
    edit_scroll: ScrollHandle,
    /// One per file of the selected decision (for horizontal code scrolling).
    code_scrolls: Vec<ScrollHandle>,
    /// The scrollbar drag in progress, if any.
    drag: Option<Drag>,
    /// Scroll geometry is only known after the first layout, so we request one
    /// extra frame on startup to draw the scrollbars at their true sizes.
    primed: bool,
    /// Frame-time instrumentation: warns when a render build exceeds the budget.
    frame_stats: FrameStats,
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
            code: None,
            vim,
            focus: cx.focus_handle(),
            code_focus: cx.focus_handle(),
            focused_once: false,
            list_scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            edit_scroll: ScrollHandle::new(),
            drag: None,
            primed: false,
            frame_stats: FrameStats::from_env(),
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
            // Code mode runs under its own key context, so these never reach it.
            FocusArea::Code => {}
        }
    }

    fn select_prev(&mut self) {
        match self.focus_area {
            FocusArea::List => self.select(self.selected.saturating_sub(1)),
            FocusArea::Detail => self.scroll_detail(-60.0),
            FocusArea::Code => {}
        }
    }

    /// The scroll-container child index of decision `flat`, given the list is a
    /// flat sequence of `[heading, rows…, heading, rows…]`.
    fn list_child_index(&self, flat: usize) -> usize {
        let mut remaining = flat;
        let mut child = 0;
        for story in &self.data.stories {
            child += 1; // the story heading
            if remaining < story.decisions.len() {
                return child + remaining;
            }
            remaining -= story.decisions.len();
            child += story.decisions.len();
        }
        child
    }

    /// The story the flat decision index `flat` belongs to.
    fn story_of(&self, flat: usize) -> Option<&StoryRow> {
        let mut remaining = flat;
        for story in &self.data.stories {
            if remaining < story.decisions.len() {
                return Some(story);
            }
            remaining -= story.decisions.len();
        }
        None
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
        // Clone the id out first: `review_ids` takes `&mut self`, so it can't hold a
        // borrow into `self.data`.
        let id = detail.id.clone();
        self.review_ids(std::slice::from_ref(&id), status);
    }

    /// Review every unreviewed decision in the selected decision's story at once
    /// ("accept or reject per group", ADR 0011), then reload.
    fn review_story(&mut self, status: ReviewStatus) {
        let Some(ids) = self.story_of(self.selected).map(|s| s.ids.clone()) else {
            return;
        };
        self.review_ids(&ids, status);
    }

    /// Apply `status` to each id, then reload the inbox and clamp the selection.
    fn review_ids(&mut self, ids: &[String], status: ReviewStatus) {
        if let Ok(concede) = Concede::discover(&self.root) {
            for id in ids {
                let _ = concede.review(id, status, None, None);
            }
        }
        self.data = load_app(&self.root);
        self.code_scrolls = code_scroll_pool(&self.data);
        self.selected = self.selected.min(self.data.total().saturating_sub(1));
        self.focus_area = FocusArea::List;
        self.primed = false;
    }

    /// Enter code mode on the selected decision's first on-disk file (ADR 0009),
    /// caret on its first changed line. Returns whether a file was opened (nothing
    /// to edit — no readable file — is a no-op).
    fn enter_code(&mut self) -> bool {
        let Some(detail) = self.data.details.get(self.selected) else {
            return false;
        };
        let Some(file) = detail.files.iter().find(|f| f.note.is_none()) else {
            return false;
        };
        let path = self.root.join(&file.path);
        // Read fresh from disk rather than reconstructing from the display rows
        // (which pad blank lines); this is the exact text we'll edit and save.
        let Ok(text) = std::fs::read_to_string(&path) else {
            return false;
        };
        let highlights: BTreeSet<u32> = file
            .code
            .iter()
            .filter(|line| line.highlighted)
            .map(|line| line.number)
            .collect();

        let mut buffer = EditBuffer::from_str(&text);
        if let Some(&first) = highlights.iter().next() {
            buffer.place_at((first as usize).saturating_sub(1), 0);
        }

        self.edit_scroll.set_offset(point(px(0.0), px(0.0)));
        self.code = Some(CodeMode {
            path,
            rel: file.path.clone(),
            buffer,
            highlights,
            dirty: false,
            status: None,
            return_area: self.focus_area,
        });
        self.focus_area = FocusArea::Code;
        self.primed = false;
        true
    }

    /// Leave code mode, discarding any unsaved edits, back to the exact spot we
    /// entered from (selection and scroll are untouched by code mode).
    fn exit_code(&mut self) {
        if let Some(code) = self.code.take() {
            self.focus_area = code.return_area;
        }
        self.primed = false;
    }

    /// Write the buffer to disk. A human edit isn't logged as a decision yet
    /// (future work, ADR 0009); this just updates the working tree.
    fn save_code(&mut self) {
        if let Some(code) = &mut self.code {
            match std::fs::write(&code.path, code.buffer.text()) {
                Ok(()) => {
                    code.dirty = false;
                    code.status = Some(format!("saved {}", Local::now().format("%H:%M:%S")));
                }
                Err(error) => code.status = Some(format!("save failed: {error}")),
            }
        }
    }

    /// Insert the clipboard text at the caret.
    fn paste_code(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if let Some(code) = &mut self.code {
            code.buffer.insert_str(&text);
            code.dirty = true;
        }
    }

    /// Apply one key press to the editable buffer. Returns whether anything
    /// changed (so the caller only requests a frame when needed). Keys bound as
    /// actions in the `ConcedeCode` context (Esc, save, paste) never arrive here.
    fn edit_key(&mut self, ev: &KeyDownEvent) -> bool {
        let Some(code) = self.code.as_mut() else {
            return false;
        };
        let modifiers = ev.keystroke.modifiers;
        // Caret moves redraw (to follow the caret) but don't dirty the buffer.
        let mut edited = true;
        match ev.keystroke.key.as_str() {
            "backspace" => code.buffer.backspace(),
            "delete" => code.buffer.delete(),
            "enter" => code.buffer.insert_newline(),
            "tab" => code.buffer.insert_str("    "),
            "left" => {
                code.buffer.move_left();
                edited = false;
            }
            "right" => {
                code.buffer.move_right();
                edited = false;
            }
            "up" => {
                code.buffer.move_up();
                edited = false;
            }
            "down" => {
                code.buffer.move_down();
                edited = false;
            }
            "home" => {
                code.buffer.move_home();
                edited = false;
            }
            "end" => {
                code.buffer.move_end();
                edited = false;
            }
            _ => {
                // A typed character: use the char the key would produce, but skip
                // when a command/control/function modifier is down (those are
                // shortcuts, not text).
                if modifiers.control || modifiers.platform || modifiers.function {
                    return false;
                }
                match ev.keystroke.key_char.as_deref() {
                    Some(text) if !text.is_empty() => code.buffer.insert_str(text),
                    _ => return false,
                }
            }
        }
        if edited {
            code.dirty = true;
        }
        true
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
        // Time the element-tree build; `frame_stats.record` (below) warns if it
        // exceeds the 16 ms budget (ADR 0010).
        let build_start = Instant::now();
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

        let hint = if self.code.is_some() {
            "editing · type to change · ⌘S save · esc discard & back"
        } else if self.vim {
            "j/k move · l detail · h list · a/r review · ⇧A/⇧R story · c edit"
        } else {
            "↑/↓ move · enter detail · esc list · ⇧A/⇧R review · ⌃A/⌃R story · c edit"
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

        let body = if self.code.is_some() {
            self.code_pane(cx)
        } else if let Some(message) = &self.data.message {
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

        // In code mode the root advertises a different key context so the
        // single-letter visual-mode binds don't fire while typing (ADR 0009).
        let key_context = if self.code.is_some() {
            "ConcedeCode"
        } else {
            "ConcedeRoot"
        };
        let root = div()
            .track_focus(&self.focus)
            .key_context(key_context)
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
            .on_action(cx.listener(|this, _: &ApproveStory, _window, cx| {
                this.review_story(ReviewStatus::Approved);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RejectStory, _window, cx| {
                this.review_story(ReviewStatus::Rejected);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &EnterCode, window, cx| {
                if this.enter_code() {
                    window.focus(&this.code_focus, cx);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ExitCode, window, cx| {
                this.exit_code();
                window.focus(&this.focus, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Save, _window, cx| {
                this.save_code();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Paste, _window, cx| {
                this.paste_code(cx);
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
            .child(body);

        self.frame_stats.record(build_start.elapsed());
        root
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

        for (s, story) in self.data.stories.iter().enumerate() {
            children.push(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .when(s > 0, |d| d.pt_2())
                    .child(div().child(story.heading.clone()))
                    .child(
                        div()
                            .text_color(rgb(FAINT))
                            .child(format!("· {}", story.status)),
                    )
                    .into_any_element(),
            );
            for row in &story.decisions {
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

    /// The code-mode editor: a header (file, dirty/save state) and the whole file
    /// as editable, vertically scrollable lines with a caret. Replaces the split
    /// while editing; Esc returns to it.
    fn code_pane(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let code = self.code.as_ref().expect("code_pane with code mode active");

        let (state_text, state_color) = if code.dirty {
            ("edited".to_owned(), ACCENT)
        } else {
            (code.status.clone().unwrap_or_default(), MUTED)
        };
        let subhead = div()
            .flex()
            .items_baseline()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .child(div().text_color(rgb(ACCENT)).child(code.rel.clone()))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(state_color))
                    .child(state_text),
            );

        let caret_row = code.buffer.row;
        let rows = code.buffer.lines.iter().enumerate().map(|(i, line)| {
            let number = i as u32 + 1;
            let gutter = div()
                .flex_none()
                .w(px(44.0))
                .text_color(rgb(FAINT))
                .child(format!("{:>4}", number));

            let text_el = if i == caret_row {
                // Split the line at the caret (by character index) and drop a thin
                // bar between the two halves, so it sits at the real glyph boundary
                // regardless of font metrics.
                let chars: Vec<char> = line.chars().collect();
                let col = code.buffer.col.min(chars.len());
                let before: String = chars[..col].iter().collect();
                let after: String = chars[col..].iter().collect();
                div()
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .text_color(rgb(TEXT))
                    .child(before)
                    .child(div().w(px(2.0)).h(px(16.0)).bg(rgb(ACCENT)))
                    .child(after)
            } else {
                div()
                    .whitespace_nowrap()
                    .text_color(rgb(TEXT))
                    // Keep blank lines at a row's height.
                    .child(if line.is_empty() {
                        " ".to_owned()
                    } else {
                        line.clone()
                    })
            };

            div()
                .id(("edit-line", i))
                .flex()
                .gap_3()
                .when(code.highlights.contains(&number), |row| {
                    row.bg(rgb(CODE_HIGHLIGHT))
                })
                .child(gutter)
                .child(text_el)
        });

        let editor = div()
            .id("code-editor")
            .track_focus(&self.code_focus)
            .flex()
            .flex_col()
            .size_full()
            .p_3()
            .text_sm()
            .overflow_y_scroll()
            .track_scroll(&self.edit_scroll)
            .on_scroll(cx.listener(|_this, _ev, _window, cx| cx.notify()))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                if this.edit_key(ev) {
                    // Keep the caret's line on screen (its row is a direct child).
                    if let Some(code) = &this.code {
                        this.edit_scroll.scroll_to_item(code.buffer.row);
                    }
                    cx.notify();
                }
            }))
            .children(rows);

        let wrapper = div()
            .relative()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(editor)
            .children(vertical_scrollbar(&self.edit_scroll, cx));

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(subhead)
            .child(wrapper)
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

    #[test]
    fn edit_buffer_round_trips_text() {
        for text in [
            "",
            "one line",
            "a\nb\nc",
            "trailing\n",
            "\nleading",
            "a\n\nb",
        ] {
            assert_eq!(EditBuffer::from_str(text).text(), text);
        }
        // Always at least one line, so indexing lines[row] is safe.
        assert_eq!(EditBuffer::from_str("").lines, vec![String::new()]);
    }

    #[test]
    fn edit_buffer_inserts_and_splits_lines() {
        let mut b = EditBuffer::from_str("ac");
        b.place_at(0, 1);
        b.insert_char('b');
        assert_eq!(b.text(), "abc");
        assert_eq!((b.row, b.col), (0, 2));

        b.insert_newline();
        assert_eq!(b.text(), "ab\nc");
        assert_eq!((b.row, b.col), (1, 0));
    }

    #[test]
    fn edit_buffer_backspace_merges_and_deletes() {
        // Backspace at column 0 merges with the previous line.
        let mut b = EditBuffer::from_str("ab\ncd");
        b.place_at(1, 0);
        b.backspace();
        assert_eq!(b.text(), "abcd");
        assert_eq!((b.row, b.col), (0, 2));

        // Backspace mid-line deletes the char before the caret.
        b.backspace();
        assert_eq!(b.text(), "acd");
        assert_eq!((b.row, b.col), (0, 1));

        // Delete at end-of-line merges the next line onto this one.
        let mut b = EditBuffer::from_str("ab\ncd");
        b.place_at(0, 2);
        b.delete();
        assert_eq!(b.text(), "abcd");
        assert_eq!((b.row, b.col), (0, 2));
    }

    #[test]
    fn edit_buffer_paste_honours_newlines_and_drops_cr() {
        let mut b = EditBuffer::from_str("");
        b.insert_str("x\r\ny");
        assert_eq!(b.text(), "x\ny");
        assert_eq!((b.row, b.col), (1, 1));
    }

    #[test]
    fn edit_buffer_moves_clamp_to_shorter_rows() {
        let mut b = EditBuffer::from_str("long line\nhi");
        b.place_at(0, 9); // end of the long line
        b.move_down(); // onto the short line: column clamps to its length
        assert_eq!((b.row, b.col), (1, 2));
        b.move_right(); // at end of last line: no wrap past the buffer
        assert_eq!((b.row, b.col), (1, 2));
        b.move_home();
        assert_eq!((b.row, b.col), (1, 0));
        b.move_left(); // column 0: wraps to the end of the previous line
        assert_eq!((b.row, b.col), (0, 9));
    }

    #[test]
    fn frame_stats_logs_breaches_and_throttles() {
        let budget = Duration::from_millis(16);
        let mut stats = FrameStats::new(budget, false);
        let t0 = Instant::now();

        // Under budget: nothing logged.
        assert_eq!(stats.record_at(Duration::from_millis(8), t0), None);

        // First breach logs.
        let first = stats
            .record_at(Duration::from_millis(20), t0)
            .expect("first breach logs");
        assert!(first.contains("slow frame"), "{first}");
        assert!(!first.contains("more since"), "{first}");

        // A second breach within the interval is suppressed...
        assert_eq!(
            stats.record_at(Duration::from_millis(25), t0 + Duration::from_millis(100)),
            None
        );
        assert_eq!(
            stats.record_at(Duration::from_millis(25), t0 + Duration::from_millis(200)),
            None
        );

        // ...and the next breach after the interval reports the suppressed count.
        let later = stats
            .record_at(Duration::from_millis(30), t0 + FRAME_LOG_INTERVAL)
            .expect("breach after the interval logs");
        assert!(later.contains("+2 more since"), "{later}");
    }

    #[test]
    fn frame_stats_trace_logs_every_frame() {
        let mut stats = FrameStats::new(Duration::from_millis(16), true);
        let t0 = Instant::now();
        // Even a fast frame is reported in trace mode.
        let line = stats
            .record_at(Duration::from_millis(1), t0)
            .expect("trace logs every frame");
        assert!(line.contains("frame"), "{line}");
        assert!(!line.contains("slow"), "{line}");
    }

    #[test]
    fn edit_buffer_edits_multibyte_lines_by_char() {
        // "café" — the accented char is multi-byte; caret is a char index, so a
        // backspace after it must not split a UTF-8 boundary.
        let mut b = EditBuffer::from_str("café");
        b.place_at(0, 4);
        b.backspace();
        assert_eq!(b.text(), "caf");
        b.insert_str("é!");
        assert_eq!(b.text(), "café!");
    }
}

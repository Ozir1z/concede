//! Mapping recorded line ranges onto the current file (line drift, ADR 0006).
//!
//! Given the hunks of a diff from the recorded content (old) to the current content
//! (new), [`map_range`] says where a recorded range is now and whether its lines were
//! edited since. Pure functions, so they're tested without git.

use crate::model::{Hunk, LineRange};

/// How a recorded range relates to the current file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mapping {
    /// The lines are unchanged; they may have moved.
    Unchanged,
    /// Some of the lines were edited since the record was made.
    Modified,
    /// Every line of the range was deleted since.
    Gone,
    /// The recorded content is no longer available, so the range is shown as recorded.
    Approximate,
}

/// Where a recorded range is in the current file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MappedRange {
    /// The range in the current file; `None` when the lines are gone.
    pub range: Option<LineRange>,
    pub mapping: Mapping,
}

impl MappedRange {
    /// A range shown as recorded, because the recorded content can't be diffed.
    pub fn approximate(range: LineRange) -> Self {
        Self {
            range: Some(range),
            mapping: Mapping::Approximate,
        }
    }
}

#[derive(Clone, Copy)]
enum Edge {
    Start,
    End,
}

/// Map `range` (old-file lines) through `hunks` (old -> new, sorted, non-overlapping).
pub fn map_range(range: LineRange, hunks: &[Hunk]) -> MappedRange {
    let modified = hunks.iter().any(|h| {
        if h.old_lines == 0 {
            // Lines inserted *inside* the range (after one of its lines but not the last).
            range.start <= h.old_start && h.old_start < range.end
        } else {
            let old_end = h.old_start + h.old_lines - 1;
            h.old_start <= range.end && range.start <= old_end
        }
    });
    let start = map_line(range.start, hunks, Edge::Start);
    let end = map_line(range.end, hunks, Edge::End);
    match LineRange::new(start, end) {
        Ok(range) => MappedRange {
            range: Some(range),
            mapping: if modified {
                Mapping::Modified
            } else {
                Mapping::Unchanged
            },
        },
        Err(_) => MappedRange {
            range: None,
            mapping: Mapping::Gone,
        },
    }
}

/// Map one old line to a new line. A line inside a replaced region maps to the edge of
/// the replacement; a line inside a deleted region maps just past the deletion, which
/// can make start > end: that's how [`map_range`] detects a range that is gone.
fn map_line(line: u32, hunks: &[Hunk], edge: Edge) -> u32 {
    let mut delta: i64 = 0;
    for h in hunks {
        if h.old_lines == 0 {
            // Pure insertion after old line `old_start`.
            if h.old_start < line {
                delta += i64::from(h.new_lines);
                continue;
            }
            break;
        }
        let old_end = h.old_start + h.old_lines - 1;
        if old_end < line {
            delta += i64::from(h.new_lines) - i64::from(h.old_lines);
            continue;
        }
        if h.old_start > line {
            break;
        }
        // `line` was replaced or deleted by this hunk.
        return match (h.new_lines, edge) {
            (0, Edge::Start) => h.new_start + 1,
            (0, Edge::End) => h.new_start,
            (_, Edge::Start) => h.new_start,
            (n, Edge::End) => h.new_start + n - 1,
        };
    }
    u32::try_from(i64::from(line) + delta).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: u32, end: u32) -> LineRange {
        LineRange::new(start, end).unwrap()
    }

    fn h(old_start: u32, old_lines: u32, new_start: u32, new_lines: u32) -> Hunk {
        Hunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
        }
    }

    fn map(range: LineRange, hunks: &[Hunk]) -> (Option<(u32, u32)>, Mapping) {
        let m = map_range(range, hunks);
        (m.range.map(|r| (r.start, r.end)), m.mapping)
    }

    #[test]
    fn no_changes_means_same_lines() {
        assert_eq!(map(r(10, 20), &[]), (Some((10, 20)), Mapping::Unchanged));
    }

    #[test]
    fn insertion_above_shifts_down_and_deletion_above_shifts_up() {
        // 3 lines inserted after line 2.
        assert_eq!(
            map(r(10, 20), &[h(2, 0, 3, 3)]),
            (Some((13, 23)), Mapping::Unchanged)
        );
        // Lines 2-4 deleted.
        assert_eq!(
            map(r(10, 20), &[h(2, 3, 1, 0)]),
            (Some((7, 17)), Mapping::Unchanged)
        );
        // Line 5 replaced by 4 lines.
        assert_eq!(
            map(r(10, 20), &[h(5, 1, 5, 4)]),
            (Some((13, 23)), Mapping::Unchanged)
        );
    }

    #[test]
    fn changes_below_do_not_move_the_range() {
        assert_eq!(
            map(r(10, 20), &[h(20, 0, 21, 5)]),
            (Some((10, 20)), Mapping::Unchanged)
        );
        assert_eq!(
            map(r(10, 20), &[h(30, 2, 30, 0)]),
            (Some((10, 20)), Mapping::Unchanged)
        );
    }

    #[test]
    fn edits_inside_the_range_mark_it_modified() {
        // Line 15 replaced by 2 lines: range grows by one.
        assert_eq!(
            map(r(10, 20), &[h(15, 1, 15, 2)]),
            (Some((10, 21)), Mapping::Modified)
        );
        // Insertion after line 12 (inside).
        assert_eq!(
            map(r(10, 20), &[h(12, 0, 13, 2)]),
            (Some((10, 22)), Mapping::Modified)
        );
        // Lines 18-25 deleted: the range is cut short.
        assert_eq!(
            map(r(10, 20), &[h(18, 8, 17, 0)]),
            (Some((10, 17)), Mapping::Modified)
        );
        // Lines 5-12 replaced by 2 lines: the start snaps to the replacement.
        assert_eq!(
            map(r(10, 20), &[h(5, 8, 5, 2)]),
            (Some((5, 14)), Mapping::Modified)
        );
    }

    #[test]
    fn insertion_right_after_the_last_line_is_outside() {
        assert_eq!(
            map(r(10, 20), &[h(20, 0, 21, 1)]),
            (Some((10, 20)), Mapping::Unchanged)
        );
        assert_eq!(
            map(r(10, 20), &[h(9, 0, 10, 1)]),
            (Some((11, 21)), Mapping::Unchanged)
        );
    }

    #[test]
    fn fully_deleted_range_is_gone() {
        assert_eq!(map(r(10, 20), &[h(8, 15, 7, 0)]), (None, Mapping::Gone));
        // Deleted at the top of the file.
        assert_eq!(map(r(1, 3), &[h(1, 5, 0, 0)]), (None, Mapping::Gone));
    }

    #[test]
    fn several_hunks_combine() {
        let hunks = [
            h(1, 0, 1, 2),
            h(5, 1, 7, 1),
            h(12, 1, 14, 3),
            h(30, 4, 34, 0),
        ];
        // +2 before, line 5 replaced 1:1 (no shift), line 12 inside -> modified, +2 lines.
        assert_eq!(map(r(10, 20), &hunks), (Some((12, 24)), Mapping::Modified));
        assert_eq!(map(r(6, 9), &hunks), (Some((8, 11)), Mapping::Unchanged));
    }
}

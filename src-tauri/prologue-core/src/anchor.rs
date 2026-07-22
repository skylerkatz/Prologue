//! Re-anchoring: relocating a line comment inside a recomputed diff using
//! its stored code anchor.
//!
//! Strategy (deliberately simple and deterministic, per the spec's
//! "exact match first, then fuzzy match within the file"):
//!
//! 1. **Exact** — scan every hunk's same-side line sequence for a window
//!    whose contents equal the anchor's selected lines verbatim.
//! 2. **Fuzzy** — otherwise score every window by the mean per-line
//!    similarity of whitespace-trimmed lines (common prefix + suffix
//!    character overlap) and accept the best window scoring at least
//!    [`FUZZY_THRESHOLD`].
//!
//! Candidates are ranked by (exactness, score, surrounding-context
//! agreement, distance from the comment's previous position, earliest
//! position) — every step is a total order, so the result is deterministic.
//! An exact match with agreeing context is `Anchored`; anything else that
//! matched is `Changed` ("code changed since commented"); no acceptable
//! window means `Orphaned`.
//!
//! Windows never span hunks: anchors are captured within a single hunk, and
//! code that migrated across hunk boundaries reads as changed enough to
//! orphan.

use crate::diff::{CommentSide, DiffLine, FileDiff};
use crate::error::CoreError;
use serde::{Deserialize, Serialize};

/// Minimum mean per-line similarity for a fuzzy window to count as a match.
const FUZZY_THRESHOLD: f64 = 0.6;

/// How many unchanged same-side lines the code anchor keeps on each side of
/// the selection.
const ANCHOR_CONTEXT: usize = 3;

/// Enough verbatim code to re-locate a line comment after edits: the selected
/// lines, up to [`ANCHOR_CONTEXT`] same-side lines around them, and the hunk
/// header. Stored as JSON in the `code_anchor` column.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnchor {
    pub hunk_header: String,
    pub context_before: Vec<String>,
    pub lines: Vec<String>,
    pub context_after: Vec<String>,
}

/// Build the code anchor for a line selection: the selected lines verbatim
/// (on `side`), up to [`ANCHOR_CONTEXT`] same-side lines around them, and the
/// containing hunk's header. The selection must fall inside a single hunk —
/// the UI constrains selections the same way.
pub(crate) fn extract_anchor(
    diff: &FileDiff,
    side: CommentSide,
    start: u32,
    end: u32,
) -> Result<CodeAnchor, CoreError> {
    let lineno = |line: &DiffLine| match side {
        CommentSide::Old => line.old_lineno,
        CommentSide::New => line.new_lineno,
    };
    for hunk in &diff.hunks {
        let mut first: Option<usize> = None;
        let mut last = 0;
        let mut selected = Vec::new();
        for (i, line) in hunk.lines.iter().enumerate() {
            let Some(n) = lineno(line) else { continue };
            if n < start || n > end {
                continue;
            }
            first.get_or_insert(i);
            last = i;
            selected.push(line.content.clone());
        }
        let Some(first) = first else { continue };
        if selected.len() as u32 != end - start + 1 {
            return Err(CoreError::SelectionCrossesHunks);
        }
        let mut context_before: Vec<String> = hunk.lines[..first]
            .iter()
            .rev()
            .filter(|l| lineno(l).is_some())
            .take(ANCHOR_CONTEXT)
            .map(|l| l.content.clone())
            .collect();
        context_before.reverse();
        let context_after: Vec<String> = hunk.lines[last + 1..]
            .iter()
            .filter(|l| lineno(l).is_some())
            .take(ANCHOR_CONTEXT)
            .map(|l| l.content.clone())
            .collect();
        return Ok(CodeAnchor {
            hunk_header: hunk.header.clone(),
            context_before,
            lines: selected,
            context_after,
        });
    }
    Err(CoreError::NoDiffLines {
        path: diff.path.clone(),
        start,
        end,
        side: side.as_str(),
    })
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AnchorStatus {
    /// Anchor lines found verbatim with unchanged surrounding context.
    Anchored,
    /// Anchor located, but the code changed since the comment was made
    /// (fuzzy match, or exact match with different surrounding context).
    Changed,
    /// The anchor cannot be located in the current diff.
    Orphaned,
}

/// A successful relocation: the anchor's new line range on its side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relocation {
    pub start_line: u32,
    pub end_line: u32,
    /// True when the code around (or inside) the anchor changed.
    pub changed: bool,
}

/// Locate `anchor` in `diff` on `side`. `prev_start` (the comment's last
/// known start line) breaks ties between equally good windows.
pub fn relocate(
    diff: &FileDiff,
    side: CommentSide,
    anchor: &CodeAnchor,
    prev_start: u32,
) -> Option<Relocation> {
    let n = anchor.lines.len();
    if n == 0 {
        return None;
    }

    let mut best: Option<Candidate> = None;
    for hunk in &diff.hunks {
        let seq: Vec<(u32, &str)> = hunk
            .lines
            .iter()
            .filter_map(|l| lineno(l, side).map(|no| (no, l.content.as_str())))
            .collect();
        for i in 0..seq.len().saturating_sub(n - 1) {
            let window = &seq[i..i + n];
            let exact = window
                .iter()
                .zip(&anchor.lines)
                .all(|((_, got), want)| got == want);
            let score = if exact {
                1.0
            } else {
                window
                    .iter()
                    .zip(&anchor.lines)
                    .map(|((_, got), want)| line_similarity(got, want))
                    .sum::<f64>()
                    / n as f64
            };
            if !exact && score < FUZZY_THRESHOLD {
                continue;
            }
            let candidate = Candidate {
                exact,
                score,
                context_matches: context_matches(&seq, i, n, anchor),
                distance: seq[i].0.abs_diff(prev_start),
                start_line: seq[i].0,
                end_line: seq[i + n - 1].0,
            };
            if best.as_ref().is_none_or(|b| candidate.beats(b)) {
                best = Some(candidate);
            }
        }
    }

    best.map(|b| Relocation {
        start_line: b.start_line,
        end_line: b.end_line,
        changed: !(b.exact && b.context_matches),
    })
}

struct Candidate {
    exact: bool,
    score: f64,
    context_matches: bool,
    distance: u32,
    start_line: u32,
    end_line: u32,
}

impl Candidate {
    /// Total preference order; ties fall through to the earlier candidate,
    /// which combined with the in-order scan makes selection deterministic.
    fn beats(&self, other: &Self) -> bool {
        (self.exact, self.score, self.context_matches, other.distance)
            > (other.exact, other.score, other.context_matches, self.distance)
    }
}

/// The stored context still surrounds the window: the same-side lines
/// immediately before/after it (up to the stored context's length) must
/// equal the stored context exactly. Fewer available lines than stored ones
/// means the neighborhood was restructured — treated as changed.
fn context_matches(seq: &[(u32, &str)], i: usize, n: usize, anchor: &CodeAnchor) -> bool {
    let before_ok = anchor.context_before.len() <= i
        && seq[i - anchor.context_before.len()..i]
            .iter()
            .zip(&anchor.context_before)
            .all(|((_, got), want)| got == want);
    let after = &seq[i + n..];
    let after_ok = anchor.context_after.len() <= after.len()
        && after
            .iter()
            .zip(&anchor.context_after)
            .all(|((_, got), want)| got == want);
    before_ok && after_ok
}

fn lineno(line: &DiffLine, side: CommentSide) -> Option<u32> {
    match side {
        CommentSide::Old => line.old_lineno,
        CommentSide::New => line.new_lineno,
    }
}

/// Similarity of two lines ignoring leading/trailing whitespace: the shared
/// prefix + suffix character count relative to the combined length. 1.0 for
/// trim-equal lines, 0.0 when either side is blank and the other isn't.
fn line_similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (a.trim(), b.trim());
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let prefix = av.iter().zip(&bv).take_while(|(x, y)| x == y).count();
    let suffix = av
        .iter()
        .rev()
        .zip(bv.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let overlap = (prefix + suffix).min(av.len().min(bv.len()));
    2.0 * overlap as f64 / (av.len() + bv.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileStatus, Hunk, LineKind};

    fn ctx(old: u32, new: u32, content: &str) -> DiffLine {
        DiffLine {
            kind: LineKind::Context,
            old_lineno: Some(old),
            new_lineno: Some(new),
            content: content.to_owned(),
            intraline: None,
        }
    }

    fn add(new: u32, content: &str) -> DiffLine {
        DiffLine {
            kind: LineKind::Addition,
            old_lineno: None,
            new_lineno: Some(new),
            content: content.to_owned(),
            intraline: None,
        }
    }

    fn file_diff(hunks: Vec<Hunk>) -> FileDiff {
        FileDiff {
            path: "file.txt".into(),
            old_path: None,
            status: FileStatus::Modified,
            binary: false,
            hunks,
            new_total_lines: Some(100),
        }
    }

    fn hunk(lines: Vec<DiffLine>) -> Hunk {
        Hunk {
            header: "@@ test @@".into(),
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
            lines,
        }
    }

    fn anchor(before: &[&str], lines: &[&str], after: &[&str]) -> CodeAnchor {
        CodeAnchor {
            hunk_header: "@@ test @@".into(),
            context_before: before.iter().map(|s| (*s).to_owned()).collect(),
            lines: lines.iter().map(|s| (*s).to_owned()).collect(),
            context_after: after.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn exact_match_with_matching_context_is_anchored() {
        let diff = file_diff(vec![hunk(vec![
            ctx(1, 1, "one"),
            add(2, "two"),
            add(3, "three"),
            ctx(2, 4, "four"),
        ])]);
        let a = anchor(&["one"], &["two", "three"], &["four"]);
        let r = relocate(&diff, CommentSide::New, &a, 2).unwrap();
        assert_eq!((r.start_line, r.end_line, r.changed), (2, 3, false));
    }

    #[test]
    fn exact_match_with_different_context_is_changed() {
        let diff = file_diff(vec![hunk(vec![
            ctx(1, 1, "REWRITTEN"),
            add(2, "two"),
            add(3, "three"),
            ctx(2, 4, "four"),
        ])]);
        let a = anchor(&["one"], &["two", "three"], &["four"]);
        let r = relocate(&diff, CommentSide::New, &a, 2).unwrap();
        assert_eq!((r.start_line, r.end_line, r.changed), (2, 3, true));
    }

    #[test]
    fn missing_context_lines_count_as_changed() {
        // The window sits at the hunk edge, so the stored context line
        // before it is gone.
        let diff = file_diff(vec![hunk(vec![
            add(2, "two"),
            add(3, "three"),
            ctx(2, 4, "four"),
        ])]);
        let a = anchor(&["one"], &["two", "three"], &["four"]);
        let r = relocate(&diff, CommentSide::New, &a, 2).unwrap();
        assert!(r.changed);
    }

    #[test]
    fn fuzzy_match_relocates_slightly_edited_lines() {
        let diff = file_diff(vec![hunk(vec![
            ctx(1, 1, "one"),
            add(2, "two edited"),
            add(3, "three"),
            ctx(2, 4, "four"),
        ])]);
        let a = anchor(&["one"], &["two", "three"], &["four"]);
        let r = relocate(&diff, CommentSide::New, &a, 2).unwrap();
        assert_eq!((r.start_line, r.end_line, r.changed), (2, 3, true));
    }

    #[test]
    fn heavily_rewritten_lines_do_not_match() {
        let diff = file_diff(vec![hunk(vec![
            ctx(1, 1, "one"),
            add(2, "completely different now"),
            add(3, "nothing in common"),
            ctx(2, 4, "four"),
        ])]);
        let a = anchor(&["one"], &["two", "three"], &["four"]);
        assert!(relocate(&diff, CommentSide::New, &a, 2).is_none());
    }

    #[test]
    fn ambiguous_exact_matches_prefer_matching_context_then_proximity() {
        // The anchor line appears three times; only the middle occurrence
        // has the stored context.
        let diff = file_diff(vec![hunk(vec![
            add(1, "dup"),
            ctx(1, 2, "lead"),
            add(3, "dup"),
            ctx(2, 4, "tail"),
            add(5, "dup"),
        ])]);
        let a = anchor(&["lead"], &["dup"], &["tail"]);
        let r = relocate(&diff, CommentSide::New, &a, 1).unwrap();
        assert_eq!((r.start_line, r.changed), (3, false));

        // Without any context stored, proximity to the previous position
        // decides.
        let bare = anchor(&[], &["dup"], &[]);
        let r = relocate(&diff, CommentSide::New, &bare, 5).unwrap();
        assert_eq!(r.start_line, 5);
        let r = relocate(&diff, CommentSide::New, &bare, 1).unwrap();
        assert_eq!(r.start_line, 1);
    }

    #[test]
    fn old_side_anchors_match_deletion_lines() {
        let del = |old: u32, content: &str| DiffLine {
            kind: LineKind::Deletion,
            old_lineno: Some(old),
            new_lineno: None,
            content: content.to_owned(),
            intraline: None,
        };
        let diff = file_diff(vec![hunk(vec![
            ctx(9, 9, "keep"),
            del(10, "doomed one"),
            del(11, "doomed two"),
            ctx(12, 10, "keep too"),
        ])]);
        let a = anchor(&["keep"], &["doomed one", "doomed two"], &["keep too"]);
        let r = relocate(&diff, CommentSide::Old, &a, 10).unwrap();
        assert_eq!((r.start_line, r.end_line, r.changed), (10, 11, false));
    }
}

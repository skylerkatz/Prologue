use serde::Serialize;

use crate::diff::{DiffLine, LineKind};

/// A changed span within one line, in UTF-16 code units (`end` exclusive).
/// UTF-16 because the frontend slices JavaScript strings, whose indices are
/// UTF-16 code units — the renderer applies these ranges verbatim.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct IntralineRange {
    pub start: u32,
    pub end: u32,
}

/// Lines with more tokens than this skip intraline diffing; the DP table is
/// quadratic and lines that long aren't readable word-by-word anyway.
const MAX_TOKENS: usize = 200;

/// Below this token-level similarity a pair is a rewrite, not an edit —
/// highlighting nearly everything is noise, so highlight nothing.
const MIN_SIMILARITY: f64 = 0.4;

/// Fill `intraline` on paired deletion/addition lines within one hunk's
/// line list. A run of deletions immediately followed by a run of additions
/// pairs index-wise (GitHub-style); leftover unpaired lines stay `None`.
pub fn apply_intraline(lines: &mut [DiffLine]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Deletion {
            i += 1;
            continue;
        }
        let del_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Deletion {
            i += 1;
        }
        let add_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Addition {
            i += 1;
        }
        let pairs = (add_start - del_start).min(i - add_start);
        for p in 0..pairs {
            let (del, add) = (del_start + p, add_start + p);
            if let Some((del_ranges, add_ranges)) =
                compute_intraline(&lines[del].content, &lines[add].content)
            {
                lines[del].intraline = Some(del_ranges);
                lines[add].intraline = Some(add_ranges);
            }
        }
    }
}

/// The changed spans between one old/new line pair, or `None` when the pair
/// is effectively a rewrite (or too long) and should render un-highlighted.
/// Identical lines yield `Some` with empty range lists.
pub fn compute_intraline(
    old: &str,
    new: &str,
) -> Option<(Vec<IntralineRange>, Vec<IntralineRange>)> {
    if old == new {
        return Some((Vec::new(), Vec::new()));
    }
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    if old_tokens.len() > MAX_TOKENS || new_tokens.len() > MAX_TOKENS {
        return None;
    }

    let (old_common, new_common) = lcs_common(&old_tokens, &new_tokens);

    // Similarity in UTF-16 units: 2·common / (len(old) + len(new)). Common
    // tokens are identical strings, so measuring them on one side suffices.
    let common_units: usize = old_tokens
        .iter()
        .zip(&old_common)
        .filter(|(_, common)| **common)
        .map(|(token, _)| utf16_len(token))
        .sum();
    let total_units = utf16_len(old) + utf16_len(new);
    let similarity = (2.0 * common_units as f64) / total_units.max(1) as f64;
    if total_units > 0 && similarity < MIN_SIMILARITY {
        return None;
    }

    Some((
        changed_ranges(&old_tokens, &old_common),
        changed_ranges(&new_tokens, &new_common),
    ))
}

/// Split into word / whitespace / single-symbol tokens, so the LCS aligns
/// on word boundaries instead of characters.
fn tokenize(s: &str) -> Vec<&str> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Symbol,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Symbol
        }
    };
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut iter = s.char_indices().peekable();
    while let Some((idx, c)) = iter.next() {
        let this = class(c);
        let split_after = match iter.peek() {
            // Symbols stay single tokens; word/space runs group.
            Some((_, next)) => this == Class::Symbol || class(*next) != this,
            None => true,
        };
        if split_after {
            let end = idx + c.len_utf8();
            tokens.push(&s[start..end]);
            start = end;
        }
    }
    tokens
}

/// Longest-common-subsequence membership flags for both token lists.
fn lcs_common(old: &[&str], new: &[&str]) -> (Vec<bool>, Vec<bool>) {
    let (n, m) = (old.len(), new.len());
    // dp[i][j] = LCS length of old[i..] vs new[j..], flattened.
    let mut dp = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[at(i, j)] = if old[i] == new[j] {
                dp[at(i + 1, j + 1)] + 1
            } else {
                dp[at(i + 1, j)].max(dp[at(i, j + 1)])
            };
        }
    }
    let mut old_common = vec![false; n];
    let mut new_common = vec![false; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            old_common[i] = true;
            new_common[j] = true;
            i += 1;
            j += 1;
        } else if dp[at(i + 1, j)] >= dp[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (old_common, new_common)
}

/// Merge consecutive non-common tokens into UTF-16 ranges.
fn changed_ranges(tokens: &[&str], common: &[bool]) -> Vec<IntralineRange> {
    let mut ranges: Vec<IntralineRange> = Vec::new();
    let mut offset = 0u32;
    for (token, is_common) in tokens.iter().zip(common) {
        let len = utf16_len(token) as u32;
        if !is_common {
            match ranges.last_mut() {
                Some(last) if last.end == offset => last.end += len,
                _ => ranges.push(IntralineRange {
                    start: offset,
                    end: offset + len,
                }),
            }
        }
        offset += len;
    }
    ranges
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(pairs: &[(u32, u32)]) -> Vec<IntralineRange> {
        pairs
            .iter()
            .map(|&(start, end)| IntralineRange { start, end })
            .collect()
    }

    #[test]
    fn identical_lines_have_no_changed_ranges() {
        let (old, new) = compute_intraline("let x = 1;", "let x = 1;").unwrap();
        assert!(old.is_empty());
        assert!(new.is_empty());
    }

    #[test]
    fn a_single_changed_word_is_ranged_on_both_sides() {
        let (old, new) = compute_intraline(
            "        $result = strtoupper($input);",
            "        $result = strtolower($input);",
        )
        .unwrap();
        // 8 spaces + "$result = " puts "strtoupper"/"strtolower" at offset
        // 18, 10 units long.
        assert_eq!(old, ranges(&[(18, 28)]));
        assert_eq!(new, ranges(&[(18, 28)]));
    }

    #[test]
    fn insertion_only_ranges_the_new_side() {
        let (old, new) = compute_intraline("foo(a, c)", "foo(a, b, c)").unwrap();
        assert!(old.is_empty());
        // ", b" or "b, " depending on alignment — either way 3 units inside
        // the argument list, and nothing flagged on the old side.
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].end - new[0].start, 3);
        assert!(new[0].start >= 5 && new[0].end <= 10);
    }

    #[test]
    fn adjacent_changed_tokens_merge_into_one_range() {
        let (old, new) = compute_intraline("value = compute(n)", "value = computeFast(n)").unwrap();
        // "compute" != "computeFast": one contiguous changed word each side.
        assert_eq!(old, ranges(&[(8, 15)]));
        assert_eq!(new, ranges(&[(8, 19)]));
    }

    #[test]
    fn a_full_rewrite_returns_none() {
        assert_eq!(
            compute_intraline("return the_old_thing();", "completely different words here"),
            None,
        );
    }

    #[test]
    fn unicode_ranges_are_utf16_code_units() {
        // "héllo" is 5 UTF-16 units, "🎉" is 2 (a surrogate pair).
        let (old, new) = compute_intraline("héllo wörld", "héllo 🎉wörld").unwrap();
        assert!(old.is_empty());
        assert_eq!(new, ranges(&[(6, 8)]));
    }

    #[test]
    fn changed_word_after_a_surrogate_pair_offsets_in_utf16() {
        let (old, new) = compute_intraline("🎉 old", "🎉 new").unwrap();
        // The emoji is 2 units + 1 space: the word starts at 3.
        assert_eq!(old, ranges(&[(3, 6)]));
        assert_eq!(new, ranges(&[(3, 6)]));
    }

    #[test]
    fn overlong_lines_are_skipped() {
        let old = "x ".repeat(300);
        let new = format!("{old}y");
        assert_eq!(compute_intraline(&old, &new), None);
    }

    #[test]
    fn pairing_matches_deletion_runs_with_addition_runs() {
        let line = |kind: LineKind, content: &str| DiffLine {
            kind,
            old_lineno: None,
            new_lineno: None,
            content: content.to_owned(),
            intraline: None,
        };
        let mut lines = vec![
            line(LineKind::Context, "unchanged"),
            line(LineKind::Deletion, "let a = old_one;"),
            line(LineKind::Deletion, "entirely gone line"),
            line(LineKind::Addition, "let a = new_one;"),
            line(LineKind::Context, "tail"),
        ];
        apply_intraline(&mut lines);

        // Pair 0: deletion[0] with addition[0].
        assert_eq!(lines[1].intraline, Some(ranges(&[(8, 15)])));
        assert_eq!(lines[3].intraline, Some(ranges(&[(8, 15)])));
        // Unpaired deletion and context lines stay untouched.
        assert_eq!(lines[2].intraline, None);
        assert_eq!(lines[0].intraline, None);
        assert_eq!(lines[4].intraline, None);
    }
}

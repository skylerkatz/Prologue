//! `prologue guide`: the stored review guide — ordered sections grouping
//! the changed files, with each file's current status and line counts when
//! the diff is still computable.

use prologue_core::diff::{self, DiffSpec, DiffSummary, FileStatus, FileSummary};
use prologue_core::guide::Guide;
use prologue_core::review::Review;
use prologue_core::rusqlite::Connection;
use std::fmt::Write as _;

/// The stored guide for a review's current diff coordinates, or a clear
/// error. Guides are read-only here — generation lives in the app.
pub fn find_guide(conn: &Connection, review: &Review) -> Result<Guide, String> {
    prologue_core::guide::find_guide_impl(
        conn,
        review.id,
        &review.base_ref,
        &review.branch,
        review.mode,
    )?
    .ok_or_else(|| {
        format!(
            "No guide for review {} ({} vs {}, {}) — generate one in the Prologue app",
            review.id,
            review.branch,
            review.base_ref,
            review.mode.as_str()
        )
    })
}

/// The review's current diff summary, for per-file status and +/- counts.
/// Computing it can legitimately fail (archived review whose branch is
/// gone); degrade to paths-only output rather than refusing to print the
/// guide.
pub fn current_summary(review: &Review) -> Option<DiffSummary> {
    match diff::get_diff_summary(&DiffSpec::from(review), false) {
        Ok(summary) => Some(summary),
        Err(e) => {
            eprintln!("warning: could not compute the current diff ({e}); showing paths only");
            None
        }
    }
}

pub fn render_text(review: &Review, guide: &Guide, summary: Option<&DiffSummary>) -> String {
    let mut out = format!(
        "Review #{} — {} @ {} vs {} ({}, {})\n",
        review.id,
        crate::show::repo_name(&review.repo_path),
        review.branch,
        review.base_ref,
        review.mode.as_str(),
        review.status,
    );
    let file_count: usize = guide.sections.iter().map(|s| s.files.len()).sum();
    writeln!(
        out,
        "Guide: {} section(s), {} file(s) — model {}, {}",
        guide.sections.len(),
        file_count,
        guide.model,
        guide.created_at,
    )
    .unwrap();

    let total = guide.sections.len();
    // 01/05-style ordinals; the width grows with the section count.
    let width = total.to_string().len().max(2);
    for (index, section) in guide.sections.iter().enumerate() {
        writeln!(out, "\n{:0width$}/{:0width$} {}", index + 1, total, section.title).unwrap();
        for line in section.summary.lines() {
            if line.is_empty() {
                out.push('\n');
            } else {
                writeln!(out, "  {line}").unwrap();
            }
        }
        for path in &section.files {
            writeln!(out, "  {}", file_line(path, summary)).unwrap();
        }
    }
    out
}

/// `M code.txt +2 -1` when the file is in the current diff, the bare path
/// when it isn't (diff uncomputable, or the file left the diff since the
/// guide was generated).
fn file_line(path: &str, summary: Option<&DiffSummary>) -> String {
    let Some(file) = summary.and_then(|s| s.files.iter().find(|f| f.path == path)) else {
        return path.to_owned();
    };
    if file.binary {
        return format!("{} {path} (binary)", status_char(file));
    }
    format!("{} {path} +{} -{}", status_char(file), file.additions, file.deletions)
}

fn status_char(file: &FileSummary) -> char {
    match file.status {
        FileStatus::Added => 'A',
        FileStatus::Modified => 'M',
        FileStatus::Deleted => 'D',
        FileStatus::Renamed => 'R',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prologue_core::diff::DiffMode;
    use prologue_core::guide::{save_guide_impl, GuideSection, NewGuide};
    use prologue_core::review::open_review_impl;
    use prologue_core::testutil::{open_test_db as test_db, FixtureRepo};
    use std::collections::BTreeMap;

    /// A review of the standard fixture with a stored two-section guide
    /// covering its changed files (code.txt modified, d.txt deleted).
    fn seeded(dir: &tempfile::TempDir) -> (Connection, FixtureRepo, Review, Guide) {
        let conn = test_db(dir);
        let fixture = FixtureRepo::standard_review_fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let summary =
            diff::get_diff_summary(&DiffSpec::from(&review), false).unwrap();
        let fingerprints: BTreeMap<String, String> =
            summary.files.iter().map(|f| (f.path.clone(), f.fingerprint.clone())).collect();
        let guide = save_guide_impl(
            &conn,
            &NewGuide {
                review_id: review.id,
                base_ref: review.base_ref.clone(),
                head_ref: review.branch.clone(),
                mode: review.mode,
                fingerprints,
                model: "claude-test".to_owned(),
                cost_usd: Some(0.01),
            },
            &[
                GuideSection {
                    title: "Core change".to_owned(),
                    summary: "Replaces line 6 of the code.\n\nTwo lines now.".to_owned(),
                    files: vec!["code.txt".to_owned()],
                },
                GuideSection {
                    title: "Cleanup".to_owned(),
                    summary: "Drops the doomed file.".to_owned(),
                    files: vec!["d.txt".to_owned()],
                },
            ],
        )
        .unwrap();
        (conn, fixture, review, guide)
    }

    #[test]
    fn finds_the_stored_guide_by_the_reviews_coordinates() {
        let dir = tempfile::tempdir().unwrap();
        let (conn, _fixture, review, stored) = seeded(&dir);

        let found = find_guide(&conn, &review).unwrap();
        assert_eq!(found.id, stored.id);
        assert_eq!(found.sections.len(), 2);
        assert_eq!(found.sections[0].title, "Core change");
        assert_eq!(found.sections[1].files, vec!["d.txt".to_owned()]);
    }

    #[test]
    fn a_review_without_a_guide_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = FixtureRepo::standard_review_fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();

        let err = find_guide(&conn, &review).unwrap_err();
        assert!(err.contains(&format!("No guide for review {}", review.id)), "{err}");
        assert!(err.contains("feature vs main, committed"), "{err}");
        assert!(err.contains("Prologue app"), "{err}");
    }

    #[test]
    fn renders_sections_with_ordinals_summaries_and_file_stats() {
        let dir = tempfile::tempdir().unwrap();
        let (_conn, _fixture, review, guide) = seeded(&dir);

        let summary = current_summary(&review).expect("fixture diff computes");
        let text = render_text(&review, &guide, Some(&summary));
        assert!(text.contains("feature vs main (committed, active)"), "{text}");
        assert!(text.contains("Guide: 2 section(s), 2 file(s) — model claude-test"), "{text}");
        assert!(text.contains("01/02 Core change"), "{text}");
        assert!(text.contains("02/02 Cleanup"), "{text}");
        // Summary prose is indented, blank lines preserved.
        assert!(text.contains("  Replaces line 6 of the code.\n\n  Two lines now.\n"), "{text}");
        // The standard fixture: code.txt +2 -1 (modified), d.txt deleted.
        assert!(text.contains("  M code.txt +2 -1"), "{text}");
        assert!(text.contains("  D d.txt +0 -2"), "{text}");
    }

    #[test]
    fn renders_bare_paths_when_the_diff_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let (_conn, _fixture, review, guide) = seeded(&dir);

        let text = render_text(&review, &guide, None);
        assert!(text.contains("\n  code.txt\n"), "{text}");
        assert!(text.contains("\n  d.txt\n"), "{text}");
        assert!(!text.contains("M code.txt"), "{text}");
    }

    #[test]
    fn files_that_left_the_diff_fall_back_to_bare_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (_conn, fixture, review, guide) = seeded(&dir);

        // Restore d.txt on feature: it leaves the diff, the guide still
        // lists it.
        fixture.commit_file("d.txt", "doomed one\ndoomed two\n", "restore");
        let summary = current_summary(&review).expect("fixture diff computes");
        let text = render_text(&review, &guide, Some(&summary));
        assert!(text.contains("  M code.txt +2 -1"), "{text}");
        assert!(text.contains("\n  d.txt\n"), "{text}");
    }

    #[test]
    fn ordinal_width_grows_with_the_section_count() {
        let dir = tempfile::tempdir().unwrap();
        let (_conn, _fixture, review, mut guide) = seeded(&dir);

        // Fake a 100-section guide: ordinals become 001/100.
        let filler = guide.sections[1].clone();
        guide.sections.resize(100, filler);
        let text = render_text(&review, &guide, None);
        assert!(text.contains("001/100 Core change"), "{text}");
        assert!(text.contains("100/100 Cleanup"), "{text}");
    }
}

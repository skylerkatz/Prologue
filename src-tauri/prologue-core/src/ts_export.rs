//! Generates `src/generated/ipc-types.ts` from the IPC-facing Rust types.
//!
//! Compiled for test builds only (the `ts` feature comes in through the
//! self dev-dependency), so release builds carry no ts-rs code. The test
//! regenerates the file and FAILS when it was stale — renaming a Rust
//! field breaks `cargo test` until the regenerated bindings are committed,
//! and `tsc --noEmit` then fails wherever the frontend used the old shape.

use ts_rs::TS;

const HEADER: &str = "\
// Generated from prologue-core's Rust types by the ts_export test.
// Do not edit — run `cargo test -p prologue-core` in src-tauri to refresh.

";

/// One `export …` declaration per IPC type, all in one file so cross-type
/// references stay plain identifiers (no import juggling).
fn bindings() -> String {
    let decls = [
        crate::repo::RepoInfo::decl(),
        crate::repo::BranchList::decl(),
        crate::diff::DiffMode::decl(),
        crate::diff::FileStatus::decl(),
        crate::diff::FileSummary::decl(),
        crate::diff::DiffSummary::decl(),
        crate::diff::LineKind::decl(),
        crate::intraline::IntralineRange::decl(),
        crate::diff::DiffLine::decl(),
        crate::diff::Hunk::decl(),
        crate::diff::FileDiff::decl(),
        crate::diff::ContextLines::decl(),
        crate::review::ReviewStatus::decl(),
        crate::review::Review::decl(),
        crate::review::OpenReviewResult::decl(),
        crate::review::ReviewedFile::decl(),
        crate::review::ArchivedReview::decl(),
        crate::comment::CommentLevel::decl(),
        crate::diff::CommentSide::decl(),
        crate::comment::CommentState::decl(),
        crate::anchor::CodeAnchor::decl(),
        crate::comment::Comment::decl(),
        crate::comment::NewComment::decl(),
        crate::anchor::AnchorStatus::decl(),
        crate::comment::ReanchorResult::decl(),
        crate::export::ExportFormat::decl(),
        crate::guide::GuideSection::decl(),
        crate::guide::Guide::decl(),
    ];
    let mut out = String::from(HEADER);
    for decl in decls {
        out.push_str("export ");
        out.push_str(&decl);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn generated_ipc_types_are_current() {
        let expected = super::bindings();
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/generated/ipc-types.ts");
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != expected {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &expected).unwrap();
            panic!(
                "src/generated/ipc-types.ts was out of date with the Rust types — \
                 regenerated it; review and commit the update, then re-run"
            );
        }
    }
}

//! Fast first load through the portal, where every request costs a full
//! round trip. Two pieces:
//!
//! - `load_review`: everything the review screen shows first (summary,
//!   review, re-anchored comments, the viewer's reviewed marks, the stored
//!   guide, and the hunks of the files that open expanded), from ONE
//!   computed diff, in one call. It replaces a chain of six dependent calls.
//! - `index`: serves index.html with that same state (plus the config and
//!   the viewer) inlined, so the first render needs no API round trip at
//!   all, and with preload hints for the fonts and highlighter chunks the
//!   page would otherwise discover late.

use super::{reviewer, web_start, AmpUser, AppState, ConfigResponse};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use prologue_core::diff::{self, DiffMode, DiffSpec, DiffSummary, FileDiff};
use prologue_core::guide::{self, Guide};
use prologue_core::repo::open_git_repo;
use prologue_core::review::{
    self, Comment, ReanchorResult, Review, ReviewStatus, ReviewedFile,
};
use prologue_runtime::guide_engine::{self, GuideCliStatus};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Files bigger than this open behind a "load anyway" guard instead of
/// loading (src/diff/guards.ts MAX_AUTO_LINES); never prefetched.
const MAX_AUTO_LINES: usize = 5000;
/// Changed lines of hunks sent up front, so a huge diff does not turn into
/// a huge first response; files past it load on scroll as before.
const PREFETCH_LINES: usize = 4000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedReview {
    summary: DiffSummary,
    review: Option<Review>,
    branch_merged: bool,
    anchors: Vec<ReanchorResult>,
    comments: Vec<Comment>,
    reviewed_files: Vec<ReviewedFile>,
    guide: Option<Guide>,
    guide_cli: GuideCliStatus,
    /// Hunks of the files that open expanded, in summary order, until
    /// PREFETCH_LINES; the rest load on demand.
    file_diffs: Vec<FileDiff>,
}

/// What the app does on opening a review (summary, auto-archive, resume or
/// start the review, re-anchor, read comments, marks, and guide), done
/// server-side in one call, plus the first files' hunks.
pub fn load_review(
    state: &AppState,
    repo_path: &str,
    branch: &str,
    base: &str,
    mode: DiffMode,
    ignore_whitespace: bool,
    user: Option<&AmpUser>,
) -> Result<LoadedReview, String> {
    let spec = DiffSpec {
        repo_path: repo_path.to_owned(),
        base: base.to_owned(),
        head: branch.to_owned(),
        mode,
    };
    let repo = open_git_repo(repo_path)?;
    let view = diff::RepoDiff::compute(&repo, &spec, ignore_whitespace)?;
    let summary = view.summary()?;
    // Re-anchoring always runs on the full diff, whatever the view shows.
    let full_diff = match ignore_whitespace {
        true => Some(diff::RepoDiff::compute(&repo, &spec, false)?),
        false => None,
    };
    let full = full_diff.as_ref().unwrap_or(&view);

    let conn = state.db.lock()?;
    review::archive_stale_reviews_impl(&conn, &repo, repo_path)?;
    let opened = review::open_review_checked_impl(&conn, &repo, repo_path, branch, base, mode)?;
    let active = opened.review.as_ref().filter(|r| r.status == ReviewStatus::Active);
    let (anchors, comments, reviewed_files, guide) = match active {
        Some(r) => {
            let (anchors, comments) = review::reanchor_comments_in(&conn, full, r.id)?;
            (
                anchors,
                comments,
                review::list_reviewed_files_impl(&conn, r.id, reviewer(user))?,
                guide::find_guide_impl(&conn, r.id, base, branch, mode)?,
            )
        }
        None => Default::default(),
    };
    drop(conn);

    // Reviewed files open collapsed, so their hunks are not needed yet.
    let reviewed: HashMap<&str, &str> = reviewed_files
        .iter()
        .map(|f| (f.file_path.as_str(), f.fingerprint.as_str()))
        .collect();
    let mut budget = PREFETCH_LINES;
    let mut file_diffs = Vec::new();
    for file in &summary.files {
        let lines = file.additions + file.deletions;
        if file.binary
            || lines > MAX_AUTO_LINES
            || lines > budget
            || reviewed.get(file.path.as_str()) == Some(&file.fingerprint.as_str())
        {
            continue;
        }
        if let Ok(hunks) = view.file_diff(&file.path) {
            budget -= lines;
            file_diffs.push(hunks);
        }
    }

    Ok(LoadedReview {
        summary,
        review: opened.review,
        branch_merged: opened.branch_merged,
        anchors,
        comments,
        reviewed_files,
        guide,
        guide_cli: guide_engine::guide_cli_status(),
        file_diffs,
    })
}

/// The view preferences the browser keeps in localStorage, mirrored into
/// the `prologue-view` cookie (`<mode>.<0|1 hide whitespace>`) so the
/// inlined review matches what the page will ask for. The mode is None
/// (`auto`) until the viewer picks one: then a checkout with uncommitted
/// work opens in the working-tree mode, as App.tsx decides too.
fn view_from_cookie(headers: &HeaderMap) -> (Option<DiffMode>, bool) {
    let value = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix("prologue-view="));
    let Some((mode, ws)) = value.and_then(|v| v.split_once('.')) else {
        return (None, false);
    };
    (serde_json::from_value(json!(mode)).ok(), ws == "1")
}

/// The state index.html carries: what /api/config and /api/me answer, and
/// the review the page opens first with the coordinates it was loaded for
/// (the page uses it only when its own choice matches). Lookups that fail
/// are left out; the page then makes those calls itself.
pub fn boot_state(state: &AppState, user: Option<&AmpUser>, headers: &HeaderMap) -> Value {
    let start = web_start(state).ok();
    let review = start.as_ref().and_then(|s| {
        let repo_path = s.repo["path"].as_str()?;
        let branch = s.branch_list["current"].as_str()?;
        // App.tsx showRepo: resume the active review's base if it still
        // exists, else the default base.
        let stored = s.active_review["baseRef"].as_str();
        let branches = s.branch_list["branches"].as_array()?;
        let base = stored
            .filter(|b| branches.iter().any(|x| x == b))
            .or_else(|| s.branch_list["defaultBase"].as_str())?;
        let (mode, ignore_whitespace) = view_from_cookie(headers);
        let mode = mode.unwrap_or(match s.branch_list["uncommitted"].as_bool() {
            Some(true) => DiffMode::All,
            _ => DiffMode::Committed,
        });
        let data =
            load_review(state, repo_path, branch, base, mode, ignore_whitespace, user).ok()?;
        Some(json!({
            "repoPath": repo_path, "branch": branch, "base": base, "mode": mode,
            "ignoreWhitespace": ignore_whitespace, "data": data,
        }))
    });
    json!({
        "config": ConfigResponse { config: state.config.clone(), start },
        "me": user,
        "review": review,
    })
}

/// Hashed assets the first paint needs but discovers late: the fonts the
/// CSS names, and the highlighter's chunks, which are fetched only once a
/// hunk renders (the wasm one only after the engine loads).
const PRELOADS: &[(&str, &str)] = &[
    ("ibm-plex-sans-latin-400-normal-", ".woff2"),
    ("ibm-plex-sans-latin-500-normal-", ".woff2"),
    ("ibm-plex-sans-latin-600-normal-", ".woff2"),
    ("ibm-plex-mono-latin-400-normal-", ".woff2"),
    ("lora-latin-600-normal-", ".woff2"),
    ("core-", ".js"),
    ("engine-oniguruma-", ".js"),
    ("github-light-", ".js"),
    ("github-dark-", ".js"),
    ("wasm-", ".js"),
];

fn preload_links(dist: &std::path::Path) -> String {
    let Ok(entries) = std::fs::read_dir(dist.join("assets")) else {
        return String::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| PRELOADS.iter().any(|(p, s)| n.starts_with(p) && n.ends_with(s)))
        .collect();
    names.sort();
    names
        .iter()
        .map(|n| match n.ends_with(".js") {
            true => format!(r#"<link rel="modulepreload" crossorigin href="/assets/{n}">"#),
            false => format!(
                r#"<link rel="preload" as="font" type="font/woff2" crossorigin href="/assets/{n}">"#
            ),
        })
        .collect()
}

/// JSON that is safe inside a <script> element: comment bodies are user
/// text, and a literal `</script>` in one must not end the element.
pub(crate) fn script_json(value: &Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// index.html for `/` and every client-side route, with the boot state and
/// preload hints added. Never cached: it carries live review state.
pub async fn index(State(state): State<Arc<AppState>>, uri: Uri, headers: HeaderMap) -> Response {
    // A missing file (favicon.ico, a stale asset) is not a page.
    if uri.path().rsplit('/').next().is_some_and(|last| last.contains('.'))
        && uri.path() != "/index.html"
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(html) = std::fs::read_to_string(state.dist.join("index.html")) else {
        return (StatusCode::NOT_FOUND, "No index.html — run `npm run build`").into_response();
    };
    let user = super::AmpUser::from_headers(&headers, &state.owner);
    let boot = {
        let state = Arc::clone(&state);
        tokio::task::spawn_blocking(move || boot_state(&state, user.as_ref(), &headers))
            .await
            .unwrap_or(Value::Null)
    };
    let inject = format!(
        "{}<script id=\"prologue-boot\" type=\"application/json\">{}</script>\n  </head>",
        preload_links(&state.dist),
        script_json(&boot)
    );
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        // data-ui="web" from the first paint, not once main.tsx runs: it
        // applies the web theme and shows index.html's loading skeleton.
        html.replacen("<html", "<html data-ui=\"web\"", 1)
            .replacen("</head>", &inject, 1),
    )
        .into_response()
}

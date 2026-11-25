//! `prologue-server` — Prologue without Tauri or a display. Serves the built
//! frontend and exposes the app's commands over HTTP so the same UI runs in
//! a browser (e.g. through an Amp portal into a headless Linux orb):
//!
//! - `POST /api/invoke/{command}` with the same JSON args object the Tauri
//!   `invoke` call takes (camelCase keys); replies 200 with the command's
//!   JSON result, or 4xx/5xx with the error string as a JSON string.
//! - `GET /api/events` — Server-Sent Events carrying what the app emits as
//!   Tauri events (`repo-changed`, `comments-changed`). The first event of
//!   every connection is `hello`, so a reconnecting client can catch up.
//! - `GET /api/config` — the repo and base ref this server was started for.
//! - `GET /api/me` — the Amp user the portal says is asking (or null).
//!
//! The command bodies are the same `prologue-core` / `prologue-runtime`
//! calls `src-tauri/src/commands.rs` makes; only the transport differs.

mod amp;
mod boot;

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use prologue_core::db::{self, Db};
use prologue_core::diff::{self, DiffMode, DiffSpec};
use prologue_core::export::{self, ExportFormat};
use prologue_core::guide;
use prologue_core::repo::{self, open_git_repo};
use prologue_core::review::{self, CommentState, NewComment};
use prologue_runtime::events;
use prologue_runtime::guide_engine::{self, GuideRuntime};
use prologue_runtime::watcher::{self, RepoWatcher};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use tower_http::compression::{CompressionLayer, CompressionLevel};
use tower_http::services::ServeDir;

#[derive(Parser)]
#[command(about = "Headless Prologue: the review UI over HTTP")]
struct Cli {
    /// Git repository to review; the UI opens it on load.
    #[arg(long, env = "PROLOGUE_REPO")]
    repo: PathBuf,
    /// Base ref the diff is computed against when the branch has no review
    /// yet (default: auto-detected, as in the app).
    #[arg(long, env = "PROLOGUE_BASE")]
    base: Option<String>,
    /// Port to listen on.
    #[arg(long, env = "PORT", default_value_t = 8080)]
    port: u16,
    /// Address to bind.
    #[arg(long, env = "PROLOGUE_HOST", default_value = "0.0.0.0")]
    host: String,
    /// The built frontend (`npm run build` output). Default: `dist` next to
    /// this binary (the release bundle's layout), else `./dist`.
    #[arg(long, env = "PROLOGUE_DIST")]
    dist: Option<PathBuf>,
    /// Directory holding reviews.db (default: the app data directory the
    /// `prologue` CLI reads, so the CLI works without `--db`).
    #[arg(long, env = "PROLOGUE_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Webhook that "Send to Amp" POSTs open comment ids to. A credential:
    /// only the server reads it, never the browser.
    #[arg(long, env = "PROLOGUE_AMP_WEBHOOK_URL", hide_env_values = true)]
    amp_webhook_url: Option<String>,
    /// The Amp thread named in Send to Amp payloads.
    #[arg(long, env = "AMP_THREAD_ID", default_value = "")]
    amp_thread_id: String,
    /// Amp user id of the thread's owner, who may edit and delete anyone's
    /// comments (everyone else only their own).
    #[arg(long, env = "PROLOGUE_OWNER_ID")]
    owner_id: Option<String>,
    /// The owner's email, matched case-insensitively when no owner id is set.
    #[arg(long, env = "PROLOGUE_OWNER_EMAIL")]
    owner_email: Option<String>,
}

impl Cli {
    fn dist_dir(&self) -> PathBuf {
        if let Some(dist) = &self.dist {
            return dist.clone();
        }
        std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join("dist")))
            .filter(|dir| dir.join("index.html").is_file())
            .unwrap_or_else(|| PathBuf::from("dist"))
    }
}

/// What the frontend learns from `/api/config`.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WebConfig {
    repo_path: String,
    base_ref: Option<String>,
    /// Whether Send to Amp is configured; the URL itself stays here.
    amp_webhook: bool,
}

#[derive(Clone)]
struct ServerEvent {
    name: &'static str,
    payload: Value,
}

struct AppState {
    config: WebConfig,
    db: Arc<Db>,
    guides: GuideRuntime,
    repo_watcher: RepoWatcher,
    /// The repo the watcher follows — told to reconnecting clients so they
    /// refresh after missing events.
    watched: Arc<Mutex<Option<String>>>,
    events: broadcast::Sender<ServerEvent>,
    amp: Option<amp::AmpSink>,
    owner: Owner,
    /// The built frontend; index.html is served with the boot state added.
    dist: PathBuf,
}

impl AppState {
    fn emit(&self, name: &'static str, payload: Value) {
        // No subscribers is fine: nobody is looking.
        let _ = self.events.send(ServerEvent { name, payload });
    }
}

/// The Amp user behind a request, from the identity headers the Amp portal
/// proxy adds for the thread's owner and its multiplayer contributors.
/// Requests without them (the CLI's machine, in-orb tools) have none and
/// keep the plain "reviewer" authorship.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AmpUser {
    id: String,
    display_name: String,
    email: Option<String>,
    /// The thread's owner (see [`Owner`]): may edit and delete any comment.
    is_owner: bool,
    /// An owner is configured, so comments with no stored author are the
    /// owner's (see [`check_author`]); the UI hides their Edit and Delete.
    owner_configured: bool,
}

/// Who owns the Amp thread, as the plugin tells the server (the portal's
/// headers do not say). Its id wins; without one, its email matches
/// case-insensitively. Neither set: nobody is the owner (today's rules).
#[derive(Default, Clone)]
struct Owner {
    id: Option<String>,
    email: Option<String>,
}

impl Owner {
    fn new(id: Option<String>, email: Option<String>) -> Owner {
        let set = |v: Option<String>| v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        Owner { id: set(id), email: set(email) }
    }

    fn is(&self, id: &str, email: Option<&str>) -> bool {
        match (&self.id, &self.email) {
            (Some(owner), _) => owner == id,
            (None, Some(owner)) => email.is_some_and(|e| e.eq_ignore_ascii_case(owner)),
            (None, None) => false,
        }
    }

    fn is_configured(&self) -> bool {
        self.id.is_some() || self.email.is_some()
    }
}

impl AmpUser {
    fn from_headers(headers: &HeaderMap, owner: &Owner) -> Option<AmpUser> {
        let header = |name: &str| {
            let raw = headers.get(name)?.to_str().ok()?.trim();
            // Non-ASCII values arrive percent-encoded as UTF-8
            // (decodeURIComponent semantics); visible ASCII arrives as is,
            // so a value that does not decode is taken literally.
            let value = percent_encoding::percent_decode_str(raw)
                .decode_utf8()
                .map_or_else(|_| raw.to_owned(), |v| v.into_owned());
            (!value.is_empty()).then_some(value)
        };
        let id = header("x-amp-user-id")?;
        // Comment authors are printable and at most 64 characters (the
        // core's rule); a display name is shown, so fit it rather than
        // refuse the comment.
        let email = header("x-amp-user-email");
        // Without a display name, the email's local part reads better than
        // an opaque user id.
        let display_name: String = header("x-amp-user-display-name")
            .or_else(|| {
                let local = email.as_deref()?.split('@').next()?.trim();
                (!local.is_empty()).then(|| local.to_owned())
            })
            .unwrap_or_else(|| "Amp user".to_owned())
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        let is_owner = owner.is(&id, email.as_deref());
        Some(AmpUser { id, display_name, email, is_owner, owner_configured: owner.is_configured() })
    }
}

/// A command's JSON args object, looked up by the camelCase key the
/// frontend sends (Tauri converts those to snake_case params; we read them
/// as sent). A missing key reads as null, so `Option` args may be omitted.
struct Args<'a> {
    command: &'a str,
    value: Value,
}

impl Args<'_> {
    fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T, String> {
        serde_json::from_value(self.value.get(key).cloned().unwrap_or(Value::Null))
            .map_err(|e| format!("Invalid argument `{key}` for {}: {e}", self.command))
    }

    fn spec(&self) -> Result<DiffSpec, String> {
        Ok(DiffSpec {
            repo_path: self.get("repoPath")?,
            base: self.get("base")?,
            head: self.get("head")?,
            mode: self.get("mode")?,
        })
    }
}

fn ok<T: Serialize>(value: T) -> Result<Value, Failure> {
    serde_json::to_value(value).map_err(|e| Failure::Command(e.to_string()))
}

/// Why a command could not run, mapped to an HTTP status.
enum Failure {
    Unknown(String),
    Command(String),
}

impl From<String> for Failure {
    fn from(e: String) -> Self {
        Failure::Command(e)
    }
}

impl From<&str> for Failure {
    fn from(e: &str) -> Self {
        Failure::Command(e.to_owned())
    }
}

/// Whose reviewed marks a request reads and writes: the Amp user's own,
/// or the local reviewer's (the app's) without an identity.
fn reviewer(user: Option<&AmpUser>) -> &str {
    user.map_or(review::LOCAL_REVIEWER, |u| u.id.as_str())
}

/// Agent comments: the CLI writes them with no identity as "agent" (its
/// default) or "amp". Anyone may edit or delete them.
fn is_agent_author(author: &str) -> bool {
    author.eq_ignore_ascii_case("agent") || author.eq_ignore_ascii_case("amp")
}

/// A comment written by an Amp user is theirs alone to edit or delete —
/// and the thread owner's, who may change any comment. Comments with no
/// stored identity (the app's, older rows, web writes without headers) are
/// the owner's when one is configured; with none, anyone's, as before.
/// Agent comments stay editable by anyone. Returns the comment.
fn check_author(
    conn: &prologue_core::rusqlite::Connection,
    comment_id: i64,
    user: Option<&AmpUser>,
    owner: &Owner,
    action: &str,
) -> Result<review::Comment, String> {
    let comment = review::get_comment(conn, comment_id)?;
    match &comment.author_id {
        _ if user.is_some_and(|u| u.is_owner) => Ok(comment),
        Some(author) if user.map(|u| &u.id) != Some(author) => Err(format!(
            "Only {} can {action} C{comment_id} — it is their comment. Reply instead.",
            comment.author
        )),
        None if owner.is_configured() && !is_agent_author(&comment.author) => Err(format!(
            "Only the thread owner can {action} C{comment_id}. Reply instead."
        )),
        _ => Ok(comment),
    }
}

/// Commands whose success changes what other viewers' comment lists show.
/// The database watcher cannot report these: its data_version guard drops
/// this server's own writes, so they are broadcast from here. Reviewed
/// marks and re-anchoring are left out on purpose: marks are a reviewer's
/// own progress, and every client re-anchors on each refetch, so emitting
/// for it would loop.
const SHARED_WRITES: &[&str] = &[
    "create_comment",
    "update_comment",
    "delete_comment",
    "update_comment_state",
    "send_to_amp",
    "confirm_amp_send",
];

/// Run a command and, when it changed shared review state, tell every open
/// tab (including the caller's, whose refetch is idempotent).
fn dispatch(
    state: &AppState,
    command: &str,
    value: Value,
    user: Option<&AmpUser>,
) -> Result<Value, Failure> {
    let result = run_command(state, command, value, user);
    if result.is_ok() && SHARED_WRITES.contains(&command) {
        state.emit(events::COMMENTS_CHANGED, Value::Null);
    }
    result
}

/// The Tauri command table (src-tauri/src/lib.rs), over HTTP. Blocking —
/// git and SQLite work — so the handler runs it on a blocking thread.
fn run_command(
    state: &AppState,
    command: &str,
    value: Value,
    user: Option<&AmpUser>,
) -> Result<Value, Failure> {
    let a = Args { command, value };
    match command {
        "open_repo" => ok(repo::open_repo(a.get("path")?)?),
        "list_branches" => {
            let repo_path: String = a.get("repoPath")?;
            let mut list = repo::list_branches(repo_path.clone())?;
            // The configured base replaces auto-detection for the served
            // repo; a branch's stored review base still wins (frontend).
            if let Some(base) = &state.config.base_ref {
                if repo_path == state.config.repo_path {
                    list.default_base = base.clone();
                }
            }
            ok(list)
        }
        "get_diff_summary" => ok(diff::get_diff_summary(
            &a.spec()?,
            a.get("ignoreWhitespace")?,
        )?),
        "get_file_diff" => {
            let path: String = a.get("path")?;
            ok(diff::get_file_diff(
                &a.spec()?,
                a.get("ignoreWhitespace")?,
                &path,
            )?)
        }
        "get_context_lines" => ok(diff::get_context_lines(
            a.get("repoPath")?,
            a.get("head")?,
            a.get::<DiffMode>("mode")?,
            a.get("path")?,
            a.get("start")?,
            a.get("end")?,
        )?),
        "get_file_content" => ok(diff::get_file_content(
            a.get("repoPath")?,
            a.get("head")?,
            a.get::<DiffMode>("mode")?,
            a.get("path")?,
        )?),
        "open_review" => {
            let repo_path: String = a.get("repoPath")?;
            let branch: String = a.get("branch")?;
            let base_ref: String = a.get("baseRef")?;
            let mode: DiffMode = a.get("mode")?;
            let repo = open_git_repo(&repo_path)?;
            let conn = state.db.lock()?;
            ok(review::open_review_checked_impl(
                &conn, &repo, &repo_path, &branch, &base_ref, mode,
            )?)
        }
        "load_review" => ok(boot::load_review(
            state,
            &a.get::<String>("repoPath")?,
            &a.get::<String>("branch")?,
            &a.get::<String>("base")?,
            a.get("mode")?,
            a.get("ignoreWhitespace")?,
            user,
        )?),
        // The comments-changed refetch in one round trip: re-anchor, then
        // read the comments as moved.
        "reload_comments" => {
            let spec = a.spec()?;
            let review_id: i64 = a.get("reviewId")?;
            let conn = state.db.lock()?;
            let anchors = review::reanchor_comments_impl(&conn, &spec, review_id, true)?;
            ok(json!({
                "anchors": anchors,
                "comments": review::list_comments_impl(&conn, review_id)?,
            }))
        }
        "find_active_review" => {
            let repo_path: String = a.get("repoPath")?;
            let branch: String = a.get("branch")?;
            let conn = state.db.lock()?;
            ok(review::find_active_review_impl(&conn, &repo_path, &branch)?)
        }
        "list_reviewed_files" => {
            let conn = state.db.lock()?;
            ok(review::list_reviewed_files_impl(&conn, a.get("reviewId")?, reviewer(user))?)
        }
        "mark_file_reviewed" => {
            let file_path: String = a.get("filePath")?;
            let fingerprint: String = a.get("fingerprint")?;
            let conn = state.db.lock()?;
            ok(review::mark_file_reviewed_impl(
                &conn,
                a.get("reviewId")?,
                reviewer(user),
                &file_path,
                &fingerprint,
            )?)
        }
        "unmark_file_reviewed" => {
            let file_path: String = a.get("filePath")?;
            let conn = state.db.lock()?;
            ok(review::unmark_file_reviewed_impl(
                &conn,
                a.get("reviewId")?,
                reviewer(user),
                &file_path,
            )?)
        }
        "list_comments" => {
            let conn = state.db.lock()?;
            ok(review::list_comments_impl(&conn, a.get("reviewId")?)?)
        }
        "create_comment" => {
            let mut comment: NewComment = a.get("comment")?;
            if let Some(user) = user {
                comment.author = Some(user.display_name.clone());
                comment.author_id = Some(user.id.clone());
            }
            let spec = a.spec()?;
            let conn = state.db.lock()?;
            ok(review::create_comment_impl(&conn, &spec, comment)?)
        }
        "update_comment" => {
            let body: String = a.get("body")?;
            let expected: Option<String> = a.get("expectedUpdatedAt")?;
            let comment_id: i64 = a.get("commentId")?;
            let conn = state.db.lock()?;
            let current = check_author(&conn, comment_id, user, &state.owner, "edit")?;
            // Someone other than the author (the owner, or anyone on an
            // agent's comment) is named next to it; the author's own edit
            // clears that.
            let editor = user
                .filter(|u| current.author_id.as_deref() != Some(u.id.as_str()))
                .map(|u| u.display_name.as_str());
            ok(review::update_comment_impl(
                &conn,
                comment_id,
                &body,
                expected.as_deref(),
                editor,
            )?)
        }
        "delete_comment" => {
            let comment_id: i64 = a.get("commentId")?;
            let conn = state.db.lock()?;
            check_author(&conn, comment_id, user, &state.owner, "delete")?;
            ok(review::delete_comment_impl(&conn, comment_id)?)
        }
        "update_comment_state" => {
            let comment_state: CommentState = a.get("state")?;
            let conn = state.db.lock()?;
            ok(review::update_comment_state_impl(
                &conn,
                a.get("commentId")?,
                comment_state,
            )?)
        }
        "reanchor_comments" => {
            let spec = a.spec()?;
            let conn = state.db.lock()?;
            ok(review::reanchor_comments_impl(
                &conn,
                &spec,
                a.get("reviewId")?,
                true,
            )?)
        }
        "archive_stale_reviews" => {
            let repo_path: String = a.get("repoPath")?;
            let repo = open_git_repo(&repo_path)?;
            let conn = state.db.lock()?;
            ok(review::archive_stale_reviews_impl(
                &conn, &repo, &repo_path,
            )?)
        }
        "list_archived_reviews" => {
            let repo_path: String = a.get("repoPath")?;
            let conn = state.db.lock()?;
            ok(review::list_archived_reviews_impl(&conn, &repo_path)?)
        }
        "export_review" => {
            let spec = a.spec()?;
            let format: ExportFormat = a.get("format")?;
            let conn = state.db.lock()?;
            ok(export::export_review_impl(
                &conn,
                &spec,
                a.get("reviewId")?,
                format,
                true,
            )?)
        }
        "find_guide" => {
            let base: String = a.get("base")?;
            let head: String = a.get("head")?;
            let conn = state.db.lock()?;
            ok(guide::find_guide_impl(
                &conn,
                a.get("reviewId")?,
                &base,
                &head,
                a.get::<DiffMode>("mode")?,
            )?)
        }
        "generate_guide" => ok(guide_engine::generate_guide(
            &state.db,
            &state.guides,
            a.get("repoPath")?,
            a.get("base")?,
            a.get("head")?,
            a.get("mode")?,
            a.get("reviewId")?,
        )?),
        "cancel_guide" => ok(guide_engine::cancel_guide(&state.guides)),
        "guide_cli_status" => ok(guide_engine::guide_cli_status()),
        "start_watching" => {
            let repo_path: String = a.get("repoPath")?;
            let events = state.events.clone();
            state.repo_watcher.start(repo_path.clone(), move |path| {
                let _ = events.send(ServerEvent {
                    name: events::REPO_CHANGED,
                    payload: json!(path),
                });
            })?;
            *state.watched.lock().map_err(|_| "Watcher state poisoned")? = Some(repo_path);
            ok(())
        }
        "stop_watching" => {
            // Every tab shares this server's one watch; a tab going back
            // to the welcome page must not stop another tab's refreshes,
            // so the watch stays until a new repo re-targets it.
            ok(())
        }
        // Native-menu plumbing: there is no menu in a browser.
        "set_repo_menu_enabled" | "set_hide_resolved_checked" => ok(()),
        "amp_sends" => {
            let review_id: i64 = a.get("reviewId")?;
            let conn = state.db.lock()?;
            let repo = review::get_review(&conn, review_id)?.repo_path;
            let sends = prologue_core::amp_sends::list_sends(&conn, review_id)?;
            // A failed send carries the message to paste into the thread by
            // hand, so every viewer can finish it.
            let mut listed = serde_json::to_value(&sends).map_err(|e| e.to_string())?;
            for (send, value) in sends.iter().zip(listed.as_array_mut().into_iter().flatten()) {
                if send.state == prologue_core::amp_sends::SendState::Failed {
                    value["handoff"] = json!(amp::handoff_message(&repo, &send.comment_ids));
                }
            }
            ok(listed)
        }
        // "I pasted it": the manual fallback reached the thread, so the
        // send counts as confirmed, exactly as `prologue confirm-send`.
        "confirm_amp_send" => {
            let send_id: String = a.get("sendId")?;
            let conn = state.db.lock()?;
            match prologue_core::amp_sends::confirm_send(&conn, &send_id)? {
                prologue_core::amp_sends::Confirmed::Unknown => {
                    Err(Failure::from(format!("No Send to Amp has id {send_id}")))
                }
                _ => ok(()),
            }
        }
        "send_to_amp" => {
            let amp = state.amp.as_ref().ok_or(
                "Send to Amp is not configured (set PROLOGUE_AMP_WEBHOOK_URL on the server)",
            )?;
            let repo: String = a.get("repoPath")?;
            let base: String = a.get("base")?;
            let notify = || state.emit(events::COMMENTS_CHANGED, Value::Null);
            // Every viewer sees the send go to `sending`, and back to unsent
            // if the webhook refuses it (dispatch announces a success).
            amp.send(&state.db, &repo, &base, a.get("reviewId")?, notify)
                .inspect_err(|_| notify())
                .map(ok)?
        }
        "install_cli" => Err(Failure::Command(
            "Installing the command line tool is only available in the Mac app. \
                 In web mode, build it with `cargo build --release -p prologue` and put \
                 target/release/prologue on PATH."
                .to_owned(),
        )),
        _ => Err(Failure::Unknown(format!("Unknown command: {command}"))),
    }
}

async fn invoke(
    State(state): State<Arc<AppState>>,
    UrlPath(command): UrlPath<String>,
    headers: HeaderMap,
    Json(args): Json<Value>,
) -> Response {
    let user = AmpUser::from_headers(&headers, &state.owner);
    let result =
        tokio::task::spawn_blocking(move || dispatch(&state, &command, args, user.as_ref())).await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(Failure::Command(e))) => (StatusCode::BAD_REQUEST, Json(e)).into_response(),
        Ok(Err(Failure::Unknown(e))) => (StatusCode::NOT_FOUND, Json(e)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("Command failed: {e}")),
        )
            .into_response(),
    }
}

/// `/api/config`: the config plus what the UI would otherwise fetch one
/// call at a time before its first diff request (open_repo, list_branches,
/// find_active_review). Each of those is a full round trip through the
/// portal, so answering them here saves three on every page load.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigResponse {
    #[serde(flatten)]
    config: WebConfig,
    /// None when any lookup failed; the UI then makes the calls itself
    /// and shows their error.
    start: Option<WebStart>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebStart {
    repo: Value,
    branch_list: Value,
    active_review: Value,
}

fn web_start(state: &AppState) -> Result<WebStart, Failure> {
    let path = state.config.repo_path.clone();
    let repo = run_command(state, "open_repo", json!({ "path": path }), None)?;
    let branch_list = run_command(state, "list_branches", json!({ "repoPath": repo["path"] }), None)?;
    let active_review = run_command(
        state,
        "find_active_review",
        json!({ "repoPath": repo["path"], "branch": branch_list["current"] }),
        None,
    )?;
    Ok(WebStart { repo, branch_list, active_review })
}

async fn config(State(state): State<Arc<AppState>>) -> Json<ConfigResponse> {
    let start = tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        move || web_start(&state).ok()
    })
    .await
    .ok()
    .flatten();
    Json(ConfigResponse { config: state.config.clone(), start })
}

/// Echoes the portal's identity headers so the UI can tell the viewer's own
/// comments from other people's; null outside a portal.
async fn me(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Json<Option<AmpUser>> {
    Json(AmpUser::from_headers(&headers, &state.owner))
}

async fn event_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let watching = state.watched.lock().ok().and_then(|w| w.clone());
    let hello = Event::default()
        .event("hello")
        .data(json!({ "watching": watching }).to_string());
    let live = BroadcastStream::new(state.events.subscribe()).filter_map(|item| {
        // A lagged receiver dropped events; the client recovers on the next
        // one (every event means "refetch", never a delta).
        item.ok()
            .map(|e| Ok(Event::default().event(e.name).data(e.payload.to_string())))
    });
    let stream = tokio_stream::once(Ok(hello)).chain(live);
    // A named `ping` on every idle interval, not a comment: EventSource
    // hides comments from page code, and the client's watchdog needs to see
    // the stream is alive. A proxy (the Amp portal) can keep the browser's
    // half open after this server restarts, so silence is the only signal
    // of a dead stream. The pings also keep idle proxies from closing it.
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .event(Event::default().event("ping").data("{}")),
    )
}

/// Vite content-hashes everything under /assets, so a name never changes
/// meaning: browsers may keep it for good instead of revalidating each load.
async fn immutable(mut res: Response) -> Response {
    if res.status() == StatusCode::OK {
        res.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    res
}

fn router(state: Arc<AppState>, dist: &std::path::Path) -> Router {
    let assets = Router::new()
        .fallback_service(ServeDir::new(dist.join("assets")))
        .layer(axum::middleware::map_response(immutable));
    // Other paths: files at the dist root, else the app (client routes).
    let frontend =
        ServeDir::new(dist).fallback(get(boot::index).with_state(Arc::clone(&state)));
    Router::new()
        .route("/api/invoke/{command}", post(invoke))
        .route("/api/events", get(event_stream))
        .route("/api/config", get(config))
        .route("/api/me", get(me))
        .route("/", get(boot::index))
        .route("/index.html", get(boot::index))
        .nest("/assets", assets)
        .fallback_service(frontend)
        // JSON and HTML get compressed here; the SSE stream is exempt by
        // default (buffering would hold events back). Brotli at quality 4
        // costs milliseconds even on the 600 KB highlighter chunk.
        .layer(CompressionLayer::new().quality(CompressionLevel::Precise(4)))
        .with_state(state)
}

fn build_state(cli: &Cli) -> Result<Arc<AppState>, String> {
    // Canonical, so the stored repo_path matches what the `prologue` CLI
    // resolves from a working directory inside the repo.
    let repo_path = std::fs::canonicalize(&cli.repo)
        .map_err(|e| format!("Cannot open {}: {e}", cli.repo.display()))?
        .to_string_lossy()
        .into_owned();
    let branches = repo::list_branches(repo_path.clone())?;
    if let Some(base) = &cli.base {
        if !branches.branches.contains(base) {
            return Err(format!(
                "Base ref `{base}` is not a branch of {repo_path} (have: {})",
                branches.branches.join(", ")
            ));
        }
    }

    let data_dir = match &cli.data_dir {
        Some(dir) => dir.clone(),
        None => db::default_data_dir()?,
    };
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Cannot create {}: {e}", data_dir.display()))?;
    let conn = db::open(&data_dir.join("reviews.db"))?;
    let db = Arc::new(Db(Mutex::new(conn)));

    let (events_tx, _) = broadcast::channel(64);
    let amp = cli.amp_webhook_url.as_ref().filter(|u| !u.is_empty()).map(|url| {
        if cli.amp_thread_id.is_empty() {
            eprintln!("prologue-server: warning: Send to Amp is on but AMP_THREAD_ID is unset");
        }
        amp::AmpSink::new(url.clone(), cli.amp_thread_id.clone())
    });
    // Comments sent before confirmations existed stay sent.
    amp::import_legacy_sent(&db, &data_dir.join("amp-sent.json"))?;
    let state = Arc::new(AppState {
        config: WebConfig {
            repo_path,
            base_ref: cli.base.clone(),
            amp_webhook: amp.is_some(),
        },
        db: Arc::clone(&db),
        guides: GuideRuntime::default(),
        repo_watcher: RepoWatcher::default(),
        watched: Arc::new(Mutex::new(None)),
        events: events_tx,
        amp,
        owner: Owner::new(cli.owner_id.clone(), cli.owner_email.clone()),
        dist: cli.dist_dir(),
    });

    // External writers (the prologue CLI) commit to reviews.db behind the
    // server's back; surface them as `comments-changed`, as the app does.
    let expired = Arc::clone(&state);
    amp::start_expiry(Arc::clone(&db), move || {
        expired.emit(events::COMMENTS_CHANGED, Value::Null)
    });
    let emitter = Arc::clone(&state);
    watcher::start_db_watching(
        &data_dir,
        move || watcher::data_version(&db),
        move || emitter.emit(events::COMMENTS_CHANGED, Value::Null),
    )?;
    eprintln!(
        "prologue-server: reviews database {}",
        data_dir.join("reviews.db").display()
    );
    Ok(state)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let state = match build_state(&cli) {
        Ok(state) => state,
        Err(e) => {
            eprintln!("prologue-server: {e}");
            std::process::exit(1);
        }
    };
    let dist = cli.dist_dir();
    if !dist.join("index.html").is_file() {
        eprintln!(
            "prologue-server: warning: no index.html in {} — run `npm run build` first",
            dist.display()
        );
    }
    let addr = format!("{}:{}", cli.host, cli.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("prologue-server: cannot listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "prologue-server: reviewing {} on http://{addr}",
        state.config.repo_path
    );
    // No graceful shutdown: open SSE streams would hold it forever, and
    // every write is already committed per request.
    if let Err(e) = axum::serve(listener, router(state, &dist)).await {
        eprintln!("prologue-server: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prologue_core::testutil::FixtureRepo;

    fn state_for(fixture: &FixtureRepo, dir: &tempfile::TempDir) -> AppState {
        let conn = db::open(&dir.path().join("reviews.db")).unwrap();
        AppState {
            config: WebConfig { repo_path: fixture.path(), base_ref: None, amp_webhook: false },
            db: Arc::new(Db(Mutex::new(conn))),
            guides: GuideRuntime::default(),
            repo_watcher: RepoWatcher::default(),
            watched: Arc::new(Mutex::new(None)),
            events: broadcast::channel(4).0,
            amp: None,
            owner: Owner::default(),
            dist: PathBuf::new(),
        }
    }

    fn call(state: &AppState, command: &str, args: Value) -> Result<Value, String> {
        call_as(state, None, command, args)
    }

    fn call_as(
        state: &AppState,
        user: Option<&AmpUser>,
        command: &str,
        args: Value,
    ) -> Result<Value, String> {
        dispatch(state, command, args, user).map_err(|e| match e {
            Failure::Unknown(e) => format!("unknown: {e}"),
            Failure::Command(e) => e,
        })
    }

    /// The commands read the exact camelCase args object src/ipc.ts sends
    /// to Tauri — a renamed key would break web mode only. Drives the
    /// review/comment round trip the UI makes, payloads copied from ipc.ts.
    #[test]
    fn commands_accept_the_frontend_args_shape() {
        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let state = state_for(&fixture, &dir);
        let repo_path = fixture.path();

        let opened = call(
            &state,
            "open_review",
            json!({ "repoPath": repo_path, "branch": "feature", "baseRef": "main", "mode": "committed" }),
        )
        .unwrap();
        let review_id = opened["review"]["id"].as_i64().unwrap();

        let created = call(
            &state,
            "create_comment",
            json!({
                "repoPath": repo_path, "base": "main", "head": "feature", "mode": "committed",
                "comment": { "reviewId": review_id, "level": "line", "filePath": "code.txt",
                             "side": "new", "startLine": 6, "endLine": 7, "body": "why beta?" },
            }),
        )
        .unwrap();
        assert_eq!(created["startLine"], 6);

        let listed = call(&state, "list_comments", json!({ "reviewId": review_id })).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["body"], "why beta?");
    }

    /// Through a portal, comments and replies are stored under the Amp
    /// user's id and display name — whatever author the payload claims.
    /// Without the headers they stay the reviewer's.
    #[test]
    fn portal_identity_headers_author_comments() {
        let mut headers = HeaderMap::new();
        headers.insert("x-amp-user-id", "user_ada".parse().unwrap());
        // "Ada Lovelacé" as Amp sends non-ASCII: percent-encoded UTF-8.
        headers.insert("x-amp-user-display-name", "Ada%20Lovelac%C3%A9".parse().unwrap());
        headers.insert("x-amp-user-email", "ada@example.com".parse().unwrap());
        let ada = AmpUser::from_headers(&headers, &Owner::default()).unwrap();
        assert_eq!(ada.display_name, "Ada Lovelacé");
        assert_eq!(AmpUser::from_headers(&HeaderMap::new(), &Owner::default()), None);
        // Visible ASCII arrives unencoded; a stray % stays literal.
        let mut literal = HeaderMap::new();
        literal.insert("x-amp-user-id", "user_b".parse().unwrap());
        literal.insert("x-amp-user-display-name", "100% Bob".parse().unwrap());
        assert_eq!(AmpUser::from_headers(&literal, &Owner::default()).unwrap().display_name, "100% Bob");

        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let state = state_for(&fixture, &dir);
        let repo_path = fixture.path();
        let opened = call(
            &state,
            "open_review",
            json!({ "repoPath": repo_path, "branch": "feature", "baseRef": "main", "mode": "committed" }),
        )
        .unwrap();
        let review_id = opened["review"]["id"].as_i64().unwrap();
        let create = |user: Option<&AmpUser>, comment: Value| {
            call_as(
                &state,
                user,
                "create_comment",
                json!({ "repoPath": repo_path, "base": "main", "head": "feature",
                        "mode": "committed", "comment": comment }),
            )
            .unwrap()
        };

        let root = create(
            Some(&ada),
            json!({ "reviewId": review_id, "level": "review", "body": "hi",
                    "author": "someone else", "authorId": "user_spoof" }),
        );
        assert_eq!((&root["author"], &root["authorId"]), (&json!("Ada Lovelacé"), &json!("user_ada")));
        let reply = create(
            Some(&ada),
            json!({ "reviewId": review_id, "level": "review", "parentId": root["id"], "body": "ok" }),
        );
        assert_eq!(reply["authorId"], "user_ada");
        let plain = create(None, json!({ "reviewId": review_id, "level": "review", "body": "cli" }));
        assert_eq!((&plain["author"], &plain["authorId"]), (&json!("reviewer"), &Value::Null));
    }

    /// Another viewer's tab learns of a comment written through this server
    /// (the DB watcher filters these out), while the re-anchor pass every
    /// client runs on refetch stays silent so refetches cannot loop.
    #[test]
    fn web_comment_writes_notify_other_viewers() {
        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let state = state_for(&fixture, &dir);
        let mut other_tab = state.events.subscribe();
        let repo_path = fixture.path();
        let spec = json!({ "repoPath": repo_path, "base": "main", "head": "feature", "mode": "committed" });
        let opened = call(
            &state,
            "open_review",
            json!({ "repoPath": repo_path, "branch": "feature", "baseRef": "main", "mode": "committed" }),
        )
        .unwrap();
        let review_id = opened["review"]["id"].as_i64().unwrap();
        assert!(other_tab.try_recv().is_err(), "opening a review is not a comment change");

        let mut args = spec.clone();
        args["comment"] = json!({ "reviewId": review_id, "level": "review", "body": "hi" });
        let id = call(&state, "create_comment", args).unwrap()["id"].clone();
        assert_eq!(other_tab.try_recv().unwrap().name, events::COMMENTS_CHANGED);
        call(&state, "update_comment", json!({ "commentId": id, "body": "edited" })).unwrap();
        call(&state, "update_comment_state", json!({ "commentId": id, "state": "resolved" })).unwrap();
        call(&state, "delete_comment", json!({ "commentId": id })).unwrap();
        for _ in 0..3 {
            assert_eq!(other_tab.try_recv().unwrap().name, events::COMMENTS_CHANGED);
        }

        let mut args = spec;
        args["reviewId"] = json!(review_id);
        call(&state, "reanchor_comments", args).unwrap();
        // A failed write changed nothing, so it tells nobody.
        call(&state, "delete_comment", json!({ "commentId": 999 })).unwrap_err();
        assert!(other_tab.try_recv().is_err());
    }

    /// Through the portal, only a comment's Amp author may edit or delete
    /// it (the agent's stay open to all), and each viewer's Viewed marks
    /// are their own.
    #[test]
    fn edits_are_the_authors_and_marks_are_per_viewer() {
        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let state = state_for(&fixture, &dir);
        let user = |id: &str| AmpUser { id: id.to_owned(), display_name: id.to_owned(), email: None, is_owner: false, owner_configured: false };
        let (ada, grace) = (user("ada"), user("grace"));
        let repo_path = fixture.path();
        let opened = call(
            &state,
            "open_review",
            json!({ "repoPath": repo_path, "branch": "feature", "baseRef": "main", "mode": "committed" }),
        )
        .unwrap();
        let review_id = opened["review"]["id"].as_i64().unwrap();
        let create = |who: Option<&AmpUser>| {
            call_as(
                &state,
                who,
                "create_comment",
                json!({ "repoPath": repo_path, "base": "main", "head": "feature", "mode": "committed",
                        "comment": { "reviewId": review_id, "level": "review", "body": "x" } }),
            )
            .unwrap()["id"]
                .clone()
        };
        let adas = create(Some(&ada));
        let agents = create(None);

        let err = call_as(&state, Some(&grace), "update_comment", json!({ "commentId": adas, "body": "y" }))
            .unwrap_err();
        assert!(err.contains("Only ada"), "{err}");
        call_as(&state, Some(&grace), "delete_comment", json!({ "commentId": adas })).unwrap_err();
        call_as(&state, None, "delete_comment", json!({ "commentId": adas })).unwrap_err();
        call_as(&state, Some(&ada), "update_comment", json!({ "commentId": adas, "body": "y" })).unwrap();
        call_as(&state, Some(&grace), "update_comment", json!({ "commentId": agents, "body": "z" })).unwrap();
        call_as(&state, Some(&grace), "delete_comment", json!({ "commentId": agents })).unwrap();

        call_as(
            &state,
            Some(&ada),
            "mark_file_reviewed",
            json!({ "reviewId": review_id, "filePath": "code.txt", "fingerprint": "f" }),
        )
        .unwrap();
        let marks = |who: Option<&AmpUser>| {
            call_as(&state, who, "list_reviewed_files", json!({ "reviewId": review_id }))
                .unwrap()
                .as_array()
                .unwrap()
                .len()
        };
        assert_eq!((marks(Some(&ada)), marks(Some(&grace)), marks(None)), (1, 0, 0));
    }

    /// The owner (by id, else by email) may edit and delete anyone's
    /// comment, keeping its author and naming the editor; others still only
    /// their own. Without owner settings nobody is the owner.
    #[test]
    fn the_thread_owner_may_change_any_comment() {
        let headers = |id: &str, email: &str| {
            let mut h = HeaderMap::new();
            h.insert("x-amp-user-id", id.parse().unwrap());
            h.insert("x-amp-user-display-name", id.parse().unwrap());
            h.insert("x-amp-user-email", email.parse().unwrap());
            h
        };
        let by_id = Owner::new(Some("sky".into()), Some("other@x.com".into()));
        let by_email = Owner::new(None, Some("Sky@Example.com".into()));
        let user = |o: &Owner, id, email| AmpUser::from_headers(&headers(id, email), o).unwrap();
        assert!(user(&by_id, "sky", "a@b.c").is_owner);
        assert!(!user(&by_id, "grace", "other@x.com").is_owner, "the id wins over the email");
        assert!(user(&by_email, "u1", "sky@EXAMPLE.com").is_owner);
        assert!(!user(&by_email, "u2", "grace@example.com").is_owner);
        assert!(!user(&Owner::new(Some(" ".into()), None), "u3", "x").is_owner);

        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let state = state_for(&fixture, &dir);
        let sky = user(&by_id, "sky", "s@x");
        let grace = user(&by_id, "grace", "g@x");
        let repo_path = fixture.path();
        let review_id = call(
            &state,
            "open_review",
            json!({ "repoPath": repo_path, "branch": "feature", "baseRef": "main", "mode": "committed" }),
        )
        .unwrap()["review"]["id"]
            .as_i64()
            .unwrap();
        let create = |who: &AmpUser| {
            call_as(&state, Some(who), "create_comment",
                json!({ "repoPath": repo_path, "base": "main", "head": "feature", "mode": "committed",
                        "comment": { "reviewId": review_id, "level": "review", "body": "x" } }))
                .unwrap()["id"].clone()
        };
        let graces = create(&grace);
        let skys = create(&sky);

        let edited =
            call_as(&state, Some(&sky), "update_comment", json!({ "commentId": graces, "body": "fixed" })).unwrap();
        assert_eq!((&edited["author"], &edited["editedBy"]), (&json!("grace"), &json!("sky")));
        let own = call_as(&state, Some(&grace), "update_comment", json!({ "commentId": graces, "body": "mine" }))
            .unwrap();
        assert_eq!(own["editedBy"], Value::Null, "the author's own edit clears it");
        call_as(&state, Some(&grace), "update_comment", json!({ "commentId": skys, "body": "y" })).unwrap_err();
        call_as(&state, Some(&grace), "delete_comment", json!({ "commentId": skys })).unwrap_err();
        call_as(&state, Some(&sky), "delete_comment", json!({ "commentId": graces })).unwrap();
    }

    /// With an owner configured, comments with no stored author (the app's,
    /// older rows) are the owner's: others may only reply. Agent comments
    /// stay open to all, and without an owner nothing changes.
    #[test]
    fn unattributed_comments_belong_to_the_owner() {
        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_for(&fixture, &dir);
        let owner = Owner::new(Some("sky".into()), None);
        let user = |id: &str| {
            let mut h = HeaderMap::new();
            h.insert("x-amp-user-id", id.parse().unwrap());
            AmpUser::from_headers(&h, &owner).unwrap()
        };
        let (sky, grace) = (user("sky"), user("grace"));
        let repo_path = fixture.path();
        let review_id = call(
            &state,
            "open_review",
            json!({ "repoPath": repo_path, "branch": "feature", "baseRef": "main", "mode": "committed" }),
        )
        .unwrap()["review"]["id"]
            .as_i64()
            .unwrap();
        // Written as the CLI and the app write: no identity.
        let unattributed = |state: &AppState, author: &str| {
            call(state, "create_comment",
                json!({ "repoPath": repo_path, "base": "main", "head": "feature", "mode": "committed",
                        "comment": { "reviewId": review_id, "level": "review", "body": "x", "author": author } }))
                .unwrap()["id"].clone()
        };
        let (apps, agents, amps) =
            (unattributed(&state, "reviewer"), unattributed(&state, "agent"), unattributed(&state, "Amp"));

        // No owner configured: anyone, as before.
        call_as(&state, Some(&grace), "update_comment", json!({ "commentId": apps, "body": "a" })).unwrap();

        state.owner = owner.clone();
        let err = call_as(&state, Some(&grace), "update_comment", json!({ "commentId": apps, "body": "b" }))
            .unwrap_err();
        assert!(err.contains("Only the thread owner"), "{err}");
        call_as(&state, Some(&grace), "delete_comment", json!({ "commentId": apps })).unwrap_err();
        call_as(&state, None, "delete_comment", json!({ "commentId": apps })).unwrap_err();
        call_as(&state, Some(&grace), "update_comment", json!({ "commentId": agents, "body": "c" })).unwrap();
        call_as(&state, Some(&grace), "delete_comment", json!({ "commentId": amps })).unwrap();
        call_as(&state, Some(&sky), "update_comment", json!({ "commentId": apps, "body": "d" })).unwrap();
        call_as(&state, Some(&sky), "delete_comment", json!({ "commentId": apps })).unwrap();
        assert!(grace.owner_configured && !grace.is_owner && sky.is_owner);
    }

    /// Without a display-name header, a viewer is named by their email's
    /// local part, else generically — never by their opaque user id.
    #[test]
    fn a_missing_display_name_falls_back_to_the_email() {
        let mut h = HeaderMap::new();
        h.insert("x-amp-user-id", "user_01abc".parse().unwrap());
        assert_eq!(AmpUser::from_headers(&h, &Owner::default()).unwrap().display_name, "Amp user");
        h.insert("x-amp-user-email", "skyler@example.com".parse().unwrap());
        assert_eq!(AmpUser::from_headers(&h, &Owner::default()).unwrap().display_name, "skyler");
    }

    /// load_review answers what the separate calls would, in one: the
    /// opened review, its comments with anchors, this viewer's marks, and
    /// hunks identical to get_file_diff's for the files that open expanded
    /// (not ones this viewer already marked reviewed).
    #[test]
    fn load_review_matches_the_separate_calls() {
        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let state = state_for(&fixture, &dir);
        let ada = AmpUser { id: "ada".into(), display_name: "Ada".into(), email: None, is_owner: false, owner_configured: false };
        let repo_path = fixture.path();
        let spec = json!({ "repoPath": repo_path, "base": "main", "head": "feature", "mode": "committed" });
        let load = |who: Option<&AmpUser>| {
            let mut args = spec.clone();
            args["branch"] = json!("feature");
            args["ignoreWhitespace"] = json!(false);
            call_as(&state, who, "load_review", args).unwrap()
        };

        let first = load(Some(&ada));
        let review_id = first["review"]["id"].as_i64().unwrap();
        assert_eq!(first["review"]["status"], "active");
        let mut args = spec.clone();
        args["comment"] = json!({ "reviewId": review_id, "level": "line", "filePath": "code.txt",
                                  "side": "new", "startLine": 6, "endLine": 6, "body": "why?" });
        call_as(&state, Some(&ada), "create_comment", args).unwrap();
        let code = first["summary"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == "code.txt")
            .unwrap()
            .clone();
        call_as(
            &state,
            Some(&ada),
            "mark_file_reviewed",
            json!({ "reviewId": review_id, "filePath": "code.txt", "fingerprint": code["fingerprint"] }),
        )
        .unwrap();

        let prefetched = |loaded: &Value| -> Vec<String> {
            loaded["fileDiffs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d["path"].as_str().unwrap().to_owned())
                .collect()
        };
        let for_ada = load(Some(&ada));
        assert_eq!(for_ada["review"]["id"], review_id, "resumes, not a new review");
        assert_eq!(for_ada["comments"][0]["body"], "why?");
        assert_eq!(for_ada["anchors"][0]["status"], "anchored");
        assert_eq!(for_ada["reviewedFiles"].as_array().unwrap().len(), 1);
        assert_eq!(prefetched(&for_ada), vec!["d.txt"], "Ada's reviewed file stays collapsed");

        let for_grace = load(None);
        assert_eq!(for_grace["reviewedFiles"].as_array().unwrap().len(), 0);
        assert_eq!(prefetched(&for_grace), vec!["code.txt", "d.txt"]);
        let mut args = spec.clone();
        args["path"] = json!("code.txt");
        args["ignoreWhitespace"] = json!(false);
        assert_eq!(for_grace["fileDiffs"][0], call(&state, "get_file_diff", args).unwrap());

        let mut args = spec;
        args["reviewId"] = json!(review_id);
        let reloaded = call(&state, "reload_comments", args).unwrap();
        assert_eq!((&reloaded["anchors"], &reloaded["comments"]), (&for_ada["anchors"], &for_ada["comments"]));
    }

    /// Until the viewer picks a mode (cookie `auto`, or none), the inlined
    /// review shows uncommitted work when the checkout has some; a picked
    /// mode always wins.
    #[test]
    fn the_inlined_review_defaults_to_the_working_tree_when_it_is_dirty() {
        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let state = state_for(&fixture, &dir);
        let inlined_mode = |cookie: Option<&str>| {
            let mut headers = HeaderMap::new();
            if let Some(c) = cookie {
                headers.insert("cookie", c.parse().unwrap());
            }
            boot::boot_state(&state, None, &headers)["review"]["mode"].clone()
        };
        assert_eq!(inlined_mode(None), "committed");
        fixture.write("agent.txt", "uncommitted\n");
        assert_eq!(inlined_mode(None), "all");
        assert_eq!(inlined_mode(Some("prologue-view=auto.0")), "all");
        assert_eq!(inlined_mode(Some("x=1; prologue-view=committed.0")), "committed");
    }

    /// The page inlines user-written comment text in a <script>; a body
    /// that says `</script>` must not end the element.
    #[test]
    fn inlined_boot_state_cannot_close_its_script() {
        let body = json!({ "body": "</script><script>alert(1)</script> & <!--" });
        let inlined = boot::script_json(&body);
        assert!(!inlined.contains('<') && !inlined.contains('>'), "{inlined}");
        assert_eq!(serde_json::from_str::<Value>(&inlined).unwrap(), body);
    }

    /// Hashed assets are cached for good, but only when they exist; the
    /// page itself is never cached (it carries live review state), and a
    /// missing file is a 404, not the app.
    #[tokio::test]
    async fn assets_are_immutable_and_the_page_is_not_cached() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt as _;
        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let dist = dir.path().join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::write(dist.join("index.html"), "<html><head></head><body></body></html>").unwrap();
        std::fs::write(dist.join("assets/index-abc123.js"), "export {}").unwrap();
        let mut state = state_for(&fixture, &dir);
        state.dist = dist.clone();
        let app = router(Arc::new(state), &dist);
        let get = |path: &str| {
            let app = app.clone();
            let req = Request::get(path).body(Body::empty()).unwrap();
            async move { app.oneshot(req).await.unwrap() }
        };
        let cache = |res: &Response| {
            res.headers()
                .get("cache-control")
                .map(|v| v.to_str().unwrap().to_owned())
        };

        let asset = get("/assets/index-abc123.js").await;
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(cache(&asset).as_deref(), Some("public, max-age=31536000, immutable"));
        let missing = get("/assets/index-gone.js").await;
        assert_eq!((missing.status(), cache(&missing)), (StatusCode::NOT_FOUND, None));
        assert_eq!(get("/favicon.ico").await.status(), StatusCode::NOT_FOUND);
        for page in ["/", "/some/client/route"] {
            let res = get(page).await;
            assert_eq!(res.status(), StatusCode::OK, "{page}");
            assert_eq!(cache(&res).as_deref(), Some("no-cache"), "{page}");
            let html = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
            assert!(String::from_utf8_lossy(&html).contains(r#"id="prologue-boot""#), "{page}");
        }
    }

    /// /api/config answers the UI's startup calls the way the commands
    /// would: the configured base overrides auto-detection, and the
    /// current branch's active review comes back so its base is resumed.
    #[test]
    fn config_carries_the_startup_answers() {
        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_for(&fixture, &dir);
        state.config.base_ref = Some("feature".to_owned());
        let start = web_start(&state).ok().unwrap();
        assert_eq!(start.repo["path"], json!(fixture.path()));
        assert_eq!(start.branch_list["defaultBase"], "feature");
        assert_eq!(start.active_review, Value::Null);

        let branch = start.branch_list["current"].clone();
        call(
            &state,
            "open_review",
            json!({ "repoPath": fixture.path(), "branch": branch, "baseRef": "main", "mode": "committed" }),
        )
        .unwrap();
        let start = web_start(&state).ok().unwrap();
        assert_eq!(start.active_review["baseRef"], "main");
    }

    #[test]
    fn unknown_commands_and_bad_args_fail_distinctly() {
        let fixture = FixtureRepo::standard_review_fixture();
        let dir = tempfile::tempdir().unwrap();
        let state = state_for(&fixture, &dir);

        let err = call(&state, "drop_tables", json!({})).unwrap_err();
        assert!(err.starts_with("unknown:"), "{err}");
        // A missing required arg is a command error naming the key.
        let err = call(&state, "list_comments", json!({})).unwrap_err();
        assert!(err.contains("`reviewId`"), "{err}");
        // Native-only actions refuse with guidance rather than 404.
        let err = call(&state, "install_cli", json!({})).unwrap_err();
        assert!(err.contains("Mac app"), "{err}");
    }
}

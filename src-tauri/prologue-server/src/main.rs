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
//!
//! The command bodies are the same `prologue-core` / `prologue-runtime`
//! calls `src-tauri/src/commands.rs` makes; only the transport differs.

mod amp;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
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
use tower_http::services::{ServeDir, ServeFile};

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
}

impl AppState {
    fn emit(&self, name: &'static str, payload: Value) {
        // No subscribers is fine: nobody is looking.
        let _ = self.events.send(ServerEvent { name, payload });
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

/// The Tauri command table (src-tauri/src/lib.rs), over HTTP. Blocking —
/// git and SQLite work — so the handler runs it on a blocking thread.
fn dispatch(state: &AppState, command: &str, value: Value) -> Result<Value, Failure> {
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
        "find_active_review" => {
            let repo_path: String = a.get("repoPath")?;
            let branch: String = a.get("branch")?;
            let conn = state.db.lock()?;
            ok(review::find_active_review_impl(&conn, &repo_path, &branch)?)
        }
        "list_reviewed_files" => {
            let conn = state.db.lock()?;
            ok(review::list_reviewed_files_impl(&conn, a.get("reviewId")?)?)
        }
        "mark_file_reviewed" => {
            let file_path: String = a.get("filePath")?;
            let fingerprint: String = a.get("fingerprint")?;
            let conn = state.db.lock()?;
            ok(review::mark_file_reviewed_impl(
                &conn,
                a.get("reviewId")?,
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
                &file_path,
            )?)
        }
        "list_comments" => {
            let conn = state.db.lock()?;
            ok(review::list_comments_impl(&conn, a.get("reviewId")?)?)
        }
        "create_comment" => {
            let comment: NewComment = a.get("comment")?;
            let spec = a.spec()?;
            let conn = state.db.lock()?;
            ok(review::create_comment_impl(&conn, &spec, comment)?)
        }
        "update_comment" => {
            let body: String = a.get("body")?;
            let conn = state.db.lock()?;
            ok(review::update_comment_impl(
                &conn,
                a.get("commentId")?,
                &body,
            )?)
        }
        "delete_comment" => {
            let conn = state.db.lock()?;
            ok(review::delete_comment_impl(&conn, a.get("commentId")?)?)
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
        "amp_sent_comments" => ok(match &state.amp {
            Some(amp) => amp.sent_ids(a.get("reviewId")?),
            None => Vec::new(),
        }),
        "send_to_amp" => {
            let amp = state.amp.as_ref().ok_or(
                "Send to Amp is not configured (set PROLOGUE_AMP_WEBHOOK_URL on the server)",
            )?;
            let repo: String = a.get("repoPath")?;
            let base: String = a.get("base")?;
            ok(amp.send(&state.db, &repo, &base, a.get("reviewId")?)?)
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
    Json(args): Json<Value>,
) -> Response {
    let result = tokio::task::spawn_blocking(move || dispatch(&state, &command, args)).await;
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

async fn config(State(state): State<Arc<AppState>>) -> Json<WebConfig> {
    Json(state.config.clone())
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

fn router(state: Arc<AppState>, dist: &std::path::Path) -> Router {
    // Unknown paths fall back to index.html (single-page app).
    let frontend = ServeDir::new(dist).fallback(ServeFile::new(dist.join("index.html")));
    Router::new()
        .route("/api/invoke/{command}", post(invoke))
        .route("/api/events", get(event_stream))
        .route("/api/config", get(config))
        .fallback_service(frontend)
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
        amp::AmpSink::new(url.clone(), cli.amp_thread_id.clone(), data_dir.join("amp-sent.json"))
    });
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
    });

    // External writers (the prologue CLI) commit to reviews.db behind the
    // server's back; surface them as `comments-changed`, as the app does.
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
        }
    }

    fn call(state: &AppState, command: &str, args: Value) -> Result<Value, String> {
        dispatch(state, command, args).map_err(|e| match e {
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

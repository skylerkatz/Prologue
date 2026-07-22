//! Guide generation engine: everything process-shaped around the Review
//! Guide. prologue-core's guide module owns prompt assembly, validation,
//! and persistence; this module finds the `claude` CLI, runs it as a
//! subprocess with a timeout and a cancel handle, and maps every failure
//! to a distinct user-readable error.
//!
//! Flags verified against claude CLI 2.1.217: `--bare` is deliberately NOT
//! used — it never reads OAuth/Keychain credentials, and riding the user's
//! subscription login is the point. Isolation comes from a neutral working
//! directory instead (no CLAUDE.md auto-discovery).

use prologue_core::db::Db;
use prologue_core::diff::{DiffMode, DiffSpec, RepoDiff};
use prologue_core::guide::{self, Guide, GuideEngine, GuideRequest, GuideResponse, NewGuide};
use prologue_core::repo::open_git_repo;
use prologue_core::review::{self, ReviewStatus};
use prologue_core::rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use tauri::Manager;

/// Hardcoded per settled design decision 3 — no settings surface in v1.
const GUIDE_MODEL: &str = "sonnet";
const GUIDE_EFFORT: &str = "low";
const GUIDE_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub const MISSING_CLI_ERROR: &str =
    "The `claude` CLI was not found — install Claude Code to generate guides.";
const AUTH_ERROR: &str =
    "Claude Code is not logged in — run `claude` in a terminal to log in.";
const CANCELLED_ERROR: &str = "Guide generation was cancelled.";
const BUSY_ERROR: &str = "A guide is already being generated.";

// ---------------------------------------------------------------------------
// CLI resolution
// ---------------------------------------------------------------------------

/// Resolved CLI path. Hits are cached (re-verified with a cheap `is_file`
/// on each read); misses re-probe every call, so installing the CLI
/// mid-session is picked up without an app restart.
static CLAUDE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn clear_cached_claude() {
    *CLAUDE_PATH.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

pub fn resolve_claude() -> Option<PathBuf> {
    {
        let mut cached = CLAUDE_PATH.lock().unwrap_or_else(PoisonError::into_inner);
        match cached.as_ref() {
            Some(path) if path.is_file() => return Some(path.clone()),
            Some(_) => *cached = None,
            None => {}
        }
    }
    let found = find_claude_in(&candidate_dirs()).or_else(login_shell_claude);
    if let Some(path) = &found {
        *CLAUDE_PATH.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.clone());
    }
    found
}

/// Where `claude` lives in practice. A Finder-launched app inherits
/// launchd's minimal PATH, so the PATH variable is a hint, not the answer —
/// the well-known install dirs (Homebrew, /usr/local, the native
/// installer's ~/.claude/local, ~/.local/bin) are probed too.
fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    dirs.extend([PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]);
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.extend([
            home.join(".local/bin"),
            home.join(".claude/local"),
            home.join("bin"),
        ]);
    }
    dirs
}

fn find_claude_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|dir| dir.join("claude")).find(|p| p.is_file())
}

/// Last resort: ask a zsh login shell, which sources the user's dotfiles
/// and so sees whatever PATH a terminal would (nvm, Herd, custom dirs).
fn login_shell_claude() -> Option<PathBuf> {
    let output = Command::new("/bin/zsh")
        .args(["-lc", "command -v claude"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    path.is_file().then_some(path)
}

// ---------------------------------------------------------------------------
// Subprocess run
// ---------------------------------------------------------------------------

/// The CLI's `--json-schema` validator rejects a schema carrying the
/// draft-2020-12 `$schema` meta-key ("no schema with key or ref …" —
/// verified against 2.1.217), so strip it before passing the schema
/// through. Every keyword the guide schema uses is draft-agnostic.
fn cli_schema(schema: &str) -> Result<String, String> {
    let mut value: serde_json::Value =
        serde_json::from_str(schema).map_err(|e| format!("Invalid guide output schema: {e}"))?;
    if let Some(map) = value.as_object_mut() {
        map.remove("$schema");
    }
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

/// Read a child stream to the end on its own thread — the child would
/// stall on a full pipe if nobody drained it while the poll loop waits.
fn drain<R: Read + Send + 'static>(stream: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut stream) = stream {
            let _ = stream.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

fn spawn_error(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        // The cached path went stale (CLI uninstalled/moved); re-probe next
        // time instead of failing the same way forever.
        clear_cached_claude();
        MISSING_CLI_ERROR.to_owned()
    } else {
        format!("Could not start the `claude` CLI: {e}")
    }
}

fn looks_like_auth_failure(text: &str) -> bool {
    const AUTH_HINTS: &[&str] = &[
        "log in",
        "login",
        "logged in",
        "authentication",
        "authenticate",
        "unauthorized",
        "api key",
        "oauth",
        "credential",
    ];
    let lower = text.to_lowercase();
    AUTH_HINTS.iter().any(|hint| lower.contains(hint))
}

fn excerpt(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_owned()
    } else {
        let cut: String = text.chars().take(max_chars).collect();
        format!("{cut}…")
    }
}

fn exit_error(status: ExitStatus, stderr: &str) -> String {
    if looks_like_auth_failure(stderr) {
        return AUTH_ERROR.to_owned();
    }
    let detail = excerpt(stderr.trim(), 300);
    if detail.is_empty() {
        format!("The `claude` CLI failed ({status}).")
    } else {
        format!("The `claude` CLI failed ({status}): {detail}")
    }
}

/// The `--output-format json` result envelope, reduced to the fields the
/// guide needs. Auth failures arrive here too: exit code 0 with
/// `is_error: true` and "Not logged in · Please run /login" in `result`
/// (verified against 2.1.217), not as a non-zero exit.
#[derive(Deserialize)]
struct ResultEnvelope {
    #[serde(default)]
    is_error: bool,
    subtype: Option<String>,
    result: Option<serde_json::Value>,
    structured_output: Option<serde_json::Value>,
    total_cost_usd: Option<f64>,
    /// Keyed by exact model id (e.g. "claude-sonnet-5") — the alias we pass
    /// to `--model` resolves to whatever is current, so record the truth.
    #[serde(rename = "modelUsage", default)]
    model_usage: serde_json::Map<String, serde_json::Value>,
}

fn parse_envelope(stdout: &str) -> Result<GuideResponse, String> {
    let envelope: ResultEnvelope = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("Unexpected output from the `claude` CLI: {e}"))?;
    let result_text = envelope
        .result
        .as_ref()
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if envelope.is_error {
        if looks_like_auth_failure(result_text) {
            return Err(AUTH_ERROR.to_owned());
        }
        let detail = if result_text.is_empty() {
            envelope.subtype.unwrap_or_else(|| "unknown error".to_owned())
        } else {
            excerpt(result_text, 300)
        };
        return Err(format!("Claude could not generate the guide: {detail}"));
    }
    let structured = envelope
        .structured_output
        .ok_or_else(|| "The `claude` CLI returned no structured output for the guide.".to_owned())?;
    let model = envelope
        .model_usage
        .keys()
        .next()
        .cloned()
        .unwrap_or_else(|| GUIDE_MODEL.to_owned());
    Ok(GuideResponse {
        structured,
        model,
        cost_usd: envelope.total_cost_usd,
    })
}

/// Spawn `claude -p` for `request`, parking the child in `job` so
/// `cancel_guide` can kill it, and reap it with `timeout` as the deadline.
fn run_claude(
    claude: &Path,
    request: &GuideRequest,
    job: &GuideJob,
    timeout: Duration,
) -> Result<GuideResponse, String> {
    let schema = cli_schema(request.schema)?;
    if job.cancelled.load(Ordering::Relaxed) {
        return Err(CANCELLED_ERROR.to_owned());
    }
    let mut child = Command::new(claude)
        .arg("-p")
        .arg(&request.prompt)
        .args(["--model", GUIDE_MODEL, "--effort", GUIDE_EFFORT])
        .args(["--allowedTools", ""])
        .args(["--output-format", "json", "--json-schema"])
        .arg(&schema)
        // CLAUDE.md auto-discovery keys off the working directory; a neutral
        // cwd keeps the surrounding project's context (and its token cost)
        // out of every guide call.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| spawn_error(&e))?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    *job.child_slot() = Some(child);

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        if Instant::now() >= deadline {
            timed_out = true;
        }
        if timed_out || job.cancelled.load(Ordering::Relaxed) {
            if let Some(child) = job.child_slot().as_mut() {
                let _ = child.kill();
            }
        }
        let waited = match job.child_slot().as_mut() {
            Some(child) => child.try_wait(),
            None => break None,
        };
        match waited {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(e) => return Err(format!("Could not wait for the `claude` CLI: {e}")),
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    *job.child_slot() = None;

    // On the kill paths, return without joining the drain threads: a
    // grandchild orphaned by the kill can hold the inherited pipes open,
    // and joining would block until it exits. The threads finish (and
    // free their pipe ends) on their own.
    if job.cancelled.load(Ordering::Relaxed) {
        return Err(CANCELLED_ERROR.to_owned());
    }
    if timed_out {
        return Err(format!(
            "Guide generation timed out after {} seconds.",
            timeout.as_secs()
        ));
    }
    let Some(status) = status else {
        return Err(CANCELLED_ERROR.to_owned());
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        return Err(exit_error(status, &stderr));
    }
    parse_envelope(&stdout)
}

/// [`GuideEngine`] over the `claude -p` subprocess.
struct CliGuideEngine<'a> {
    claude: PathBuf,
    job: &'a GuideJob,
    timeout: Duration,
}

impl GuideEngine for CliGuideEngine<'_> {
    fn generate(&self, request: &GuideRequest) -> Result<GuideResponse, String> {
        run_claude(&self.claude, request, self.job, self.timeout)
    }
}

// ---------------------------------------------------------------------------
// Job state and commands
// ---------------------------------------------------------------------------

/// One in-flight generation: the child handle (for kill) and the cancel
/// flag the poll loop watches.
#[derive(Default, Debug)]
pub struct GuideJob {
    child: Mutex<Option<Child>>,
    cancelled: AtomicBool,
}

impl GuideJob {
    fn child_slot(&self) -> MutexGuard<'_, Option<Child>> {
        self.child.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(child) = self.child_slot().as_mut() {
            let _ = child.kill();
        }
    }
}

/// Managed state: the at-most-one running guide generation.
#[derive(Default)]
pub struct GuideRuntime(Mutex<Option<Arc<GuideJob>>>);

impl GuideRuntime {
    fn slot(&self) -> MutexGuard<'_, Option<Arc<GuideJob>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn begin(&self) -> Result<Arc<GuideJob>, String> {
        let mut slot = self.slot();
        if slot.is_some() {
            return Err(BUSY_ERROR.to_owned());
        }
        let job = Arc::new(GuideJob::default());
        *slot = Some(Arc::clone(&job));
        Ok(job)
    }

    fn finish(&self) {
        *self.slot() = None;
    }
}

/// Archived reviews are read-only everywhere else in the app; guide
/// generation refuses them too rather than writing into a frozen review.
fn ensure_review_active(conn: &Connection, review_id: i64) -> Result<(), String> {
    match review::get_review(conn, review_id)?.status {
        ReviewStatus::Active => Ok(()),
        ReviewStatus::Archived => Err(
            "This review is archived and read-only — guides can only be generated for \
             active reviews."
                .to_owned(),
        ),
    }
}

/// The whole generation pipeline, on a blocking thread. The database mutex
/// is held only for the pre-flight archived check and the final save —
/// never while the model runs, so the rest of the app stays responsive.
fn generate_blocking(
    app: &tauri::AppHandle,
    job: &GuideJob,
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    review_id: i64,
) -> Result<Guide, String> {
    let db = app.state::<Db>();
    {
        let conn = crate::commands::lock(&db)?;
        ensure_review_active(&conn, review_id)?;
    }
    let claude = resolve_claude().ok_or_else(|| MISSING_CLI_ERROR.to_owned())?;

    let repo = open_git_repo(&repo_path)?;
    let spec = DiffSpec { repo_path, base, head, mode };
    let repo_diff = RepoDiff::compute(&repo, &spec, false)?;
    let summary = repo_diff.summary()?;
    if summary.files.is_empty() {
        return Err("The diff is empty — there is nothing to build a guide from.".to_owned());
    }
    let inputs = guide::guide_file_inputs(&repo_diff, &summary)?;
    let prompt = guide::build_prompt(&inputs);
    let engine = CliGuideEngine { claude, job, timeout: GUIDE_TIMEOUT };
    let response = engine.generate(&GuideRequest {
        prompt: prompt.text,
        schema: guide::OUTPUT_SCHEMA,
    })?;

    let changed_paths: Vec<String> = summary.files.iter().map(|f| f.path.clone()).collect();
    let sections = guide::validate_sections(&response.structured, &changed_paths)?;
    let fingerprints: BTreeMap<String, String> = summary
        .files
        .iter()
        .map(|f| (f.path.clone(), f.fingerprint.clone()))
        .collect();

    let conn = crate::commands::lock(&db)?;
    // The review could have been archived while the model ran.
    ensure_review_active(&conn, review_id)?;
    guide::save_guide_impl(
        &conn,
        &NewGuide {
            review_id,
            base_ref: summary.base_ref.clone(),
            head_ref: summary.head_ref.clone(),
            mode,
            fingerprints,
            model: response.model,
            cost_usd: response.cost_usd,
        },
        &sections,
    )
}

/// Generate (or regenerate — a full replace) the review guide for the
/// current diff coordinates. One at a time; `cancel_guide` kills the
/// subprocess mid-run; the run times out after ~120s.
#[tauri::command]
pub async fn generate_guide(
    app: tauri::AppHandle,
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    review_id: i64,
) -> Result<Guide, String> {
    let job = app.state::<GuideRuntime>().begin()?;
    let result = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || generate_blocking(&app, &job, repo_path, base, head, mode, review_id)
    })
    .await
    .unwrap_or_else(|e| Err(format!("Guide generation task failed: {e}")));
    app.state::<GuideRuntime>().finish();
    result
}

/// Kill the in-flight guide generation, if any; returns whether there was
/// one. The generating command itself reports the cancellation as its
/// error — this only pulls the trigger.
#[tauri::command]
pub fn cancel_guide(runtime: tauri::State<'_, GuideRuntime>) -> bool {
    match runtime.slot().as_ref() {
        Some(job) => {
            job.cancel();
            true
        }
        None => false,
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GuideCliStatus {
    pub available: bool,
    /// Resolved absolute path, for display and debugging.
    pub path: Option<String>,
}

/// Whether the `claude` CLI is reachable — the UI disables the Guide
/// button (with an install hint) when it is not. Cheap once installed (a
/// cached-path check); a miss re-probes, including the login-shell lookup.
#[tauri::command]
pub async fn guide_cli_status() -> GuideCliStatus {
    tauri::async_runtime::spawn_blocking(|| {
        let path = resolve_claude();
        GuideCliStatus {
            available: path.is_some(),
            path: path.map(|p| p.display().to_string()),
        }
    })
    .await
    .unwrap_or(GuideCliStatus { available: false, path: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;

    fn request() -> GuideRequest {
        GuideRequest { prompt: "hello".to_owned(), schema: guide::OUTPUT_SCHEMA }
    }

    /// A fake `claude` executable running `body` as a shell script.
    fn stub_claude(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    const SUCCESS_ENVELOPE: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"done","structured_output":{"sections":[{"title":"Core","summary":"s","files":["a.rs"]}]},"total_cost_usd":0.0123,"modelUsage":{"claude-sonnet-5":{"inputTokens":2}}}"#;

    #[test]
    fn cli_schema_strips_the_meta_schema_key() {
        let cleaned = cli_schema(guide::OUTPUT_SCHEMA).unwrap();
        assert!(!cleaned.contains("$schema"));
        let value: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(value["required"], json!(["sections"]));
        assert_eq!(value["properties"]["sections"]["type"], "array");
    }

    #[test]
    fn envelope_success_yields_structured_output_model_and_cost() {
        let response = parse_envelope(SUCCESS_ENVELOPE).unwrap();
        assert_eq!(response.structured["sections"][0]["title"], "Core");
        assert_eq!(response.model, "claude-sonnet-5");
        assert_eq!(response.cost_usd, Some(0.0123));
    }

    #[test]
    fn envelope_model_falls_back_to_the_requested_alias() {
        let raw = r#"{"is_error":false,"structured_output":{"sections":[]},"modelUsage":{}}"#;
        let response = parse_envelope(raw).unwrap();
        assert_eq!(response.model, GUIDE_MODEL);
        assert_eq!(response.cost_usd, None);
    }

    #[test]
    fn envelope_not_logged_in_maps_to_the_login_hint() {
        // Verified real shape: auth failure is exit 0 + is_error, not a
        // non-zero exit.
        let raw = r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login","total_cost_usd":0}"#;
        assert_eq!(parse_envelope(raw).unwrap_err(), AUTH_ERROR);
    }

    #[test]
    fn envelope_other_errors_carry_the_result_text() {
        let raw = r#"{"is_error":true,"result":"overloaded, try later"}"#;
        let err = parse_envelope(raw).unwrap_err();
        assert!(err.contains("overloaded, try later"), "{err}");
    }

    #[test]
    fn envelope_without_structured_output_is_an_error() {
        let raw = r#"{"is_error":false,"result":"prose instead of JSON"}"#;
        let err = parse_envelope(raw).unwrap_err();
        assert!(err.contains("no structured output"), "{err}");
    }

    #[test]
    fn envelope_garbage_is_an_error() {
        let err = parse_envelope("claude: command exploded").unwrap_err();
        assert!(err.contains("Unexpected output"), "{err}");
    }

    #[test]
    fn exit_errors_classify_auth_vs_generic() {
        let auth = "Invalid API key · Please run /login";
        let status = std::process::Command::new("false").status().unwrap();
        assert_eq!(exit_error(status, auth), AUTH_ERROR);
        let generic = exit_error(status, "segfault or whatever");
        assert!(generic.contains("segfault or whatever"), "{generic}");
        assert!(exit_error(status, "").contains("failed"));
    }

    #[test]
    fn find_claude_probes_dirs_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let hit = dir.path().join("bin");
        std::fs::create_dir_all(&hit).unwrap();
        std::fs::write(hit.join("claude"), "").unwrap();
        let dirs = vec![dir.path().join("missing"), hit.clone()];
        assert_eq!(find_claude_in(&dirs), Some(hit.join("claude")));
        assert_eq!(find_claude_in(&dirs[..1]), None);
    }

    #[test]
    fn candidate_dirs_include_the_well_known_locations() {
        let dirs = candidate_dirs();
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        assert!(dirs.contains(&home.join(".claude/local")));
        assert!(dirs.contains(&home.join(".local/bin")));
    }

    #[test]
    fn run_claude_passes_the_verified_flag_set() {
        let dir = tempfile::tempdir().unwrap();
        let args_file = dir.path().join("args.txt");
        let stub = stub_claude(
            dir.path(),
            &format!(
                "printf '%s\\n' \"$@\" > {}\necho '{SUCCESS_ENVELOPE}'",
                args_file.display()
            ),
        );
        let job = GuideJob::default();
        run_claude(&stub, &request(), &job, GUIDE_TIMEOUT).unwrap();

        let args = std::fs::read_to_string(&args_file).unwrap();
        let args: Vec<&str> = args.lines().collect();
        assert_eq!(
            &args[..10],
            [
                "-p", "hello", "--model", "sonnet", "--effort", "low", "--allowedTools", "",
                "--output-format", "json"
            ]
        );
        assert_eq!(args[10], "--json-schema");
        assert!(!args[11].contains("$schema"), "meta-key must be stripped");
        assert!(args[11].contains("\"sections\""));
    }

    #[test]
    fn run_claude_parses_the_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let stub = stub_claude(dir.path(), &format!("echo '{SUCCESS_ENVELOPE}'"));
        let job = GuideJob::default();
        let response = run_claude(&stub, &request(), &job, GUIDE_TIMEOUT).unwrap();
        assert_eq!(response.model, "claude-sonnet-5");
        // The child slot is cleared once the run is over.
        assert!(job.child_slot().is_none());
    }

    #[test]
    fn run_claude_maps_auth_stderr_on_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let stub = stub_claude(dir.path(), "echo 'Please run /login' >&2\nexit 1");
        let job = GuideJob::default();
        let err = run_claude(&stub, &request(), &job, GUIDE_TIMEOUT).unwrap_err();
        assert_eq!(err, AUTH_ERROR);
    }

    #[test]
    fn run_claude_times_out_and_kills_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let stub = stub_claude(dir.path(), "sleep 30");
        let job = GuideJob::default();
        let started = Instant::now();
        let err = run_claude(&stub, &request(), &job, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(job.child_slot().is_none());
    }

    #[test]
    fn cancel_kills_the_child_and_reports_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let stub = stub_claude(dir.path(), "sleep 30");
        let job = Arc::new(GuideJob::default());
        let canceller = {
            let job = Arc::clone(&job);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                job.cancel();
            })
        };
        let started = Instant::now();
        let err = run_claude(&stub, &request(), &job, GUIDE_TIMEOUT).unwrap_err();
        assert_eq!(err, CANCELLED_ERROR);
        assert!(started.elapsed() < Duration::from_secs(5));
        canceller.join().unwrap();
    }

    #[test]
    fn runtime_allows_one_job_at_a_time() {
        let runtime = GuideRuntime::default();
        let _job = runtime.begin().unwrap();
        assert_eq!(runtime.begin().unwrap_err(), BUSY_ERROR);
        runtime.finish();
        assert!(runtime.begin().is_ok());
    }

    #[test]
    fn archived_reviews_refuse_guide_generation() {
        let dir = tempfile::tempdir().unwrap();
        let conn = prologue_core::db::open(&dir.path().join("t.db")).unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES ('/r', 'feature', 'main', 'committed')",
            [],
        )
        .unwrap();
        let review_id = conn.last_insert_rowid();
        assert!(ensure_review_active(&conn, review_id).is_ok());
        conn.execute("UPDATE reviews SET status = 'archived' WHERE id = ?1", [review_id])
            .unwrap();
        let err = ensure_review_active(&conn, review_id).unwrap_err();
        assert!(err.contains("archived"), "{err}");
        // A missing review is its own error, not an archived one.
        assert!(ensure_review_active(&conn, 999).unwrap_err().contains("not found"));
    }
}

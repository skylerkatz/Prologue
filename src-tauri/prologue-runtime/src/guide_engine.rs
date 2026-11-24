//! Guide generation engine: everything process-shaped around the Review
//! Guide. prologue-core's guide module owns prompt assembly, validation,
//! and persistence; this module picks a CLI to run the model — Claude Code
//! (`claude -p`) or Amp (`amp -x`) — runs it as a subprocess with a timeout
//! and a cancel handle, and maps every failure to a distinct user-readable
//! error.
//!
//! Engine choice: the Mac app rides the user's Claude Code subscription
//! when `claude` is installed; Amp takes over when `claude` is missing, and
//! is preferred inside an Amp orb (`AMP_ORB` set), where the Amp CLI is
//! always present and already signed in.
//!
//! Claude flags verified against claude CLI 2.1.217: `--bare` is
//! deliberately NOT used — it never reads OAuth/Keychain credentials, and
//! riding the user's subscription login is the point. Isolation comes from
//! a neutral working directory instead (no CLAUDE.md auto-discovery).

use prologue_core::db::Db;
use prologue_core::diff::{DiffMode, DiffSpec, RepoDiff};
use prologue_core::guide::{self, Guide, GuideEngine, GuideRequest, GuideResponse, NewGuide};
use prologue_core::repo::open_git_repo;
use prologue_core::review::{self, ReviewStatus};
use prologue_core::rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Hardcoded per settled design decision 3 — no settings surface in v1.
const GUIDE_MODEL: &str = "sonnet";
const GUIDE_EFFORT: &str = "low";
/// The same model and effort through Amp (verified against Amp CLI
/// 0.0.1795543373).
const AMP_GUIDE_MODEL: &str = "anthropic/claude-sonnet-5-5";
/// Amp has no `--json-schema`; a reply that fails the schema check gets
/// one more try, within the same overall timeout.
const AMP_ATTEMPTS: usize = 2;
const GUIDE_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub const MISSING_CLI_ERROR: &str = "No guide engine was found — install Claude Code (the \
     `claude` CLI) or the Amp CLI (`amp`) to generate guides.";
const CANCELLED_ERROR: &str = "Guide generation was cancelled.";
const BUSY_ERROR: &str = "A guide is already being generated.";

/// Which CLI writes the guide.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineKind {
    ClaudeCode,
    Amp,
}

impl EngineKind {
    fn binary(self) -> &'static str {
        match self {
            EngineKind::ClaudeCode => "claude",
            EngineKind::Amp => "amp",
        }
    }

    /// How errors name the CLI.
    fn label(self) -> &'static str {
        match self {
            EngineKind::ClaudeCode => "the `claude` CLI",
            EngineKind::Amp => "the Amp CLI",
        }
    }

    fn missing_error(self) -> &'static str {
        match self {
            EngineKind::ClaudeCode => {
                "The `claude` CLI was not found — install Claude Code to generate guides."
            }
            EngineKind::Amp => {
                "The Amp CLI (`amp`) was not found — install Amp to generate guides."
            }
        }
    }

    fn auth_error(self) -> &'static str {
        match self {
            EngineKind::ClaudeCode => {
                "Claude Code is not logged in — run `claude` in a terminal to log in."
            }
            EngineKind::Amp => {
                "The Amp CLI is not logged in — run `amp login` in a terminal (or set \
                 AMP_API_KEY) to log in."
            }
        }
    }

    /// Resolved CLI path. Hits are cached (re-verified with a cheap
    /// `is_file` on each read); misses re-probe every call, so installing
    /// the CLI mid-session is picked up without an app restart.
    fn cache(self) -> &'static Mutex<Option<PathBuf>> {
        static CLAUDE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
        static AMP_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
        match self {
            EngineKind::ClaudeCode => &CLAUDE_PATH,
            EngineKind::Amp => &AMP_PATH,
        }
    }
}

// ---------------------------------------------------------------------------
// CLI resolution and engine choice
// ---------------------------------------------------------------------------

fn clear_cached(kind: EngineKind) {
    *kind.cache().lock().unwrap_or_else(PoisonError::into_inner) = None;
}

pub fn resolve_cli(kind: EngineKind) -> Option<PathBuf> {
    {
        let mut cached = kind.cache().lock().unwrap_or_else(PoisonError::into_inner);
        match cached.as_ref() {
            Some(path) if path.is_file() => return Some(path.clone()),
            Some(_) => *cached = None,
            None => {}
        }
    }
    let found =
        find_in(&candidate_dirs(), kind.binary()).or_else(|| login_shell_lookup(kind.binary()));
    if let Some(path) = &found {
        *kind.cache().lock().unwrap_or_else(PoisonError::into_inner) = Some(path.clone());
    }
    found
}

fn in_amp_orb() -> bool {
    std::env::var_os("AMP_ORB").is_some_and(|v| !v.is_empty())
}

/// Claude Code first on a user's machine (their subscription, the original
/// engine); Amp first inside an Amp orb.
fn engine_preference(in_orb: bool) -> [EngineKind; 2] {
    if in_orb {
        [EngineKind::Amp, EngineKind::ClaudeCode]
    } else {
        [EngineKind::ClaudeCode, EngineKind::Amp]
    }
}

/// The first preferred engine whose CLI resolves. Lazy: a later engine is
/// only probed when the earlier ones are missing.
fn pick_engine(
    in_orb: bool,
    mut resolve: impl FnMut(EngineKind) -> Option<PathBuf>,
) -> Option<(EngineKind, PathBuf)> {
    engine_preference(in_orb)
        .into_iter()
        .find_map(|kind| resolve(kind).map(|path| (kind, path)))
}

pub fn resolve_engine() -> Option<(EngineKind, PathBuf)> {
    pick_engine(in_amp_orb(), resolve_cli)
}

/// Where the CLIs live in practice. A Finder-launched app inherits
/// launchd's minimal PATH, so the PATH variable is a hint, not the answer —
/// the well-known install dirs (Homebrew, /usr/local, Claude's native
/// installer's ~/.claude/local, Amp's ~/.amp/bin, ~/.local/bin) are probed
/// too.
fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.extend([
            home.join(".local/bin"),
            home.join(".claude/local"),
            home.join(".amp/bin"),
            home.join("bin"),
        ]);
    }
    dirs
}

fn find_in(dirs: &[PathBuf], binary: &str) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(binary))
        .find(|p| p.is_file())
}

/// The first shell that exists: zsh (the macOS default, where the user's
/// PATH setup lives), else the user's `$SHELL`, else bash or sh — a Debian
/// orb has no /bin/zsh.
fn pick_login_shell(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|shell| shell.is_file())
}

/// Last resort: ask a login shell, which sources the user's dotfiles and
/// so sees whatever PATH a terminal would (nvm, Herd, custom dirs).
fn login_shell_lookup(binary: &str) -> Option<PathBuf> {
    let shell = pick_login_shell(
        std::iter::once(PathBuf::from("/bin/zsh"))
            .chain(std::env::var_os("SHELL").map(PathBuf::from))
            .chain([PathBuf::from("/bin/bash"), PathBuf::from("/bin/sh")]),
    )?;
    let output = Command::new(shell)
        .args(["-lc", &format!("command -v {binary}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
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

fn spawn_error(kind: EngineKind, e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        // The cached path went stale (CLI uninstalled/moved); re-probe next
        // time instead of failing the same way forever.
        clear_cached(kind);
        kind.missing_error().to_owned()
    } else {
        format!("Could not start {}: {e}", kind.label())
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

fn exit_error(kind: EngineKind, status: ExitStatus, stderr: &str) -> String {
    if looks_like_auth_failure(stderr) {
        return kind.auth_error().to_owned();
    }
    let detail = excerpt(stderr.trim(), 300);
    if detail.is_empty() {
        format!("{} failed ({status}).", capitalize(kind.label()))
    } else {
        format!("{} failed ({status}): {detail}", capitalize(kind.label()))
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
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
            return Err(EngineKind::ClaudeCode.auth_error().to_owned());
        }
        let detail = if result_text.is_empty() {
            envelope
                .subtype
                .unwrap_or_else(|| "unknown error".to_owned())
        } else {
            excerpt(result_text, 300)
        };
        return Err(format!("Claude could not generate the guide: {detail}"));
    }
    let structured = envelope.structured_output.ok_or_else(|| {
        "The `claude` CLI returned no structured output for the guide.".to_owned()
    })?;
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

struct ChildOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

/// Spawn `command` (feeding it `stdin`, if any), parking the child in `job`
/// so `cancel_guide` can kill it, and reap it with `deadline` as the cutoff.
/// `timeout` only words the timeout error.
fn run_child(
    mut command: Command,
    stdin: Option<String>,
    kind: EngineKind,
    job: &GuideJob,
    deadline: Instant,
    timeout: Duration,
) -> Result<ChildOutput, String> {
    if job.cancelled.load(Ordering::Relaxed) {
        return Err(CANCELLED_ERROR.to_owned());
    }
    let mut child = command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| spawn_error(kind, &e))?;
    // Write the prompt on its own thread: a prompt larger than the pipe
    // buffer would block until the child reads it. Dropping the pipe at
    // the end sends EOF.
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        });
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    *job.child_slot() = Some(child);

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
            Err(e) => return Err(format!("Could not wait for {}: {e}", kind.label())),
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
    Ok(ChildOutput {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Run `claude -p` for `request`.
fn run_claude(
    claude: &Path,
    request: &GuideRequest,
    job: &GuideJob,
    timeout: Duration,
) -> Result<GuideResponse, String> {
    let schema = cli_schema(request.schema)?;
    let mut command = Command::new(claude);
    command
        .arg("-p")
        .arg(&request.prompt)
        .args(["--model", GUIDE_MODEL, "--effort", GUIDE_EFFORT])
        .args(["--allowedTools", ""])
        .args(["--output-format", "json", "--json-schema"])
        .arg(&schema)
        // CLAUDE.md auto-discovery keys off the working directory; a neutral
        // cwd keeps the surrounding project's context (and its token cost)
        // out of every guide call.
        .current_dir(std::env::temp_dir());
    let deadline = Instant::now() + timeout;
    let out = run_child(
        command,
        None,
        EngineKind::ClaudeCode,
        job,
        deadline,
        timeout,
    )?;
    if !out.status.success() {
        return Err(exit_error(EngineKind::ClaudeCode, out.status, &out.stderr));
    }
    parse_envelope(&out.stdout)
}

// ---------------------------------------------------------------------------
// Amp engine
// ---------------------------------------------------------------------------

/// Amp settings for guide runs: every tool off, so the run is one model
/// reply with no file reads, shell, or web access. `--settings-file`
/// replaces the user's settings for the run; credentials live elsewhere.
const AMP_SETTINGS: &str = r#"{"amp.tools.disable":["*"]}"#;

fn amp_settings_file() -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join("prologue-amp-guide-settings.json");
    std::fs::write(&path, AMP_SETTINGS)
        .map_err(|e| format!("Could not write the Amp settings file for the guide: {e}"))?;
    Ok(path)
}

/// The guide prompt plus the output contract `--json-schema` enforces for
/// Claude Code, which Amp lacks; a retry also says why the first reply
/// was rejected.
fn amp_prompt(request: &GuideRequest, rejected: Option<&str>) -> Result<String, String> {
    let schema = cli_schema(request.schema)?;
    let mut prompt = format!(
        "{}\n\n## Output format\n\nReply with only a JSON object that matches this JSON \
         Schema — no prose before or after it and no code fences:\n\n{schema}\n",
        request.prompt
    );
    if let Some(reason) = rejected {
        prompt.push_str(&format!(
            "\nAn earlier reply to this request was rejected because {reason}. Reply with \
             the JSON object only.\n"
        ));
    }
    Ok(prompt)
}

/// One `--stream-json` line, reduced to what the guide needs: the `system`
/// init line names the model (`agent_mode`), the final `result` line
/// carries the reply text or the error.
#[derive(Deserialize)]
struct AmpStreamLine {
    #[serde(rename = "type")]
    kind: String,
    agent_mode: Option<String>,
    #[serde(default)]
    is_error: bool,
    subtype: Option<String>,
    result: Option<serde_json::Value>,
}

/// A finished `amp -x` run: the model and its final reply text.
#[derive(Debug, PartialEq)]
struct AmpReply {
    model: String,
    text: String,
}

fn parse_amp_stream(stdout: &str) -> Result<AmpReply, String> {
    let mut model = None;
    let mut result = None;
    for line in stdout.lines() {
        let Ok(line) = serde_json::from_str::<AmpStreamLine>(line.trim()) else {
            continue;
        };
        match line.kind.as_str() {
            "system" => model = line.agent_mode.or(model),
            "result" => result = Some(line),
            _ => {}
        }
    }
    let Some(result) = result else {
        let detail = excerpt(stdout.trim(), 300);
        return Err(if detail.is_empty() {
            "The Amp CLI exited without a reply.".to_owned()
        } else {
            format!("Unexpected output from the Amp CLI: {detail}")
        });
    };
    let text = result
        .result
        .as_ref()
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    if result.is_error {
        if looks_like_auth_failure(&text) {
            return Err(EngineKind::Amp.auth_error().to_owned());
        }
        let detail = if text.is_empty() {
            result.subtype.unwrap_or_else(|| "unknown error".to_owned())
        } else {
            excerpt(&text, 300)
        };
        return Err(format!("Amp could not generate the guide: {detail}"));
    }
    Ok(AmpReply {
        model: model.unwrap_or_else(|| AMP_GUIDE_MODEL.to_owned()),
        text,
    })
}

/// [`guide::OUTPUT_SCHEMA`] as types: required fields, string/array
/// types, and `additionalProperties: false` (deny_unknown_fields). Claude
/// Code checks the schema itself; Amp replies are checked here.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Only deserialized, as a check.
struct SchemaOutput {
    sections: Vec<SchemaSection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct SchemaSection {
    title: String,
    summary: String,
    files: Vec<String>,
}

/// The JSON object in a reply that was asked to be only that object.
/// Tolerates code fences or a stray sentence around it by taking the span
/// from the first `{` to the last `}`, then checks it against the schema.
/// The error completes "rejected because …".
fn parse_guide_json(text: &str) -> Result<serde_json::Value, String> {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err("it contained no JSON object".to_owned());
    };
    if end < start {
        return Err("it contained no JSON object".to_owned());
    }
    let value: serde_json::Value = serde_json::from_str(&text[start..=end])
        .map_err(|e| format!("it was not valid JSON ({e})"))?;
    serde_json::from_value::<SchemaOutput>(value.clone())
        .map_err(|e| format!("it did not match the guide schema ({e})"))?;
    Ok(value)
}

/// Run `amp -x` for `request`, retrying once when the reply does not pass
/// the schema. Both attempts share one deadline, so the guide still gives
/// up after `timeout`.
fn run_amp(
    amp: &Path,
    request: &GuideRequest,
    job: &GuideJob,
    timeout: Duration,
) -> Result<GuideResponse, String> {
    let settings = amp_settings_file()?;
    let deadline = Instant::now() + timeout;
    let mut rejected: Option<String> = None;
    for _ in 0..AMP_ATTEMPTS {
        let mut command = Command::new(amp);
        command
            .args(["-x", "--stream-json"])
            .args(["--model", AMP_GUIDE_MODEL, "--effort", GUIDE_EFFORT])
            .arg("--settings-file")
            .arg(&settings)
            .args(["--title", "Prologue review guide"])
            // As with Claude Code: keep the surrounding project's AGENTS.md
            // out of the run.
            .current_dir(std::env::temp_dir());
        // The prompt goes in on stdin: it can approach 100 KB, past the
        // single-argument limit on Linux.
        let prompt = amp_prompt(request, rejected.as_deref())?;
        let out = run_child(
            command,
            Some(prompt),
            EngineKind::Amp,
            job,
            deadline,
            timeout,
        )?;
        if !out.status.success() {
            return Err(exit_error(EngineKind::Amp, out.status, &out.stderr));
        }
        let reply = parse_amp_stream(&out.stdout)?;
        match parse_guide_json(&reply.text) {
            Ok(structured) => {
                return Ok(GuideResponse {
                    structured,
                    model: reply.model,
                    // Amp's stream does not report cost.
                    cost_usd: None,
                });
            }
            Err(reason) => rejected = Some(reason),
        }
    }
    Err(format!(
        "Amp did not return a usable guide (tried twice): {}.",
        rejected.unwrap_or_default()
    ))
}

/// [`GuideEngine`] over the chosen CLI.
struct CliGuideEngine<'a> {
    kind: EngineKind,
    path: PathBuf,
    job: &'a GuideJob,
    timeout: Duration,
}

impl GuideEngine for CliGuideEngine<'_> {
    fn generate(&self, request: &GuideRequest) -> Result<GuideResponse, String> {
        match self.kind {
            EngineKind::ClaudeCode => run_claude(&self.path, request, self.job, self.timeout),
            EngineKind::Amp => run_amp(&self.path, request, self.job, self.timeout),
        }
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
    db: &Db,
    job: &GuideJob,
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    review_id: i64,
) -> Result<Guide, String> {
    {
        let conn = db.lock()?;
        ensure_review_active(&conn, review_id)?;
    }
    let (kind, path) = resolve_engine().ok_or_else(|| MISSING_CLI_ERROR.to_owned())?;

    let repo = open_git_repo(&repo_path)?;
    let spec = DiffSpec {
        repo_path,
        base,
        head,
        mode,
    };
    let repo_diff = RepoDiff::compute(&repo, &spec, false)?;
    let summary = repo_diff.summary()?;
    if summary.files.is_empty() {
        return Err("The diff is empty — there is nothing to build a guide from.".to_owned());
    }
    let inputs = guide::guide_file_inputs(&repo_diff, &summary)?;
    let prompt = guide::build_prompt(&inputs);
    let engine = CliGuideEngine {
        kind,
        path,
        job,
        timeout: GUIDE_TIMEOUT,
    };
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

    let conn = db.lock()?;
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
/// subprocess mid-run; the run times out after ~120s. Blocks for the whole
/// run — hosts call it from a blocking thread.
pub fn generate_guide(
    db: &Db,
    runtime: &GuideRuntime,
    repo_path: String,
    base: String,
    head: String,
    mode: DiffMode,
    review_id: i64,
) -> Result<Guide, String> {
    /// Frees the slot even if the pipeline panics, so one bad run cannot
    /// leave every later generation refused as busy.
    struct Finish<'a>(&'a GuideRuntime);
    impl Drop for Finish<'_> {
        fn drop(&mut self) {
            self.0.finish();
        }
    }
    let job = runtime.begin()?;
    let _finish = Finish(runtime);
    generate_blocking(db, &job, repo_path, base, head, mode, review_id)
}

/// Kill the in-flight guide generation, if any; returns whether there was
/// one. The generating call itself reports the cancellation as its error —
/// this only pulls the trigger.
pub fn cancel_guide(runtime: &GuideRuntime) -> bool {
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
    /// Which CLI generates guides: `claude-code` or `amp`.
    pub engine: Option<EngineKind>,
}

/// Whether a guide engine (the `claude` or `amp` CLI) is reachable — the
/// UI disables the Guide button (with an install hint) when neither is.
/// Cheap once installed (a cached-path check); a miss re-probes, including
/// the login-shell lookup, so hosts call it from a blocking thread.
pub fn guide_cli_status() -> GuideCliStatus {
    let engine = resolve_engine();
    GuideCliStatus {
        available: engine.is_some(),
        path: engine.as_ref().map(|(_, p)| p.display().to_string()),
        engine: engine.map(|(kind, _)| kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;

    fn request() -> GuideRequest {
        GuideRequest {
            prompt: "hello".to_owned(),
            schema: guide::OUTPUT_SCHEMA,
        }
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
        assert_eq!(
            parse_envelope(raw).unwrap_err(),
            EngineKind::ClaudeCode.auth_error()
        );
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
        assert_eq!(
            exit_error(EngineKind::ClaudeCode, status, auth),
            EngineKind::ClaudeCode.auth_error()
        );
        assert_eq!(
            exit_error(
                EngineKind::Amp,
                status,
                "No API key found. Run `amp login` first."
            ),
            EngineKind::Amp.auth_error()
        );
        let generic = exit_error(EngineKind::ClaudeCode, status, "segfault or whatever");
        assert!(generic.contains("segfault or whatever"), "{generic}");
        assert!(generic.starts_with("The `claude` CLI failed"), "{generic}");
        assert_eq!(
            exit_error(EngineKind::Amp, status, "").split(" (").next(),
            Some("The Amp CLI failed")
        );
    }

    #[test]
    fn find_claude_probes_dirs_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let hit = dir.path().join("bin");
        std::fs::create_dir_all(&hit).unwrap();
        std::fs::write(hit.join("claude"), "").unwrap();
        let dirs = vec![dir.path().join("missing"), hit.clone()];
        assert_eq!(find_in(&dirs, "claude"), Some(hit.join("claude")));
        assert_eq!(find_in(&dirs[..1], "claude"), None);
        assert_eq!(find_in(&dirs, "amp"), None);
    }

    #[test]
    fn candidate_dirs_include_the_well_known_locations() {
        let dirs = candidate_dirs();
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        assert!(dirs.contains(&home.join(".claude/local")));
        assert!(dirs.contains(&home.join(".local/bin")));
        assert!(dirs.contains(&home.join(".amp/bin")));
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
                "-p",
                "hello",
                "--model",
                "sonnet",
                "--effort",
                "low",
                "--allowedTools",
                "",
                "--output-format",
                "json"
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
        assert_eq!(err, EngineKind::ClaudeCode.auth_error());
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

    // -----------------------------------------------------------------------
    // Engine choice
    // -----------------------------------------------------------------------

    /// `pick_engine` over a fixed set of installed CLIs, recording which
    /// ones it probed.
    fn pick(in_orb: bool, installed: &[EngineKind]) -> (Option<EngineKind>, Vec<EngineKind>) {
        let mut probed = Vec::new();
        let picked = pick_engine(in_orb, |kind| {
            probed.push(kind);
            installed
                .contains(&kind)
                .then(|| PathBuf::from(kind.binary()))
        });
        (picked.map(|(kind, _)| kind), probed)
    }

    #[test]
    fn mac_prefers_claude_code_and_never_probes_amp_when_it_is_there() {
        use EngineKind::{Amp, ClaudeCode};
        assert_eq!(
            pick(false, &[ClaudeCode, Amp]),
            (Some(ClaudeCode), vec![ClaudeCode])
        );
        assert_eq!(pick(false, &[Amp]).0, Some(Amp));
        assert_eq!(pick(false, &[]), (None, vec![ClaudeCode, Amp]));
    }

    #[test]
    fn amp_orb_prefers_amp_and_falls_back_to_claude_code() {
        use EngineKind::{Amp, ClaudeCode};
        assert_eq!(pick(true, &[ClaudeCode, Amp]), (Some(Amp), vec![Amp]));
        assert_eq!(pick(true, &[ClaudeCode]).0, Some(ClaudeCode));
        assert_eq!(pick(true, &[]).0, None);
    }

    #[test]
    fn engine_kind_serializes_for_the_status_payload() {
        let status = GuideCliStatus {
            available: true,
            path: Some("/x/amp".to_owned()),
            engine: Some(EngineKind::Amp),
        };
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            json!({"available": true, "path": "/x/amp", "engine": "amp"})
        );
        assert_eq!(
            serde_json::to_value(EngineKind::ClaudeCode).unwrap(),
            "claude-code"
        );
    }

    #[test]
    fn login_shell_skips_shells_that_do_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let missing_zsh = dir.path().join("zsh");
        let sh = dir.path().join("sh");
        std::fs::write(&sh, "").unwrap();
        assert_eq!(
            pick_login_shell([missing_zsh.clone(), sh.clone()]),
            Some(sh)
        );
        assert_eq!(pick_login_shell([missing_zsh]), None);
    }

    // -----------------------------------------------------------------------
    // Amp output handling
    // -----------------------------------------------------------------------

    const GUIDE_JSON: &str = r#"{"sections":[{"title":"Core","summary":"s","files":["a.rs"]}]}"#;

    /// A `--stream-json` transcript whose final reply is `reply`.
    fn amp_stream(reply: &str) -> String {
        let result =
            json!({"type": "result", "subtype": "success", "is_error": false, "result": reply});
        format!(
            "{}\n{}\n{}\n",
            r#"{"type":"system","subtype":"init","tools":[],"agent_mode":"anthropic/claude-sonnet-5-5"}"#,
            r#"{"type":"assistant","message":{"content":[]}}"#,
            result
        )
    }

    #[test]
    fn amp_stream_yields_the_reply_and_model() {
        let reply = parse_amp_stream(&amp_stream("hello")).unwrap();
        assert_eq!(
            reply,
            AmpReply {
                model: "anthropic/claude-sonnet-5-5".to_owned(),
                text: "hello".to_owned()
            }
        );
    }

    #[test]
    fn amp_stream_errors_are_classified() {
        let auth =
            r#"{"type":"result","is_error":true,"result":"Not logged in. Run `amp login` first."}"#;
        assert_eq!(
            parse_amp_stream(auth).unwrap_err(),
            EngineKind::Amp.auth_error()
        );
        let other = r#"{"type":"result","is_error":true,"result":"out of credits"}"#;
        let err = parse_amp_stream(other).unwrap_err();
        assert_eq!(err, "Amp could not generate the guide: out of credits");
        assert!(parse_amp_stream("")
            .unwrap_err()
            .contains("without a reply"));
        let err = parse_amp_stream("Error: Unknown command").unwrap_err();
        assert!(
            err.contains("Unexpected output from the Amp CLI: Error: Unknown command"),
            "{err}"
        );
    }

    #[test]
    fn guide_json_tolerates_fences_and_rejects_off_schema_replies() {
        let fenced = format!("Here you go:\n```json\n{GUIDE_JSON}\n```");
        assert_eq!(
            parse_guide_json(&fenced).unwrap()["sections"][0]["files"],
            json!(["a.rs"])
        );

        let extra = r#"{"sections":[{"title":"T","summary":"s","files":[],"why":"x"}]}"#;
        assert!(parse_guide_json(extra)
            .unwrap_err()
            .contains("did not match the guide schema"));
        let missing = r#"{"sections":[{"title":"T","files":["a.rs"]}]}"#;
        assert!(parse_guide_json(missing).unwrap_err().contains("summary"));
        let wrong_type = r#"{"sections":[{"title":"T","summary":"s","files":"a.rs"}]}"#;
        assert!(parse_guide_json(wrong_type).is_err());
        let top_level_extra = r#"{"sections":[],"notes":"x"}"#;
        assert!(parse_guide_json(top_level_extra).is_err());
        assert!(parse_guide_json("{not json}")
            .unwrap_err()
            .contains("not valid JSON"));
        assert_eq!(
            parse_guide_json("I could not do it.").unwrap_err(),
            "it contained no JSON object"
        );
    }

    /// The typed check in `parse_guide_json` mirrors OUTPUT_SCHEMA; fail
    /// here if the schema's shape moves without it.
    #[test]
    fn schema_check_matches_the_output_schema() {
        let schema: serde_json::Value = serde_json::from_str(guide::OUTPUT_SCHEMA).unwrap();
        assert_eq!(schema["required"], json!(["sections"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        let item = &schema["properties"]["sections"]["items"];
        assert_eq!(item["required"], json!(["title", "summary", "files"]));
        assert_eq!(item["additionalProperties"], json!(false));
        let props: Vec<&String> = item["properties"].as_object().unwrap().keys().collect();
        assert_eq!(props.len(), 3);
    }

    /// A fake `amp`: records its args and stdin per call (`args.N`,
    /// `stdin.N`), then runs `body` with `$n` set to the call number.
    fn stub_amp(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("amp");
        let d = dir.display();
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nn=$(ls {d} | grep -c '^args\\.')\nprintf '%s\\n' \"$@\" > {d}/args.$n\n\
                 cat > {d}/stdin.$n\n{body}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn write_stream(dir: &Path, name: &str, reply: &str) {
        std::fs::write(dir.join(name), amp_stream(reply)).unwrap();
    }

    #[test]
    fn run_amp_sends_the_prompt_on_stdin_with_tools_off() {
        let dir = tempfile::tempdir().unwrap();
        write_stream(dir.path(), "ok.jsonl", GUIDE_JSON);
        let stub = stub_amp(
            dir.path(),
            &format!("cat {}/ok.jsonl", dir.path().display()),
        );
        let job = GuideJob::default();
        let response = run_amp(&stub, &request(), &job, GUIDE_TIMEOUT).unwrap();
        assert_eq!(
            response.structured,
            serde_json::from_str::<serde_json::Value>(GUIDE_JSON).unwrap()
        );
        assert_eq!(response.model, "anthropic/claude-sonnet-5-5");
        assert_eq!(response.cost_usd, None);

        let args = std::fs::read_to_string(dir.path().join("args.0")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        assert_eq!(
            &args[..7],
            [
                "-x",
                "--stream-json",
                "--model",
                AMP_GUIDE_MODEL,
                "--effort",
                "low",
                "--settings-file"
            ]
        );
        assert_eq!(std::fs::read_to_string(args[7]).unwrap(), AMP_SETTINGS);
        let stdin = std::fs::read_to_string(dir.path().join("stdin.0")).unwrap();
        assert!(stdin.starts_with("hello\n\n## Output format"), "{stdin}");
        assert!(stdin.contains("\"additionalProperties\":false") && !stdin.contains("$schema"));
        assert!(
            !dir.path().join("args.1").exists(),
            "a valid reply is not retried"
        );
    }

    #[test]
    fn run_amp_retries_once_after_an_off_schema_reply() {
        let dir = tempfile::tempdir().unwrap();
        write_stream(
            dir.path(),
            "0.jsonl",
            "Sure! The sections are Core and Tests.",
        );
        write_stream(dir.path(), "1.jsonl", GUIDE_JSON);
        let stub = stub_amp(
            dir.path(),
            &format!("cat {}/$n.jsonl", dir.path().display()),
        );
        let job = GuideJob::default();
        let response = run_amp(&stub, &request(), &job, GUIDE_TIMEOUT).unwrap();
        assert_eq!(response.structured["sections"][0]["title"], "Core");
        let retry = std::fs::read_to_string(dir.path().join("stdin.1")).unwrap();
        assert!(
            retry.contains("rejected because it contained no JSON object"),
            "{retry}"
        );
    }

    #[test]
    fn run_amp_gives_up_after_two_off_schema_replies() {
        let dir = tempfile::tempdir().unwrap();
        write_stream(dir.path(), "bad.jsonl", r#"{"sections":"none"}"#);
        let stub = stub_amp(
            dir.path(),
            &format!("cat {}/bad.jsonl", dir.path().display()),
        );
        let job = GuideJob::default();
        let err = run_amp(&stub, &request(), &job, GUIDE_TIMEOUT).unwrap_err();
        assert!(
            err.starts_with("Amp did not return a usable guide (tried twice): it did not match"),
            "{err}"
        );
        assert!(!dir.path().join("args.2").exists());
    }

    #[test]
    fn run_amp_maps_auth_stderr_and_does_not_retry() {
        let dir = tempfile::tempdir().unwrap();
        let stub = stub_amp(
            dir.path(),
            "echo 'No API key found. Run `amp login` first.' >&2\nexit 1",
        );
        let job = GuideJob::default();
        let err = run_amp(&stub, &request(), &job, GUIDE_TIMEOUT).unwrap_err();
        assert_eq!(err, EngineKind::Amp.auth_error());
        assert!(!dir.path().join("args.1").exists());
    }

    #[test]
    fn run_amp_times_out_and_kills_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let stub = stub_amp(dir.path(), "sleep 30");
        let job = GuideJob::default();
        let started = Instant::now();
        let err = run_amp(&stub, &request(), &job, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(job.child_slot().is_none());
    }

    #[test]
    fn run_amp_reports_a_missing_binary_as_missing() {
        let job = GuideJob::default();
        let err = run_amp(
            Path::new("/nonexistent/amp"),
            &request(),
            &job,
            GUIDE_TIMEOUT,
        )
        .unwrap_err();
        assert_eq!(err, EngineKind::Amp.missing_error());
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
        conn.execute(
            "UPDATE reviews SET status = 'archived' WHERE id = ?1",
            [review_id],
        )
        .unwrap();
        let err = ensure_review_active(&conn, review_id).unwrap_err();
        assert!(err.contains("archived"), "{err}");
        // A missing review is its own error, not an archived one.
        assert!(ensure_review_active(&conn, 999)
            .unwrap_err()
            .contains("not found"));
    }
}

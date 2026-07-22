use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Manager, State};

/// Event emitted to the frontend after repo activity settles; the payload is
/// the exact `repo_path` string `start_watching` was called with, so the
/// frontend can compare it against its open repo without path normalization.
const REPO_CHANGED_EVENT: &str = "repo-changed";

/// Event emitted when another connection — the prologue CLI — commits to
/// reviews.db. The app's own writes never fire it (see `start_db_watching`).
const COMMENTS_CHANGED_EVENT: &str = "comments-changed";

/// Quiet period after the last filesystem event before one refresh fires.
/// Long enough to coalesce a commit's burst of `.git` writes, short enough
/// to feel immediate.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);

/// The single active watch, following the open repo: replaced on repo
/// switch, dropped on close. Dropping the `RecommendedWatcher` stops event
/// delivery, which disconnects the channel and ends the debounce thread.
pub struct RepoWatcher(pub Mutex<Option<RecommendedWatcher>>);

impl Default for RepoWatcher {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

/// Watch `repo_path` (working tree + `.git`, recursively) and emit a
/// debounced `repo-changed` event on activity. Replaces any previous watch.
/// Gitignored paths (node_modules/, target/, …) are filtered out, so builds
/// and installs running in the reviewed repo don't trigger refreshes.
///
/// The app's own SQLite writes cannot trigger this: reviews.db lives in the
/// app data dir, never inside the watched repository.
#[tauri::command]
pub fn start_watching(
    app: AppHandle,
    state: State<RepoWatcher>,
    repo_path: String,
) -> Result<(), String> {
    let (tx, rx): (Sender<()>, Receiver<()>) = std::sync::mpsc::channel();

    let filter = prologue_core::repo::RepoEventFilter::new(&repo_path)?;
    let mut watcher = change_watcher(tx, move |path| filter.is_relevant(path), "file")?;

    watcher
        .watch(std::path::Path::new(&repo_path), RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch {repo_path}: {e}"))?;

    std::thread::spawn(move || {
        debounce_loop(&rx, DEBOUNCE_WINDOW, || {
            let _ = app.emit(REPO_CHANGED_EVENT, repo_path.clone());
        });
    });

    // Storing the new watcher drops the previous one (if any), ending its
    // event stream and debounce thread.
    *state.0.lock().map_err(|_| "Watcher state poisoned")? = Some(watcher);
    Ok(())
}

/// Drop the active watch (repo closed / back to the welcome page).
#[tauri::command]
pub fn stop_watching(state: State<RepoWatcher>) -> Result<(), String> {
    *state.0.lock().map_err(|_| "Watcher state poisoned")? = None;
    Ok(())
}

/// Watch the app-data dir (where reviews.db and its -wal live) and emit a
/// debounced `comments-changed` event when an external connection committed
/// to the database. Called once at setup; the watch lives as long as the app.
///
/// `PRAGMA data_version` on the app's own connection moves only when a
/// DIFFERENT connection commits, so the app's own writes — which do produce
/// filesystem events here — are filtered out and never self-trigger a
/// refresh.
pub fn start_db_watching(app: AppHandle, dir: std::path::PathBuf) -> Result<(), String> {
    let (tx, rx): (Sender<()>, Receiver<()>) = std::sync::mpsc::channel();

    let mut watcher = change_watcher(tx, |_| true, "database")?;

    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch {}: {e}", dir.display()))?;

    std::thread::spawn(move || {
        // The watcher moves in with the debounce thread and lives (keeps
        // sending) for the app's lifetime, so this loop never exits.
        let _watcher = watcher;
        let mut last = data_version(&app);
        debounce_loop(&rx, DEBOUNCE_WINDOW, || {
            let current = data_version(&app);
            if current.is_some() && current != last {
                last = current;
                let _ = app.emit(COMMENTS_CHANGED_EVENT, ());
            }
        });
    });
    Ok(())
}

/// The app connection's `PRAGMA data_version`; None if the database state
/// is unavailable (poisoned lock), which skips the comparison rather than
/// emitting spuriously.
fn data_version(app: &AppHandle) -> Option<i64> {
    let db = app.state::<prologue_core::db::Db>();
    let conn = db.0.lock().ok()?;
    conn.pragma_query_value(None, "data_version", |r| r.get(0)).ok()
}

/// A watcher forwarding change events to `tx`, keeping only events with at
/// least one path `relevant` accepts. `context` names the watcher in the
/// creation error ("file", "database").
fn change_watcher(
    tx: Sender<()>,
    relevant: impl Fn(&std::path::Path) -> bool + Send + 'static,
    context: &'static str,
) -> Result<RecommendedWatcher, String> {
    notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            if is_change(&event.kind) && event_is_relevant(&event, &relevant) {
                // A full channel or hung receiver is not this thread's
                // problem; the debounce loop drains everything anyway.
                let _ = tx.send(());
            }
        }
    })
    .map_err(|e| format!("Failed to create {context} watcher: {e}"))
}

/// Whether any of the event's paths passes `relevant`. Events without paths
/// stay relevant — some backends omit them, and a spurious refresh is
/// cheaper than a missed one.
fn event_is_relevant(event: &Event, relevant: &impl Fn(&std::path::Path) -> bool) -> bool {
    event.paths.is_empty() || event.paths.iter().any(|p| relevant(p))
}

/// Filesystem events that can change a diff. Access (reads) are noise —
/// the app itself reads the repo on every refresh.
fn is_change(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any
    )
}

/// Trailing debounce: after an event arrives, wait until `window` passes
/// with no further events, then call `emit` once. Runs until the sender
/// side disconnects (the watcher was dropped); a shutdown mid-burst does
/// not emit — the repo is being closed or re-targeted.
fn debounce_loop<T>(rx: &Receiver<T>, window: Duration, mut emit: impl FnMut()) {
    while rx.recv().is_ok() {
        loop {
            match rx.recv_timeout(window) {
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => {
                    emit();
                    break;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::thread;

    const WINDOW: Duration = Duration::from_millis(30);

    /// Run `debounce_loop` on a receiver, counting emits, while `send`
    /// drives the sender on this thread; returns the emit count once the
    /// loop ends (sender dropped).
    fn run_debounce(send: impl FnOnce(Sender<()>)) -> usize {
        let (tx, rx) = channel();
        let handle = thread::spawn(move || {
            let mut emits = 0;
            debounce_loop(&rx, WINDOW, || emits += 1);
            emits
        });
        send(tx);
        handle.join().unwrap()
    }

    #[test]
    fn a_burst_of_events_emits_once() {
        let emits = run_debounce(|tx| {
            for _ in 0..20 {
                tx.send(()).unwrap();
                thread::sleep(Duration::from_millis(1));
            }
            // Keep the sender alive past the quiet window; dropping it
            // earlier is a shutdown, which deliberately skips the emit.
            thread::sleep(WINDOW * 3);
        });
        assert_eq!(emits, 1);
    }

    #[test]
    fn events_inside_the_window_keep_extending_the_wait() {
        let emits = run_debounce(|tx| {
            // Each gap is under WINDOW, so the whole spread coalesces.
            for _ in 0..5 {
                tx.send(()).unwrap();
                thread::sleep(WINDOW / 2);
            }
            thread::sleep(WINDOW * 3);
        });
        assert_eq!(emits, 1);
    }

    #[test]
    fn separated_bursts_emit_separately() {
        let emits = run_debounce(|tx| {
            tx.send(()).unwrap();
            thread::sleep(WINDOW * 3);
            tx.send(()).unwrap();
            thread::sleep(WINDOW * 3);
        });
        assert_eq!(emits, 2);
    }

    #[test]
    fn no_events_means_no_emit() {
        let emits = run_debounce(|_tx| {});
        assert_eq!(emits, 0);
    }

    /// The invariant `start_db_watching` rests on: `PRAGMA data_version`
    /// is unchanged by this connection's own commits and moves when any
    /// other connection commits — that's the whole self-trigger filter.
    #[test]
    fn data_version_moves_only_on_other_connections_commits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reviews.db");
        let ours = prologue_core::db::open(&path).unwrap();
        let version = |conn: &prologue_core::rusqlite::Connection| -> i64 {
            conn.pragma_query_value(None, "data_version", |r| r.get(0)).unwrap()
        };

        let before = version(&ours);
        ours.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES ('/r', 'b', 'main', 'committed')",
            [],
        )
        .unwrap();
        assert_eq!(version(&ours), before, "own write must not move data_version");

        let theirs = prologue_core::db::open(&path).unwrap();
        theirs
            .execute(
                "INSERT INTO reviews (repo_path, branch, base_ref, mode)
                 VALUES ('/r2', 'b', 'main', 'committed')",
                [],
            )
            .unwrap();
        assert_ne!(version(&ours), before, "external write must move data_version");
    }

    #[test]
    fn disconnect_mid_burst_does_not_emit() {
        let emits = run_debounce(|tx| {
            tx.send(()).unwrap();
            // Sender drops immediately — shutdown wins over the pending emit.
        });
        assert_eq!(emits, 0);
    }

    /// A repo with ignore rules, plus the filter `start_watching` builds
    /// for it — the exact wiring the repo watch runs events through.
    fn ignore_fixture() -> (prologue_core::testutil::FixtureRepo, prologue_core::repo::RepoEventFilter)
    {
        let fixture = prologue_core::testutil::FixtureRepo::new();
        fixture.commit_file(".gitignore", "node_modules/\ntarget/\n", "ignore rules");
        fixture.commit_file("src/main.rs", "fn main() {}\n", "code");
        let filter = prologue_core::repo::RepoEventFilter::new(&fixture.path()).unwrap();
        (fixture, filter)
    }

    fn change_event(path: std::path::PathBuf) -> Event {
        Event::new(EventKind::Create(notify::event::CreateKind::File)).add_path(path)
    }

    #[test]
    fn gitignored_paths_do_not_forward_repo_events() {
        let (fixture, filter) = ignore_fixture();
        let root = std::path::PathBuf::from(fixture.path());
        let relevant = |p: &std::path::Path| filter.is_relevant(p);

        let ignored = change_event(root.join("node_modules/pkg/index.js"));
        assert!(!event_is_relevant(&ignored, &relevant));
        let build_output = change_event(root.join("target/debug/deps/foo.o"));
        assert!(!event_is_relevant(&build_output, &relevant));
        // One relevant path among ignored ones is enough to forward.
        let mixed = change_event(root.join("node_modules/x.js")).add_path(root.join("src/main.rs"));
        assert!(event_is_relevant(&mixed, &relevant));
    }

    #[test]
    fn tracked_files_and_git_head_still_forward_repo_events() {
        let (fixture, filter) = ignore_fixture();
        let root = std::path::PathBuf::from(fixture.path());
        let relevant = |p: &std::path::Path| filter.is_relevant(p);

        let tracked = change_event(root.join("src/main.rs"));
        assert!(event_is_relevant(&tracked, &relevant));
        let commit = change_event(root.join(".git/HEAD"));
        assert!(event_is_relevant(&commit, &relevant));
        // Pathless events (some backends omit paths) stay conservative.
        let pathless = Event::new(EventKind::Any);
        assert!(event_is_relevant(&pathless, &relevant));
    }
}

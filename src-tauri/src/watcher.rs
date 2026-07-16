use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, State};

/// Event emitted to the frontend after repo activity settles; the payload is
/// the exact `repo_path` string `start_watching` was called with, so the
/// frontend can compare it against its open repo without path normalization.
const REPO_CHANGED_EVENT: &str = "repo-changed";

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

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            if is_change(&event.kind) {
                // A full channel or hung receiver is not this thread's
                // problem; the debounce loop drains everything anyway.
                let _ = tx.send(());
            }
        }
    })
    .map_err(|e| format!("Failed to create file watcher: {e}"))?;

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

    #[test]
    fn disconnect_mid_burst_does_not_emit() {
        let emits = run_debounce(|tx| {
            tx.send(()).unwrap();
            // Sender drops immediately — shutdown wins over the pending emit.
        });
        assert_eq!(emits, 0);
    }
}

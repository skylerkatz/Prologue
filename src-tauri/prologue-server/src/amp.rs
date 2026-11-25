//! "Send to Amp": when `PROLOGUE_AMP_WEBHOOK_URL` is set, the browser's
//! Export button asks the server to notify an Amp thread that open comments
//! await it. The webhook URL is a credential, so only this server ever sees
//! or calls it. The payload names comments by id and never carries their
//! text — the agent reads them with the `prologue` CLI.
//!
//! Sends are confirmed, not fire-and-forget: Amp can accept the webhook
//! POST and still never run the thread's handler (an archived thread).
//! Each send gets a `sendId` and stays `sending` until the handler runs
//! `prologue confirm-send <sendId>`; a refused POST fails it at once, and
//! an unconfirmed one fails after [`CONFIRM_TIMEOUT_SECS`], so its comments
//! can be sent again. State lives in the database (`amp_sends`, schema v9);
//! amp-sent.json from older servers is adopted as confirmed sends.

use prologue_core::amp_sends;
use prologue_core::db::Db;
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

/// How long a send may wait for `prologue confirm-send` before it counts
/// as lost.
pub const CONFIRM_TIMEOUT_SECS: u64 = 60;

pub struct AmpSink {
    url: String,
    thread_id: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub send_id: String,
    pub comment_ids: Vec<String>,
    pub count: usize,
}

impl AmpSink {
    pub fn new(url: String, thread_id: String) -> Self {
        Self { url, thread_id }
    }

    /// Record the review's open, unsent thread roots as a `sending` send
    /// (`recorded` runs once that is visible, so other viewers can see it),
    /// POST the webhook, and fail the send right away unless it answers 2xx.
    pub fn send(
        &self,
        db: &Db,
        repo: &str,
        base: &str,
        review_id: i64,
        recorded: impl Fn(),
    ) -> Result<SendResult, String> {
        let send_id = uuid::Uuid::new_v4().to_string();
        let ids = amp_sends::begin_send(&*db.lock()?, review_id, &send_id)?;
        recorded();
        let comment_ids: Vec<String> = ids.iter().map(|id| format!("C{id}")).collect();
        let payload = json!({
            "sendId": send_id,
            "threadID": self.thread_id,
            "commentIds": comment_ids,
            "count": ids.len(),
            "repo": repo,
            "base": base,
        });
        if let Err(e) = self.post(&send_id, "the comments", &payload) {
            amp_sends::fail_send(&*db.lock()?, &send_id)?;
            return Err(e);
        }
        Ok(SendResult { send_id, count: comment_ids.len(), comment_ids })
    }

    /// POST and log the outcome; Err unless 2xx. Neither the log nor the
    /// error may echo the URL: it is a credential.
    fn post(&self, send_id: &str, what: &str, payload: &serde_json::Value) -> Result<(), String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build()
            .into();
        match agent.post(&self.url).send_json(payload) {
            Ok(mut res) => {
                let status = res.status().as_u16();
                let body = res.body_mut().read_to_string().unwrap_or_default();
                let short: String = body.trim().chars().take(200).collect();
                eprintln!("prologue-server: Send to Amp {send_id}: webhook answered {status} {short:?}");
                if (200..300).contains(&status) {
                    Ok(())
                } else {
                    Err(format!("Amp did not accept {what} (HTTP {status}). Try again."))
                }
            }
            Err(e) => {
                let e = redact(&e, &self.url);
                eprintln!("prologue-server: Send to Amp {send_id}: webhook failed: {e}");
                Err(format!("Amp did not accept {what}: {e}"))
            }
        }
    }
}

/// Adopt the pre-v9 amp-sent.json (review id → sent root ids) as confirmed
/// sends; a missing or unreadable file means nothing was sent yet.
pub fn import_legacy_sent(db: &Db, path: &Path) -> Result<(), String> {
    let Some(sent) = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<BTreeMap<i64, BTreeSet<i64>>>(&text).ok())
    else {
        return Ok(());
    };
    amp_sends::import_legacy(&*db.lock()?, &sent)
}

/// Fail sends unconfirmed for [`CONFIRM_TIMEOUT_SECS`], every few seconds,
/// calling `changed` when any did. Timestamps drive it, so sends left in
/// flight by a stopped server are settled once one runs again.
pub fn start_expiry(db: std::sync::Arc<Db>, changed: impl Fn() + Send + 'static) {
    std::thread::spawn(move || loop {
        if let Ok(conn) = db.lock() {
            if amp_sends::expire_sends(&conn, CONFIRM_TIMEOUT_SECS).unwrap_or(0) > 0 {
                drop(conn);
                eprintln!("prologue-server: Send to Amp: a send went unconfirmed for {CONFIRM_TIMEOUT_SECS}s; its comments are unsent again");
                changed();
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    });
}

/// The message the Amp plugin adds to the thread for a send — the manual
/// fallback when automatic sending failed, pasted by the reviewer, so the
/// agent handles both the same way. Mirrors the plugin's text exactly; keep
/// them in step.
pub fn handoff_message(repo: &str, comment_ids: &[i64]) -> String {
    let range = match (comment_ids.first(), comment_ids.last()) {
        (Some(first), Some(last)) if first != last => format!("C{first}–C{last}"),
        (Some(only), _) => format!("C{only}"),
        _ => String::new(),
    };
    format!(
        "You have {} open review comments in Prologue ({range}) for {repo}. Use the \
         prologue-review skill to work through them, reply to each with the prologue CLI, \
         then give me a short summary.",
        comment_ids.len()
    )
}

/// The Amp plugin built from the same commit as this server, whose
/// `PLUGIN_VERSION` is the newest a release knows of.
const PLUGIN_SOURCE: &str = include_str!("../../../amp-plugin/prologue/index.ts");

/// `PLUGIN_VERSION` in the plugin source above.
pub fn latest_plugin_version() -> u32 {
    PLUGIN_SOURCE
        .lines()
        .find_map(|line| line.trim().strip_prefix("const PLUGIN_VERSION = "))
        .and_then(|rest| rest.trim_end_matches(';').trim().parse().ok())
        .expect("amp-plugin/prologue/index.ts declares const PLUGIN_VERSION = <number>")
}

/// The message that asks the agent to update the plugin: copied by the
/// reviewer for plugins too old to take an update request, and added to
/// the thread by newer ones. Mirrors the plugin's text exactly; keep them
/// in step.
pub const PLUGIN_UPDATE_MESSAGE: &str =
    "Update my Prologue Amp plugin to the latest version: clone the plugin repository it is \
     installed in (my personal plugins, unless it is a workspace plugin), replace its `prologue` \
     directory with `amp-plugin/prologue` from the main branch of \
     https://github.com/skylerkatz/prologue, commit, and push. Then reload plugins.";

impl AmpSink {
    /// Ask the plugin to add [`PLUGIN_UPDATE_MESSAGE`] to the thread. The
    /// payload names only the request; the plugin writes the message itself.
    pub fn request_plugin_update(&self) -> Result<(), String> {
        self.post("plugin update", "the update request", &json!({ "type": "update-plugin", "threadID": self.thread_id }))
    }
}

fn redact(e: &ureq::Error, url: &str) -> String {
    e.to_string().replace(url, "<webhook>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use amp_sends::SendState;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    /// A one-shot HTTP sink answering `status`; returns the request body.
    fn sink(status: u16) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/hook/secret-token", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut len = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let mut stream = reader.into_inner();
            write!(stream, "HTTP/1.1 {status} X\r\ncontent-length: 2\r\n\r\nok").unwrap();
            String::from_utf8(body).unwrap()
        });
        (url, handle)
    }

    fn review_with_comments() -> (tempfile::TempDir, Db, i64) {
        let dir = tempfile::tempdir().unwrap();
        let conn = prologue_core::db::open(&dir.path().join("reviews.db")).unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode) VALUES ('/r', 'f', 'main', 'committed')",
            [],
        )
        .unwrap();
        let review = conn.last_insert_rowid();
        // C1 open root, C2 resolved root, C3 reply to C1, C4 open root.
        for (state, parent, body) in [
            ("open", None, "one"),
            ("resolved", None, "two"),
            ("open", Some(1), "reply"),
            ("open", None, "four"),
        ] {
            conn.execute(
                "INSERT INTO comments (review_id, level, commit_sha, state, body, parent_id)
                 VALUES (?1, 'review', 'sha', ?2, ?3, ?4)",
                prologue_core::rusqlite::params![review, state, body, parent],
            )
            .unwrap();
        }
        (dir, Db(Mutex::new(conn)), review)
    }

    fn state(db: &Db, id: &str) -> SendState {
        amp_sends::get_send(&db.lock().unwrap(), id).unwrap().unwrap().state
    }

    /// An accepted POST leaves the send `sending` (not sent) until the
    /// thread confirms it; the payload names the send and no comment text.
    #[test]
    fn an_accepted_send_waits_for_confirmation() {
        let (_dir, db, review) = review_with_comments();
        let (url, server) = sink(200);
        let amp = AmpSink::new(url, "T-123".to_owned());
        let recorded = std::cell::Cell::new(false);

        let result = amp.send(&db, "/r", "main", review, || recorded.set(true)).unwrap();
        let body: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"sendId": result.send_id, "threadID": "T-123", "commentIds": ["C1", "C4"],
                   "count": 2, "repo": "/r", "base": "main"})
        );
        assert!(recorded.get());
        assert_eq!(state(&db, &result.send_id), SendState::Sending);
        // In flight: nothing new to send, and no second copy goes out.
        let err = amp.send(&db, "/r", "main", review, || {}).unwrap_err();
        assert!(err.contains("No new"), "{err}");

        amp_sends::confirm_send(&db.lock().unwrap(), &result.send_id).unwrap();
        assert_eq!(state(&db, &result.send_id), SendState::Sent);
    }

    #[test]
    fn a_rejected_send_reverts_at_once_and_hides_the_url() {
        let (_dir, db, review) = review_with_comments();
        let (url, server) = sink(500);
        let amp = AmpSink::new(url, "T-1".to_owned());
        let err = amp.send(&db, "/r", "main", review, || {}).unwrap_err();
        server.join().unwrap();
        assert!(err.contains("500") && !err.contains("secret-token"), "{err}");
        let sends = amp_sends::list_sends(&db.lock().unwrap(), review).unwrap();
        assert_eq!(sends[0].state, SendState::Failed);
        // The same comments can go again.
        let (url, server) = sink(200);
        let amp = AmpSink::new(url, "T-1".to_owned());
        assert_eq!(amp.send(&db, "/r", "main", review, || {}).unwrap().count, 2);
        server.join().unwrap();
    }

    #[test]
    fn handoff_message_matches_the_plugin() {
        assert_eq!(
            handoff_message("/work/app", &[1, 2, 3]),
            "You have 3 open review comments in Prologue (C1–C3) for /work/app. Use the \
             prologue-review skill to work through them, reply to each with the prologue CLI, \
             then give me a short summary."
        );
        assert!(handoff_message("/r", &[7]).starts_with("You have 1 open review comments in Prologue (C7) for /r."));
    }

    #[test]
    fn legacy_amp_sent_json_stays_sent() {
        let (dir, db, review) = review_with_comments();
        let path = dir.path().join("amp-sent.json");
        std::fs::write(&path, format!(r#"{{"{review}":[1]}}"#)).unwrap();
        import_legacy_sent(&db, &path).unwrap();
        import_legacy_sent(&db, &dir.path().join("missing.json")).unwrap();
        let (url, server) = sink(200);
        let result = AmpSink::new(url, "T".to_owned()).send(&db, "/r", "main", review, || {}).unwrap();
        server.join().unwrap();
        assert_eq!(result.comment_ids, vec!["C4"]);
    }
}

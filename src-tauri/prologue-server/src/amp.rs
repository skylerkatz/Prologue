//! "Send to Amp": when `PROLOGUE_AMP_WEBHOOK_URL` is set, the browser's
//! Export button asks the server to notify an Amp thread that open comments
//! await it. The webhook URL is a credential, so only this server ever sees
//! or calls it. The payload names comments by id and never carries their
//! text — the agent reads them with the `prologue` CLI.
//!
//! Which comments were sent is remembered per review in `amp-sent.json`
//! next to reviews.db (server bookkeeping, deliberately outside the shared
//! schema the app and CLI migrate), so unsent ones can show as pending.

use prologue_core::db::Db;
use prologue_core::review::{self, CommentState};
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

pub struct AmpSink {
    url: String,
    thread_id: String,
    sent_path: PathBuf,
    /// review id → ids of root comments already sent.
    sent: Mutex<BTreeMap<i64, BTreeSet<i64>>>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub comment_ids: Vec<String>,
    pub count: usize,
}

impl AmpSink {
    pub fn new(url: String, thread_id: String, sent_path: PathBuf) -> Self {
        // A missing or unreadable file means nothing was sent yet.
        let sent = std::fs::read_to_string(&sent_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { url, thread_id, sent_path, sent: Mutex::new(sent) }
    }

    pub fn sent_ids(&self, review_id: i64) -> Vec<i64> {
        let sent = self.sent.lock().unwrap_or_else(|e| e.into_inner());
        sent.get(&review_id).map(|ids| ids.iter().copied().collect()).unwrap_or_default()
    }

    /// POST the review's open, not-yet-sent thread roots to the webhook and
    /// record them as sent once it accepts (2xx).
    pub fn send(&self, db: &Db, repo: &str, base: &str, review_id: i64) -> Result<SendResult, String> {
        let open: Vec<i64> = {
            let conn = db.lock()?;
            review::list_comments_impl(&conn, review_id)?
                .into_iter()
                .filter(|c| c.parent_id.is_none() && c.state == CommentState::Open)
                .map(|c| c.id)
                .collect()
        };
        let already: BTreeSet<i64> = self.sent_ids(review_id).into_iter().collect();
        let unsent: Vec<i64> = open.into_iter().filter(|id| !already.contains(id)).collect();
        if unsent.is_empty() {
            return Err("No new open comments to send to Amp".to_owned());
        }
        let comment_ids: Vec<String> = unsent.iter().map(|id| format!("C{id}")).collect();
        let payload = json!({
            "threadID": self.thread_id,
            "commentIds": comment_ids,
            "count": unsent.len(),
            "repo": repo,
            "base": base,
        });
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .into();
        // The error text must not echo the URL: it is a credential.
        agent
            .post(&self.url)
            .send_json(&payload)
            .map_err(|e| format!("Amp did not accept the comments: {}", redact(&e, &self.url)))?;

        let mut sent = self.sent.lock().unwrap_or_else(|e| e.into_inner());
        sent.entry(review_id).or_default().extend(&unsent);
        let text = serde_json::to_string(&*sent).map_err(|e| e.to_string())?;
        std::fs::write(&self.sent_path, text)
            .map_err(|e| format!("Sent, but could not record it in {}: {e}", self.sent_path.display()))?;
        Ok(SendResult { count: comment_ids.len(), comment_ids })
    }
}

fn redact(e: &ureq::Error, url: &str) -> String {
    e.to_string().replace(url, "<webhook>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

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
            write!(stream, "HTTP/1.1 {status} X\r\ncontent-length: 0\r\n\r\n").unwrap();
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

    #[test]
    fn sends_only_open_unsent_roots_without_text_and_remembers_them() {
        let (dir, db, review) = review_with_comments();
        let path = dir.path().join("amp-sent.json");
        let (url, server) = sink(200);
        let amp = AmpSink::new(url, "T-123".to_owned(), path.clone());

        let result = amp.send(&db, "/r", "main", review).unwrap();
        let body: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"threadID": "T-123", "commentIds": ["C1", "C4"], "count": 2, "repo": "/r", "base": "main"})
        );
        assert_eq!(result.count, 2);
        // Survives a restart, and a second send has nothing new.
        let reloaded = AmpSink::new("http://unused".to_owned(), "T-123".to_owned(), path);
        assert_eq!(reloaded.sent_ids(review), vec![1, 4]);
        assert!(reloaded.send(&db, "/r", "main", review).unwrap_err().contains("No new"));
    }

    #[test]
    fn a_rejected_send_records_nothing_and_hides_the_url() {
        let (dir, db, review) = review_with_comments();
        let (url, server) = sink(500);
        let amp = AmpSink::new(url, "T-1".to_owned(), dir.path().join("amp-sent.json"));
        let err = amp.send(&db, "/r", "main", review).unwrap_err();
        server.join().unwrap();
        assert!(!err.contains("secret-token"), "{err}");
        assert!(amp.sent_ids(review).is_empty());
    }
}

//! Send to Amp deliveries (schema v9). A send names the open thread roots
//! it carries and moves through:
//!
//! - `sending`: recorded before the webhook POST; the comments count as
//!   neither pending nor sent while the Amp thread has yet to confirm.
//! - `sent`: the thread's handler ran `prologue confirm-send <sendId>`.
//! - `failed`: the webhook refused it, it never arrived, or no confirmation
//!   came in time; its comments are unsent again and can be resent.
//!
//! prologue-server writes `sending` and `failed`; the CLI writes `sent`.

use crate::comment::{list_comments_impl, CommentState};
use crate::db::{db_err, NOW};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SendState {
    Sending,
    Sent,
    Failed,
}

impl SendState {
    fn parse(s: &str) -> SendState {
        match s {
            "sent" => SendState::Sent,
            "sending" => SendState::Sending,
            _ => SendState::Failed,
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AmpSend {
    pub send_id: String,
    pub review_id: i64,
    pub comment_ids: Vec<i64>,
    pub state: SendState,
    pub created_at: String,
    pub updated_at: String,
}

/// The error when every open thread root is already sent or in flight.
pub const NOTHING_TO_SEND: &str = "No new open comments to send to Amp";

fn row_to_send(r: &rusqlite::Row<'_>) -> rusqlite::Result<AmpSend> {
    let ids: String = r.get("comment_ids")?;
    let state: String = r.get("state")?;
    Ok(AmpSend {
        send_id: r.get("send_id")?,
        review_id: r.get("review_id")?,
        comment_ids: serde_json::from_str(&ids).unwrap_or_default(),
        state: SendState::parse(&state),
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

/// Every send of a review, oldest first.
pub fn list_sends(conn: &Connection, review_id: i64) -> Result<Vec<AmpSend>, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM amp_sends WHERE review_id = ?1 ORDER BY created_at, send_id")
        .map_err(db_err)?;
    let sends = stmt
        .query_map([review_id], row_to_send)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(sends)
}

pub fn get_send(conn: &Connection, send_id: &str) -> Result<Option<AmpSend>, String> {
    conn.query_row("SELECT * FROM amp_sends WHERE send_id = ?1", [send_id], row_to_send)
        .optional()
        .map_err(db_err)
}

/// Record a new send of the review's open thread roots that are neither
/// sent nor in flight, as `sending`, and return their ids. Picking and
/// recording happen on one connection under the caller's lock, so two
/// viewers pressing Send at once cannot both send the same comments.
pub fn begin_send(conn: &Connection, review_id: i64, send_id: &str) -> Result<Vec<i64>, String> {
    let taken: BTreeSet<i64> = list_sends(conn, review_id)?
        .into_iter()
        .filter(|s| s.state != SendState::Failed)
        .flat_map(|s| s.comment_ids)
        .collect();
    let ids: Vec<i64> = list_comments_impl(conn, review_id)?
        .into_iter()
        .filter(|c| c.parent_id.is_none() && c.state == CommentState::Open)
        .map(|c| c.id)
        .filter(|id| !taken.contains(id))
        .collect();
    if ids.is_empty() {
        return Err(NOTHING_TO_SEND.to_owned());
    }
    conn.execute(
        "INSERT INTO amp_sends (send_id, review_id, comment_ids, state) VALUES (?1, ?2, ?3, 'sending')",
        (send_id, review_id, serde_json::to_string(&ids).map_err(|e| e.to_string())?),
    )
    .map_err(db_err)?;
    Ok(ids)
}

/// A send that did not get through: its comments are unsent again. Only a
/// send still `sending` fails — a confirmation that already landed wins.
pub fn fail_send(conn: &Connection, send_id: &str) -> Result<bool, String> {
    let changed = conn
        .execute(
            &format!(
                "UPDATE amp_sends SET state = 'failed', updated_at = {NOW}
                 WHERE send_id = ?1 AND state = 'sending'"
            ),
            [send_id],
        )
        .map_err(db_err)?;
    Ok(changed > 0)
}

/// What `confirm-send` found.
#[derive(Debug, PartialEq)]
pub enum Confirmed {
    /// The send is now `sent`; `was` is the state it had (a `failed` one
    /// timed out before the thread confirmed, but its comments did arrive).
    Now { send: AmpSend, was: SendState },
    /// It was already `sent`; nothing changed.
    Already(AmpSend),
    /// No send has this id.
    Unknown,
}

/// The Amp thread received this send. Idempotent.
pub fn confirm_send(conn: &Connection, send_id: &str) -> Result<Confirmed, String> {
    let Some(send) = get_send(conn, send_id)? else {
        return Ok(Confirmed::Unknown);
    };
    if send.state == SendState::Sent {
        return Ok(Confirmed::Already(send));
    }
    conn.execute(
        &format!("UPDATE amp_sends SET state = 'sent', updated_at = {NOW} WHERE send_id = ?1"),
        [send_id],
    )
    .map_err(db_err)?;
    let was = send.state;
    Ok(Confirmed::Now { send: get_send(conn, send_id)?.unwrap_or(send), was })
}

/// Fail every send still `sending` that was recorded at least
/// `timeout_secs` ago; returns how many. Driven by timestamps, so sends in
/// flight when a server stopped are settled by the next one.
pub fn expire_sends(conn: &Connection, timeout_secs: u64) -> Result<usize, String> {
    conn.execute(
        &format!(
            "UPDATE amp_sends SET state = 'failed', updated_at = {NOW}
             WHERE state = 'sending'
               AND created_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)"
        ),
        [format!("-{timeout_secs} seconds")],
    )
    .map_err(db_err)
}

/// Adopt amp-sent.json (review id → root ids sent, from before v9) as one
/// confirmed send per review, `legacy-<review id>`. Idempotent; reviews
/// deleted since are skipped.
pub fn import_legacy(conn: &Connection, sent: &BTreeMap<i64, BTreeSet<i64>>) -> Result<(), String> {
    for (review_id, ids) in sent {
        let ids: Vec<i64> = ids.iter().copied().collect();
        conn.execute(
            "INSERT OR IGNORE INTO amp_sends (send_id, review_id, comment_ids, state)
             SELECT ?1, ?2, ?3, 'sent' WHERE EXISTS (SELECT 1 FROM reviews WHERE id = ?2)",
            (
                format!("legacy-{review_id}"),
                review_id,
                serde_json::to_string(&ids).map_err(|e| e.to_string())?,
            ),
        )
        .map_err(db_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_with_comments() -> (tempfile::TempDir, Connection, i64) {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&dir.path().join("reviews.db")).unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode) VALUES ('/r', 'f', 'main', 'committed')",
            [],
        )
        .unwrap();
        // C1 open root, C2 resolved root, C3 reply to C1, C4 open root.
        for (state, parent, body) in
            [("open", None, "one"), ("resolved", None, "two"), ("open", Some(1), "reply"), ("open", None, "four")]
        {
            conn.execute(
                "INSERT INTO comments (review_id, level, commit_sha, state, body, parent_id)
                 VALUES (1, 'review', 'sha', ?1, ?2, ?3)",
                rusqlite::params![state, body, parent],
            )
            .unwrap();
        }
        (dir, conn, 1)
    }

    fn state(conn: &Connection, id: &str) -> SendState {
        get_send(conn, id).unwrap().unwrap().state
    }

    /// In-flight and sent comments are not offered again; a failed send's
    /// comments are, and a confirmation is final and idempotent.
    #[test]
    fn sends_move_from_sending_to_sent_or_back_to_unsent() {
        let (_dir, conn, review) = review_with_comments();
        assert_eq!(begin_send(&conn, review, "a").unwrap(), vec![1, 4]);
        assert_eq!(begin_send(&conn, review, "b").unwrap_err(), NOTHING_TO_SEND);

        assert!(fail_send(&conn, "a").unwrap());
        assert_eq!(begin_send(&conn, review, "b").unwrap(), vec![1, 4], "failed ones are unsent again");

        assert!(matches!(confirm_send(&conn, "b").unwrap(), Confirmed::Now { was: SendState::Sending, .. }));
        assert!(matches!(confirm_send(&conn, "b").unwrap(), Confirmed::Already(_)));
        assert!(!fail_send(&conn, "b").unwrap(), "a confirmed send never fails");
        assert_eq!(state(&conn, "b"), SendState::Sent);
        assert_eq!(confirm_send(&conn, "nope").unwrap(), Confirmed::Unknown);
        assert_eq!(begin_send(&conn, review, "c").unwrap_err(), NOTHING_TO_SEND);
    }

    #[test]
    fn only_sends_older_than_the_timeout_expire() {
        let (_dir, conn, review) = review_with_comments();
        begin_send(&conn, review, "old").unwrap();
        conn.execute(
            "UPDATE amp_sends SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-61 seconds')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comments (review_id, level, commit_sha, body) VALUES (1, 'review', 'sha', 'five')",
            [],
        )
        .unwrap();
        begin_send(&conn, review, "new").unwrap();

        assert_eq!(expire_sends(&conn, 60).unwrap(), 1);
        assert_eq!((state(&conn, "old"), state(&conn, "new")), (SendState::Failed, SendState::Sending));
        // A late confirmation still records that the comments arrived.
        assert!(matches!(confirm_send(&conn, "old").unwrap(), Confirmed::Now { was: SendState::Failed, .. }));
    }

    #[test]
    fn legacy_sent_ids_stay_sent() {
        let (_dir, conn, review) = review_with_comments();
        let legacy = BTreeMap::from([(review, BTreeSet::from([1])), (99, BTreeSet::from([7]))]);
        import_legacy(&conn, &legacy).unwrap();
        import_legacy(&conn, &legacy).unwrap();
        let sends = list_sends(&conn, review).unwrap();
        assert_eq!(sends.len(), 1);
        assert_eq!((sends[0].state, &sends[0].comment_ids), (SendState::Sent, &vec![1]));
        assert_eq!(begin_send(&conn, review, "a").unwrap(), vec![4]);
    }
}

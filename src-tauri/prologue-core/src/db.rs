use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// Review database handle; the app manages it as Tauri state.
pub struct Db(pub Mutex<Connection>);

/// Timestamp expression used for all created_at/updated_at columns:
/// ISO-8601 UTC with millisecond precision.
pub const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

/// Ordered migrations; entry `i` brings the schema to version `i + 1`.
/// Append new entries for future schema changes — never edit applied ones.
const MIGRATIONS: &[&str] = &[
    // v1: reviews + comments per the spec's data model. Comment lifecycle
    // (state) and code anchors are stored now so M5/M6 need no migration.
    "CREATE TABLE reviews (
        id INTEGER PRIMARY KEY,
        repo_path TEXT NOT NULL,
        branch TEXT NOT NULL,
        base_ref TEXT NOT NULL,
        mode TEXT NOT NULL CHECK (mode IN ('committed','staged','all')),
        status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','archived')),
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    );
    CREATE UNIQUE INDEX reviews_one_active_per_branch
        ON reviews (repo_path, branch) WHERE status = 'active';
    CREATE TABLE comments (
        id INTEGER PRIMARY KEY,
        review_id INTEGER NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        level TEXT NOT NULL CHECK (level IN ('review','file','line')),
        file_path TEXT,
        side TEXT CHECK (side IN ('old','new')),
        start_line INTEGER,
        end_line INTEGER,
        code_anchor TEXT,
        commit_sha TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open','resolved','dismissed')),
        body TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    );
    CREATE INDEX comments_by_review ON comments (review_id);",
    // v2: comment replies — flat threads under a root comment. Replies carry
    // review_id, parent_id, body, and timestamps; positional columns stay
    // NULL (they inherit the root's context). Deleting a root cascades.
    "ALTER TABLE comments ADD COLUMN parent_id INTEGER REFERENCES comments(id) ON DELETE CASCADE;",
];

/// The newest schema version this build knows how to read and migrate to.
/// External binaries compare it against a database's recorded version before
/// opening (never operate on a schema newer than the binary).
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Open (creating if needed) the reviews database at `path` and bring its
/// schema up to date.
pub fn open(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path)
        .map_err(|e| format!("Failed to open review database: {e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("Failed to enable WAL mode: {e}"))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| format!("Failed to enable foreign keys: {e}"))?;
    // Writers from more than one process (app + CLI) share this database;
    // wait briefly on a locked connection instead of failing immediately.
    conn.busy_timeout(std::time::Duration::from_millis(500))
        .map_err(|e| format!("Failed to set busy timeout: {e}"))?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let db_err = |e: rusqlite::Error| format!("Failed to migrate review database: {e}");
    // Take the write lock up front so two processes opening concurrently
    // cannot both read the version table and race the same migration.
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(db_err)?;
    let conn = &*tx;
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT ({NOW})
        );"
    ))
    .map_err(db_err)?;
    let current: i64 = conn
        .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .map_err(db_err)?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = i as i64 + 1;
        if version <= current {
            continue;
        }
        conn.execute_batch(sql).map_err(db_err)?;
        conn.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [version])
            .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_the_schema_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reviews.db");

        let conn = open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        drop(conn);

        // Reopening must not re-run applied migrations.
        let conn = open(&path).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, MIGRATIONS.len() as i64);
    }

    #[test]
    fn open_enables_wal_foreign_keys_and_busy_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("reviews.db")).unwrap();
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
        let timeout: i64 = conn
            .pragma_query_value(None, "busy_timeout", |r| r.get(0))
            .unwrap();
        assert_eq!(timeout, 500);
    }

    #[test]
    fn only_one_active_review_per_repo_and_branch() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("reviews.db")).unwrap();
        let insert = "INSERT INTO reviews (repo_path, branch, base_ref, mode, status)
                      VALUES (?1, ?2, 'main', 'committed', ?3)";
        conn.execute(insert, ["/repo", "feature", "active"]).unwrap();
        // A second active review for the same (repo, branch) is rejected...
        assert!(conn.execute(insert, ["/repo", "feature", "active"]).is_err());
        // ...but archived duplicates and other branches are fine.
        conn.execute(insert, ["/repo", "feature", "archived"]).unwrap();
        conn.execute(insert, ["/repo", "other", "active"]).unwrap();
    }

    /// A database as v1 left it: only the first migration applied, with a
    /// review and comments already stored (Skyler's real reviews.db shape).
    fn v1_database_with_rows(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT ({NOW})
            );"
        ))
        .unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute("INSERT INTO schema_migrations (version) VALUES (1)", [])
            .unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES ('/repo', 'feature', 'main', 'committed')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comments (review_id, level, commit_sha, body, state)
             VALUES (1, 'review', 'abc', 'existing note', 'resolved')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn v2_migration_applies_in_place_to_a_v1_database_with_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reviews.db");
        v1_database_with_rows(&path);

        let conn = open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        // Existing rows survive, with parent_id defaulting to NULL.
        let (body, state, parent_id): (String, String, Option<i64>) = conn
            .query_row(
                "SELECT body, state, parent_id FROM comments WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(body, "existing note");
        assert_eq!(state, "resolved");
        assert_eq!(parent_id, None);
    }

    #[test]
    fn deleting_a_root_comment_cascades_to_its_replies() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("reviews.db")).unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES ('/repo', 'feature', 'main', 'committed')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comments (review_id, level, commit_sha, body)
             VALUES (1, 'review', 'abc', 'root')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comments (review_id, level, commit_sha, body, parent_id)
             VALUES (1, 'review', 'abc', 'reply', 1), (1, 'review', 'abc', 'reply two', 1)",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM comments WHERE id = 1", []).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM comments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn deleting_a_review_cascades_to_its_comments() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("reviews.db")).unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES ('/repo', 'feature', 'main', 'committed')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comments (review_id, level, commit_sha, body)
             VALUES (1, 'review', 'abc', 'note')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM reviews WHERE id = 1", []).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM comments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::sync::Mutex;

/// Review database handle; the app manages it as Tauri state.
pub struct Db(pub Mutex<Connection>);

/// The app's bundle identifier — also the name of its data directory under
/// ~/Library/Application Support, where reviews.db lives. Must stay in sync
/// with tauri.conf.json's `identifier` (the app debug-asserts the match at
/// startup); the CLI derives its default database path from it.
pub const APP_IDENTIFIER: &str = "com.skylerkatz.prologue";

/// Timestamp expression used for all created_at/updated_at columns:
/// ISO-8601 UTC with millisecond precision.
pub const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

/// Shared formatter for rusqlite errors surfaced to users.
pub(crate) fn db_err(e: rusqlite::Error) -> String {
    format!("Review database error: {e}")
}

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
    // v3: author attribution. The app's own writes stay 'reviewer' (the
    // default); external writers (the prologue CLI) record who spoke — the
    // UI badges anything that isn't 'reviewer'.
    "ALTER TABLE comments ADD COLUMN author TEXT NOT NULL DEFAULT 'reviewer';",
    // v4: per-file reviewed marks. `fingerprint` is the content identity of
    // both diff sides at mark time; a row whose fingerprint no longer matches
    // the current diff renders as "changed since review" until re-marked or
    // unmarked. Rows for paths that leave the diff (renames) are left in
    // place — invisible, and cascade-deleted with the review.
    "CREATE TABLE reviewed_files (
        review_id INTEGER NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        file_path TEXT NOT NULL,
        fingerprint TEXT NOT NULL,
        reviewed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        PRIMARY KEY (review_id, file_path)
    );",
    // v5: review guides — AI-generated grouping of a review's changed files
    // into ordered sections. One guide per (review, base, head, mode);
    // regeneration replaces the row. `fingerprint_json` maps file path →
    // FileSummary fingerprint at generation time, for detecting files that
    // changed since the guide. `cost_usd` is logged for the record only.
    "CREATE TABLE guides (
        id INTEGER PRIMARY KEY,
        review_id INTEGER NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        base_ref TEXT NOT NULL,
        head_ref TEXT NOT NULL,
        mode TEXT NOT NULL CHECK (mode IN ('committed','staged','all')),
        fingerprint_json TEXT NOT NULL,
        model TEXT NOT NULL,
        cost_usd REAL,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
    );
    CREATE UNIQUE INDEX guides_one_per_diff
        ON guides (review_id, base_ref, head_ref, mode);
    CREATE TABLE guide_sections (
        id INTEGER PRIMARY KEY,
        guide_id INTEGER NOT NULL REFERENCES guides(id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        title TEXT NOT NULL,
        summary TEXT NOT NULL,
        files_json TEXT NOT NULL
    );
    CREATE INDEX guide_sections_by_guide ON guide_sections (guide_id, position);",
];

/// The newest schema version this build knows how to read and migrate to.
/// External binaries compare it against a database's recorded version before
/// opening (never operate on a schema newer than the binary).
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Open (creating if needed) the reviews database at `path` and bring its
/// schema up to date. Files this build cannot understand — foreign
/// databases, or a schema newer than the binary — are refused untouched.
pub fn open(path: &Path) -> Result<Connection, String> {
    check_compatible(path)?;
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

/// The schema-version seatbelt, shared by every binary that opens the
/// database: a file with foreign tables is not ours to touch, and a schema
/// newer than this build must not be operated on (migrations only go
/// forward). Inspected over a separate read-only connection so a refused
/// file is left byte-identical. A missing file passes — `open` creates it.
fn check_compatible(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Ok(());
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Failed to open review database: {e}"))?;
    let db_err = |e: rusqlite::Error| format!("Failed to read {}: {e}", path.display());

    let tables: i64 = conn
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'", [], |r| r.get(0))
        .map_err(db_err)?;
    let has_migrations: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if has_migrations == 0 {
        // A brand-new empty file may be migrated; anything with foreign
        // tables is not ours to touch.
        return if tables == 0 {
            Ok(())
        } else {
            Err(format!("{} is not a Prologue reviews database", path.display()))
        };
    }

    let version = stored_schema_version(&conn).map_err(db_err)?;
    if version > SCHEMA_VERSION {
        return Err(format!(
            "This reviews database is newer than this build (database schema v{version}, \
             this build knows v{SCHEMA_VERSION}) — update the Prologue app or rebuild prologue"
        ));
    }
    Ok(())
}

/// The highest applied migration version, 0 on a fresh migrations table.
fn stored_schema_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |r| r.get(0))
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
    let current: i64 = stored_schema_version(conn).map_err(db_err)?;
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

    /// The app's own open path (this `open`) must refuse a database written
    /// by a newer build — same seatbelt the CLI gets.
    #[test]
    fn open_refuses_a_database_newer_than_this_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reviews.db");
        {
            let conn = open(&path).unwrap();
            conn.execute("INSERT INTO schema_migrations (version) VALUES (999)", [])
                .unwrap();
        }

        let err = open(&path).unwrap_err();
        assert!(err.contains("v999"), "{err}");
        assert!(err.contains(&format!("v{SCHEMA_VERSION}")), "{err}");

        // The refused file was not touched: the recorded version is intact.
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 999);
    }

    #[test]
    fn open_refuses_a_foreign_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE users (id INTEGER PRIMARY KEY);").unwrap();
        }

        let err = open(&path).unwrap_err();
        assert!(err.contains("not a Prologue reviews database"), "{err}");
        // No schema was created alongside the foreign tables.
        let conn = Connection::open(&path).unwrap();
        let reviews: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'reviews'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reviews, 0);
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
    fn v3_migration_applies_in_place_to_a_v2_database_with_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reviews.db");
        v1_database_with_rows(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[1]).unwrap();
            conn.execute("INSERT INTO schema_migrations (version) VALUES (2)", [])
                .unwrap();
        }

        let conn = open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        // Existing rows survive and are attributed to the reviewer.
        let (body, author): (String, String) = conn
            .query_row("SELECT body, author FROM comments WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(body, "existing note");
        assert_eq!(author, "reviewer");
    }

    #[test]
    fn v4_migration_applies_in_place_to_a_v3_database_with_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reviews.db");
        v1_database_with_rows(&path);
        {
            let conn = Connection::open(&path).unwrap();
            for (i, sql) in MIGRATIONS[1..3].iter().enumerate() {
                conn.execute_batch(sql).unwrap();
                conn.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [i as i64 + 2],
                )
                .unwrap();
            }
        }

        let conn = open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        // Existing rows survive and the new table is usable.
        let body: String = conn
            .query_row("SELECT body FROM comments WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(body, "existing note");
        conn.execute(
            "INSERT INTO reviewed_files (review_id, file_path, fingerprint)
             VALUES (1, 'src/a.rs', 'aaa:bbb:644')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn v5_migration_applies_in_place_to_a_v4_database_with_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reviews.db");
        v1_database_with_rows(&path);
        {
            let conn = Connection::open(&path).unwrap();
            for (i, sql) in MIGRATIONS[1..4].iter().enumerate() {
                conn.execute_batch(sql).unwrap();
                conn.execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    [i as i64 + 2],
                )
                .unwrap();
            }
        }

        let conn = open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        // Existing rows survive and the new tables are usable.
        let body: String = conn
            .query_row("SELECT body FROM comments WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(body, "existing note");
        conn.execute(
            "INSERT INTO guides (review_id, base_ref, head_ref, mode, fingerprint_json, model)
             VALUES (1, 'main', 'feature', 'committed', '{}', 'sonnet')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO guide_sections (guide_id, position, title, summary, files_json)
             VALUES (1, 0, 'Core', 'The change.', '[\"src/a.rs\"]')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn one_guide_per_review_base_head_and_mode() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("reviews.db")).unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES ('/repo', 'feature', 'main', 'committed')",
            [],
        )
        .unwrap();
        let insert = "INSERT INTO guides (review_id, base_ref, head_ref, mode, fingerprint_json, model)
                      VALUES (1, ?1, ?2, ?3, '{}', 'sonnet')";
        conn.execute(insert, ["main", "feature", "committed"]).unwrap();
        // A second guide at the same coordinates is rejected...
        assert!(conn.execute(insert, ["main", "feature", "committed"]).is_err());
        // ...but other modes and refs are fine.
        conn.execute(insert, ["main", "feature", "all"]).unwrap();
        conn.execute(insert, ["develop", "feature", "committed"]).unwrap();
    }

    #[test]
    fn deleting_a_review_cascades_to_its_guide_and_sections() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("reviews.db")).unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES ('/repo', 'feature', 'main', 'committed')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO guides (review_id, base_ref, head_ref, mode, fingerprint_json, model)
             VALUES (1, 'main', 'feature', 'committed', '{}', 'sonnet')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO guide_sections (guide_id, position, title, summary, files_json)
             VALUES (1, 0, 'Core', 'The change.', '[]')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM reviews WHERE id = 1", []).unwrap();
        let guides: i64 = conn
            .query_row("SELECT COUNT(*) FROM guides", [], |r| r.get(0))
            .unwrap();
        let sections: i64 = conn
            .query_row("SELECT COUNT(*) FROM guide_sections", [], |r| r.get(0))
            .unwrap();
        assert_eq!((guides, sections), (0, 0));
    }

    #[test]
    fn deleting_a_review_cascades_to_its_reviewed_files() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("reviews.db")).unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES ('/repo', 'feature', 'main', 'committed')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reviewed_files (review_id, file_path, fingerprint)
             VALUES (1, 'src/a.rs', 'aaa:bbb:644')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM reviews WHERE id = 1", []).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM reviewed_files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
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

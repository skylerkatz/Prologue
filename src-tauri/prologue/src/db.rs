//! Opening the reviews database: read-only for the query commands,
//! writable for comment/reply. The write path shares core's schema-version
//! seatbelt (`prologue_core::db::open`); the read path opens at the SQLite
//! level with `SQLITE_OPEN_READ_ONLY` and never migrates.

use prologue_core::db::{ensure_no_schema_sql, harden_connection, APP_IDENTIFIER, SCHEMA_VERSION};
use prologue_core::rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

/// The Prologue app's database (its Tauri app-data directory).
pub fn default_db_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support")
        .join(APP_IDENTIFIER)
        .join("reviews.db"))
}

/// Open `path` for reading, with `SQLITE_OPEN_READ_ONLY` — the file cannot
/// be modified, so a nominally-read command (`reviews`, `show`, …) can never
/// migrate an older database in place (e.g. one shared with a running older
/// app version). A schema-version mismatch in either direction is refused:
/// a newer database needs a newer binary; an older one is migrated by the
/// app or by a write command, never by a read.
pub fn open_reviews_db(path: &Path) -> Result<Connection, String> {
    ensure_exists(path)?;
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Failed to open review database: {e}"))?;
    // READ_ONLY stops writes, not SQL embedded in the file's schema — a
    // crafted --db carrying a malicious view would still run it on SELECT.
    // Same hardening as core's write path, from the same helper.
    harden_connection(&conn)?;
    // The app may hold the write lock briefly (WAL checkpoint); wait a
    // moment instead of failing immediately.
    conn.busy_timeout(std::time::Duration::from_millis(500))
        .map_err(|e| format!("Failed to set busy timeout: {e}"))?;
    check_readable_version(&conn, path)?;
    Ok(conn)
}

/// Open `path` for the comment/reply commands: core's shared seatbelt (a
/// newer schema is refused, an older one is migrated), and the connection
/// can write. Lifecycle state stays untouchable regardless — the CLI has no
/// commands that change it.
pub fn open_reviews_db_for_write(path: &Path) -> Result<Connection, String> {
    ensure_exists(path)?;
    prologue_core::db::open(path)
}

// The CLI never creates the database (core's open would) — a missing file
// means the app has not run yet.
fn ensure_exists(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!(
            "No reviews database at {} — launch the Prologue app once to create it.",
            path.display()
        ));
    }
    Ok(())
}

/// The read-path version seatbelt. Unlike core's (which migrates older
/// schemas in place), a read-only connection cannot migrate, so both
/// directions are refused with a pointer at what can.
fn check_readable_version(conn: &Connection, path: &Path) -> Result<(), String> {
    let db_err =
        |e: prologue_core::rusqlite::Error| format!("Failed to read {}: {e}", path.display());
    let has_migrations: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if has_migrations == 0 {
        return Err(format!("{} is not a Prologue reviews database", path.display()));
    }
    let version: i64 = conn
        .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |r| r.get(0))
        .map_err(db_err)?;
    if version > SCHEMA_VERSION {
        return Err(format!(
            "This reviews database is newer than this build (database schema v{version}, \
             this build knows v{SCHEMA_VERSION}) — update the Prologue app or rebuild prologue"
        ));
    }
    if version < SCHEMA_VERSION {
        return Err(format!(
            "This reviews database has an older schema (v{version}; this build reads \
             v{SCHEMA_VERSION}) — launch the Prologue app once to migrate it"
        ));
    }
    // Shared with core's seatbelt: a file carrying views or triggers is
    // running someone else's SQL and is refused outright.
    ensure_no_schema_sql(conn, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prologue_core::db::SCHEMA_VERSION;

    fn at_version_db(dir: &tempfile::TempDir) -> PathBuf {
        let path = dir.path().join("reviews.db");
        prologue_core::db::open(&path).unwrap();
        path
    }

    #[test]
    fn opens_an_at_version_database_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = at_version_db(&dir);

        let conn = open_reviews_db(&path).unwrap();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM reviews", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);

        // The connection refuses writes.
        let err = conn
            .execute(
                "INSERT INTO reviews (repo_path, branch, base_ref, mode)
                 VALUES ('/r', 'b', 'main', 'committed')",
                [],
            )
            .unwrap_err();
        assert!(err.to_string().contains("readonly"), "{err}");
    }

    #[test]
    fn the_write_open_shares_the_seatbelt_but_accepts_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = at_version_db(&dir);

        let conn = open_reviews_db_for_write(&path).unwrap();
        conn.execute(
            "INSERT INTO reviews (repo_path, branch, base_ref, mode)
             VALUES ('/r', 'b', 'main', 'committed')",
            [],
        )
        .unwrap();
        drop(conn);

        // A too-new database is refused on the write path too.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute("INSERT INTO schema_migrations (version) VALUES (999)", []).unwrap();
        }
        let err = open_reviews_db_for_write(&path).unwrap_err();
        assert!(err.contains("rebuild prologue"), "{err}");
    }

    #[test]
    fn refuses_a_database_newer_than_this_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = at_version_db(&dir);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute("INSERT INTO schema_migrations (version) VALUES (999)", []).unwrap();
        }

        let err = open_reviews_db(&path).unwrap_err();
        assert!(err.contains("rebuild prologue"), "{err}");
        assert!(err.contains("v999"), "{err}");
    }

    #[test]
    fn the_read_path_refuses_an_older_database_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reviews.db");
        // A v1-era database: only the first migration applied.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at TEXT NOT NULL DEFAULT ''
                 );
                 CREATE TABLE reviews (id INTEGER PRIMARY KEY);
                 CREATE TABLE comments (id INTEGER PRIMARY KEY);
                 INSERT INTO schema_migrations (version) VALUES (1);",
            )
            .unwrap();
        }

        // A read command must not silently upgrade a database it merely
        // wanted to look at (it may belong to an older app version).
        let err = open_reviews_db(&path).unwrap_err();
        assert!(err.contains("older schema"), "{err}");
        assert!(err.contains("launch the Prologue app"), "{err}");

        // The refused file was not touched: still v1.
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
        drop(conn);

        // The write path keeps migrating — core's tests cover the shape.
        open_reviews_db_for_write(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    /// F5's exact attack vector: a crafted --db that passes the version
    /// seatbelt but replaces a queried table with a VIEW carrying foreign
    /// SQL. READ_ONLY does not stop view SQL from running on SELECT, so the
    /// file must be refused before any query touches it.
    #[test]
    fn the_read_path_refuses_a_crafted_database_with_a_reviews_view() {
        let dir = tempfile::tempdir().unwrap();
        let path = at_version_db(&dir);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "ALTER TABLE reviews RENAME TO reviews_real;
                 CREATE VIEW reviews AS SELECT * FROM reviews_real;",
            )
            .unwrap();
        }

        let err = open_reviews_db(&path).unwrap_err();
        assert!(err.contains("not a Prologue reviews database"), "{err}");
    }

    /// The read connection gets the same session hardening as the write
    /// path (trusted_schema off, defensive mode) — shared helper, no drift.
    #[test]
    fn the_read_connection_is_hardened_like_the_write_one() {
        use prologue_core::rusqlite::config::DbConfig;

        let dir = tempfile::tempdir().unwrap();
        let path = at_version_db(&dir);

        for conn in [open_reviews_db(&path).unwrap(), open_reviews_db_for_write(&path).unwrap()] {
            let trusted: i64 =
                conn.pragma_query_value(None, "trusted_schema", |r| r.get(0)).unwrap();
            assert_eq!(trusted, 0);
            assert!(conn.db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE).unwrap());
        }
    }

    #[test]
    fn rejects_a_missing_file_and_a_foreign_database() {
        let dir = tempfile::tempdir().unwrap();

        let err = open_reviews_db(&dir.path().join("nope.db")).unwrap_err();
        assert!(err.contains("No reviews database"), "{err}");

        let foreign = dir.path().join("other.db");
        {
            let conn = Connection::open(&foreign).unwrap();
            conn.execute_batch("CREATE TABLE users (id INTEGER PRIMARY KEY);").unwrap();
        }
        let err = open_reviews_db(&foreign).unwrap_err();
        assert!(err.contains("not a Prologue reviews database"), "{err}");
    }
}

//! Opening the reviews database read-only. The schema-version seatbelt
//! lives in `prologue_core::db::open`, shared with the app.

use prologue_core::db::APP_IDENTIFIER;
use prologue_core::rusqlite::Connection;
use std::path::{Path, PathBuf};

/// The Prologue app's database (its Tauri app-data directory).
pub fn default_db_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support")
        .join(APP_IDENTIFIER)
        .join("reviews.db"))
}

/// Open `path` for reading. Core's open runs the shared schema seatbelt
/// first: a database newer than this binary is refused, an older one is
/// migrated by the same shared migrations the app runs. The returned
/// connection is `query_only` — writes fail at the SQLite level.
pub fn open_reviews_db(path: &Path) -> Result<Connection, String> {
    let conn = open_checked(path)?;
    conn.pragma_update(None, "query_only", "ON")
        .map_err(|e| format!("Failed to make the connection read-only: {e}"))?;
    Ok(conn)
}

/// Open `path` for the comment/reply commands: same seatbelt, but the
/// connection can write. Lifecycle state stays untouchable regardless — the
/// CLI has no commands that change it.
pub fn open_reviews_db_for_write(path: &Path) -> Result<Connection, String> {
    open_checked(path)
}

fn open_checked(path: &Path) -> Result<Connection, String> {
    // The CLI never creates the database (core's open would) — a missing
    // file means the app has not run yet.
    if !path.is_file() {
        return Err(format!(
            "No reviews database at {} — launch the Prologue app once to create it.",
            path.display()
        ));
    }
    prologue_core::db::open(path)
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
    fn migrates_an_older_database_in_place() {
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

        // Not asserting the migrated shape here (core's tests cover it) —
        // just that an older version passes the seatbelt and is brought up
        // to the current version.
        open_reviews_db(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
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

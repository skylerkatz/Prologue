//! Opening the reviews database read-only, with the schema-version seatbelt.

use prologue_core::db::SCHEMA_VERSION;
use prologue_core::rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

/// The Prologue app's database (its Tauri app-data directory).
pub fn default_db_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support/com.skylerkatz.prologue/reviews.db"))
}

/// Open `path` for reading. The schema version is checked before anything
/// else touches the file: a database newer than this binary is refused, an
/// older one is migrated by the same shared migrations the app runs. The
/// returned connection is `query_only` — writes fail at the SQLite level.
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
    if !path.is_file() {
        return Err(format!(
            "No reviews database at {} — launch the Prologue app once to create it.",
            path.display()
        ));
    }
    check_schema_version(path)?;
    prologue_core::db::open(path)
}

/// Inspect the recorded schema version over a read-only connection (zero
/// side effects) and refuse anything this binary is too old to understand.
fn check_schema_version(path: &Path) -> Result<(), String> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Failed to open review database: {e}"))?;
    let db_err = |e: prologue_core::rusqlite::Error| format!("Failed to read {}: {e}", path.display());

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

    let version: i64 = conn
        .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |r| r.get(0))
        .map_err(db_err)?;
    if version > SCHEMA_VERSION {
        return Err(format!(
            "prologue is older than your reviews.db (database schema v{version}, \
             this prologue knows v{SCHEMA_VERSION}) — rebuild prologue"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

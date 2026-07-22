//! Review, diff, and persistence core shared by the Prologue app and the
//! `prologue` CLI. No Tauri types anywhere in this crate's dependency
//! tree.

pub mod anchor;
pub mod comment;
pub mod db;
pub mod diff;
pub mod error;
pub mod export;
pub mod guide;
pub mod intraline;
pub mod repo;
pub mod review;
// Test-only git fixtures; the `testutil` feature lets sibling crates (the
// CLI) use them from their dev-dependencies. Never enabled in normal builds.
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;
// TS binding generation for the IPC types; test builds only (the `ts`
// feature comes from the self dev-dependency).
#[cfg(all(test, feature = "ts"))]
mod ts_export;

// Callers hold the database connection themselves (the app wraps it in
// Tauri-managed state); re-export rusqlite so they name the same version
// this crate was built against.
pub use rusqlite;

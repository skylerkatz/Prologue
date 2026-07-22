//! Review, diff, and persistence core shared by the Prologue app and any
//! external binaries (CLI, MCP server). No Tauri types anywhere in this
//! crate's dependency tree.

pub mod anchor;
pub mod db;
pub mod diff;
pub mod error;
pub mod export;
pub mod intraline;
pub mod repo;
pub mod review;
// Test-only git fixtures; the `testutil` feature lets sibling crates (the
// CLI) use them from their dev-dependencies. Never enabled in normal builds.
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

// Callers hold the database connection themselves (the app wraps it in
// Tauri-managed state); re-export rusqlite so they name the same version
// this crate was built against.
pub use rusqlite;

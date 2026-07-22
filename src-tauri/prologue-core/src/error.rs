//! Typed errors for the failure modes callers branch on.
//!
//! Core APIs historically returned `Result<_, String>`, and the interesting
//! failures were detected by matching message text — which made the exact
//! wording load-bearing. `CoreError` gives those cases variants to match on;
//! `Display` renders the exact historical strings, so the IPC and CLI
//! surfaces are unchanged.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// The requested file has no delta in the current diff — re-anchoring
    /// treats this as "the file left the diff", not a failure.
    NoChangesForFile(String),
    /// A line-comment selection spans more than one hunk.
    SelectionCrossesHunks,
    /// The selected range has no diff lines on the requested side.
    NoDiffLines {
        path: String,
        start: u32,
        end: u32,
        side: &'static str,
    },
    /// Any other failure, carrying the already-formatted message.
    Other(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::NoChangesForFile(path) => write!(f, "No changes for file: {path}"),
            CoreError::SelectionCrossesHunks => {
                f.write_str("A comment selection cannot cross hunk boundaries")
            }
            CoreError::NoDiffLines { path, start, end, side } => {
                write!(f, "No diff lines at {path}:{start}-{end} ({side}) to comment on")
            }
            CoreError::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CoreError {}

/// Lets `?` lift the crate's untyped `String` errors into typed contexts.
impl From<String> for CoreError {
    fn from(message: String) -> Self {
        CoreError::Other(message)
    }
}

/// Lets `?` lift `&str` errors (`ok_or("…")`) into typed contexts.
impl From<&str> for CoreError {
    fn from(message: &str) -> Self {
        CoreError::Other(message.to_owned())
    }
}

/// Collapse back to the string surface at the IPC/CLI boundary.
impl From<CoreError> for String {
    fn from(err: CoreError) -> Self {
        err.to_string()
    }
}

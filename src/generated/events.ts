// Generated from src-tauri/src/events.rs by its events test.
// Do not edit — run `cargo test` in src-tauri to refresh.

/** Debounced repo activity; payload is the watched repoPath string. */
export const REPO_CHANGED_EVENT = "repo-changed";
/** An external connection (the prologue CLI) committed to reviews.db. */
export const COMMENTS_CHANGED_EVENT = "comments-changed";
/** View > Archived Reviews… chosen. */
export const MENU_VIEW_ARCHIVED_EVENT = "menu-view-archived";
/** View > Refresh chosen. */
export const MENU_REFRESH_EVENT = "menu-refresh";
/** View > Hide Resolved Comments toggled; payload is the new checked state. */
export const MENU_HIDE_RESOLVED_EVENT = "menu-hide-resolved";

//! Event names shared with the frontend — the single Rust source of truth.
//! The test below regenerates `src/generated/events.ts` from these consts
//! and fails while it is stale, so a rename here cannot silently diverge
//! from what the frontend listens for.

/// Debounced repo activity; payload is the watched repo_path string.
pub const REPO_CHANGED: &str = "repo-changed";
/// An external connection (the prologue CLI) committed to reviews.db.
pub const COMMENTS_CHANGED: &str = "comments-changed";
/// View > Archived Reviews… chosen.
pub const MENU_VIEW_ARCHIVED: &str = "menu-view-archived";
/// View > Refresh chosen.
pub const MENU_REFRESH: &str = "menu-refresh";
/// View > Hide Resolved Comments toggled; payload is the new checked state.
pub const MENU_HIDE_RESOLVED: &str = "menu-hide-resolved";
/// Help > Keyboard Shortcuts chosen.
pub const MENU_SHOW_SHORTCUTS: &str = "menu-show-shortcuts";
/// Prologue > Check for Updates… chosen.
pub const MENU_CHECK_UPDATES: &str = "menu-check-updates";
/// Prologue > What's New… chosen.
pub const MENU_WHATS_NEW: &str = "menu-whats-new";

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn generated_events_module_is_current() {
        let expected = format!(
            "// Generated from src-tauri/src/events.rs by its events test.\n\
             // Do not edit — run `cargo test` in src-tauri to refresh.\n\
             \n\
             /** Debounced repo activity; payload is the watched repoPath string. */\n\
             export const REPO_CHANGED_EVENT = \"{repo_changed}\";\n\
             /** An external connection (the prologue CLI) committed to reviews.db. */\n\
             export const COMMENTS_CHANGED_EVENT = \"{comments_changed}\";\n\
             /** View > Archived Reviews… chosen. */\n\
             export const MENU_VIEW_ARCHIVED_EVENT = \"{menu_view_archived}\";\n\
             /** View > Refresh chosen. */\n\
             export const MENU_REFRESH_EVENT = \"{menu_refresh}\";\n\
             /** View > Hide Resolved Comments toggled; payload is the new checked state. */\n\
             export const MENU_HIDE_RESOLVED_EVENT = \"{menu_hide_resolved}\";\n\
             /** Help > Keyboard Shortcuts chosen. */\n\
             export const MENU_SHOW_SHORTCUTS_EVENT = \"{menu_show_shortcuts}\";\n\
             /** Prologue > Check for Updates… chosen. */\n\
             export const MENU_CHECK_UPDATES_EVENT = \"{menu_check_updates}\";\n\
             /** Prologue > What's New… chosen. */\n\
             export const MENU_WHATS_NEW_EVENT = \"{menu_whats_new}\";\n",
            repo_changed = super::REPO_CHANGED,
            comments_changed = super::COMMENTS_CHANGED,
            menu_view_archived = super::MENU_VIEW_ARCHIVED,
            menu_refresh = super::MENU_REFRESH,
            menu_hide_resolved = super::MENU_HIDE_RESOLVED,
            menu_show_shortcuts = super::MENU_SHOW_SHORTCUTS,
            menu_check_updates = super::MENU_CHECK_UPDATES,
            menu_whats_new = super::MENU_WHATS_NEW,
        );
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/generated/events.ts");
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != expected {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &expected).unwrap();
            panic!(
                "src/generated/events.ts was out of date with events.rs — \
                 regenerated it; review and commit the update, then re-run"
            );
        }
    }
}

mod cli_install;
mod commands;
mod events;
mod guide_engine;
mod watcher;

use prologue_core::db;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

/// Menu item id for the View > Archived Reviews… entry.
const MENU_VIEW_ARCHIVED_ID: &str = "view-archived";
/// Menu item id for the View > Refresh entry.
const MENU_REFRESH_ID: &str = "view-refresh";
/// Menu item id for the View > Hide Resolved Comments check item.
const MENU_HIDE_RESOLVED_ID: &str = "view-hide-resolved";
/// Menu item id for the Help > Keyboard Shortcuts entry.
const MENU_SHOW_SHORTCUTS_ID: &str = "help-shortcuts";
/// Menu item id for the Prologue > Check for Updates… entry.
const MENU_CHECK_UPDATES_ID: &str = "check-updates";

/// Handles to the View and Help menu items that only make sense with a
/// repo open; the frontend enables them on repo open and disables them on
/// the welcome screen.
struct RepoMenuItems {
    refresh: tauri::menu::MenuItem<tauri::Wry>,
    archived: tauri::menu::MenuItem<tauri::Wry>,
    hide_resolved: tauri::menu::CheckMenuItem<tauri::Wry>,
    shortcuts: tauri::menu::MenuItem<tauri::Wry>,
}

#[tauri::command]
fn set_repo_menu_enabled(app: tauri::AppHandle, enabled: bool) {
    if let Some(items) = app.try_state::<RepoMenuItems>() {
        let _ = items.refresh.set_enabled(enabled);
        let _ = items.archived.set_enabled(enabled);
        let _ = items.hide_resolved.set_enabled(enabled);
        let _ = items.shortcuts.set_enabled(enabled);
    }
}

/// Mirror the stored hide-resolved preference onto the menu check mark.
/// The frontend owns the preference (localStorage); this is the JS→menu leg.
#[tauri::command]
fn set_hide_resolved_checked(app: tauri::AppHandle, checked: bool) {
    if let Some(items) = app.try_state::<RepoMenuItems>() {
        let _ = items.hide_resolved.set_checked(checked);
    }
}

/// Customize the default menu: "Install 'prologue' Command Line Tool…" in
/// the app submenu after About, "Refresh" / "Archived Reviews…" / "Hide
/// Resolved Comments" in View, and "Keyboard Shortcuts" in Help.
fn setup_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, HELP_SUBMENU_ID};

    let menu = Menu::default(app.handle())?;
    let items = menu.items()?;
    if let Some(app_submenu) = items.first().and_then(|i| i.as_submenu()) {
        // Directly under About, the conventional macOS spot (Sparkle's).
        // Always enabled: checking makes sense on the welcome screen too.
        let check_updates = MenuItem::with_id(
            app,
            MENU_CHECK_UPDATES_ID,
            "Check for Updates…",
            true,
            None::<&str>,
        )?;
        app_submenu.insert(&check_updates, 1)?;
        let item = MenuItem::with_id(
            app,
            "install-cli",
            format!("Install '{}' Command Line Tool…", cli_install::CLI_NAME),
            true,
            None::<&str>,
        )?;
        app_submenu.insert(&item, 2)?;
    }
    // Help > Keyboard Shortcuts opens the same overlay as the in-app `?`
    // key. No accelerator: a menu accelerator would swallow the keystroke
    // even while typing in a composer; the webview handler suppresses that
    // itself. Starts disabled like the View items — the overlay lives in
    // the review screen.
    let shortcuts = MenuItem::with_id(
        app,
        MENU_SHOW_SHORTCUTS_ID,
        "Keyboard Shortcuts",
        false,
        None::<&str>,
    )?;
    if let Some(help_submenu) = items
        .iter()
        .filter_map(|i| i.as_submenu())
        .find(|s| s.id().as_ref() == HELP_SUBMENU_ID)
    {
        help_submenu.append(&shortcuts)?;
    }
    // Locate View by title — Menu::default gives it no stable id.
    if let Some(view_submenu) = items
        .iter()
        .filter_map(|i| i.as_submenu())
        .find(|s| s.text().is_ok_and(|t| t == "View"))
    {
        // Both start disabled until the frontend reports an open repo —
        // neither action means anything on the welcome screen.
        let refresh = MenuItem::with_id(
            app,
            MENU_REFRESH_ID,
            "Refresh",
            false,
            Some("CmdOrCtrl+R"),
        )?;
        let archived = MenuItem::with_id(
            app,
            MENU_VIEW_ARCHIVED_ID,
            "Archived Reviews…",
            false,
            Some("CmdOrCtrl+Shift+A"),
        )?;
        // Starts unchecked as well; the frontend pushes the stored
        // preference (and enables on repo open) before it is clickable.
        let hide_resolved = CheckMenuItem::with_id(
            app,
            MENU_HIDE_RESOLVED_ID,
            "Hide Resolved Comments",
            false,
            false,
            Some("CmdOrCtrl+Shift+H"),
        )?;
        // "Enter Full Screen" conventionally stays at the bottom.
        view_submenu.insert(&refresh, 0)?;
        view_submenu.insert(&archived, 1)?;
        view_submenu.insert(&hide_resolved, 2)?;
        view_submenu.insert(&PredefinedMenuItem::separator(app)?, 3)?;
        app.manage(RepoMenuItems {
            refresh,
            archived,
            hide_resolved,
            shortcuts,
        });
    }
    app.set_menu(menu)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        // Must stay the first registered plugin so a second launch bails out
        // before setup() opens reviews.db — two instances would otherwise
        // share the same SQLite database.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Reviews and comments live in the app data dir — never inside
            // the reviewed repository.
            let dir = app.path().app_data_dir()?;
            // tauri.conf.json's `identifier` decides this directory; the CLI
            // reaches the same file via db::APP_IDENTIFIER. A mismatch would
            // silently split the app and CLI onto two databases.
            debug_assert!(
                dir.ends_with(db::APP_IDENTIFIER),
                "app_data_dir {dir:?} does not end with db::APP_IDENTIFIER \
                 ({}) — tauri.conf.json and prologue-core disagree",
                db::APP_IDENTIFIER
            );
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("reviews.db"))?;
            app.manage(db::Db(Mutex::new(conn)));
            app.manage(watcher::RepoWatcher::default());
            app.manage(guide_engine::GuideRuntime::default());
            // External writers (the prologue CLI) commit to reviews.db
            // behind the app's back; surface them as `comments-changed`.
            watcher::start_db_watching(app.handle().clone(), dir)?;
            setup_menu(app)?;
            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "install-cli" => {
                use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                let (kind, text) = match cli_install::install_cli(None) {
                    Ok(report) => (MessageDialogKind::Info, report.message),
                    Err(e) => (MessageDialogKind::Error, format!("Install failed: {e}")),
                };
                app.dialog()
                    .message(text)
                    .title("Install Command Line Tool")
                    .kind(kind)
                    .show(|_| {});
            }
            MENU_VIEW_ARCHIVED_ID => {
                // ReviewShell listens; with no repo open nothing is mounted
                // and the event is a deliberate no-op.
                let _ = app.emit(events::MENU_VIEW_ARCHIVED, ());
            }
            MENU_REFRESH_ID => {
                let _ = app.emit(events::MENU_REFRESH, ());
            }
            MENU_SHOW_SHORTCUTS_ID => {
                let _ = app.emit(events::MENU_SHOW_SHORTCUTS, ());
            }
            MENU_CHECK_UPDATES_ID => {
                let _ = app.emit(events::MENU_CHECK_UPDATES, ());
            }
            MENU_HIDE_RESOLVED_ID => {
                // macOS auto-toggles the check state before the event
                // fires, so is_checked() is already the new value.
                if let Some(items) = app.try_state::<RepoMenuItems>() {
                    if let Ok(checked) = items.hide_resolved.is_checked() {
                        let _ = app.emit(events::MENU_HIDE_RESOLVED, checked);
                    }
                }
            }
            _ => {}
        });

    // Agent automation bridge (WebSocket on 127.0.0.1:9223+). Debug builds
    // with the `mcp-bridge` feature only; the plugin's default bind is
    // 0.0.0.0, so pin it to localhost.
    #[cfg(all(debug_assertions, feature = "mcp-bridge"))]
    let builder = builder.plugin(
        tauri_plugin_mcp_bridge::Builder::new()
            .bind_address("127.0.0.1")
            .build(),
    );

    builder
        .invoke_handler(tauri::generate_handler![
            commands::open_repo,
            commands::list_branches,
            commands::get_diff_summary,
            commands::get_file_diff,
            commands::get_context_lines,
            commands::open_review,
            commands::find_active_review,
            commands::list_reviewed_files,
            commands::mark_file_reviewed,
            commands::unmark_file_reviewed,
            commands::list_comments,
            commands::create_comment,
            commands::update_comment,
            commands::delete_comment,
            commands::update_comment_state,
            commands::reanchor_comments,
            commands::archive_stale_reviews,
            commands::list_archived_reviews,
            commands::export_review,
            commands::find_guide,
            guide_engine::generate_guide,
            guide_engine::cancel_guide,
            guide_engine::guide_cli_status,
            cli_install::install_cli,
            watcher::start_watching,
            watcher::stop_watching,
            set_repo_menu_enabled,
            set_hide_resolved_checked
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    /// tauri.conf.json's `identifier` and core's APP_IDENTIFIER must name
    /// the same directory — a mismatch silently splits the app and CLI onto
    /// two databases. The startup debug_assert catches it at runtime; this
    /// catches it in CI without launching the app.
    #[test]
    fn bundle_identifier_matches_the_shared_constant() {
        let conf = include_str!("../tauri.conf.json");
        let expected = format!("\"identifier\": \"{}\"", prologue_core::db::APP_IDENTIFIER);
        assert!(
            conf.contains(&expected),
            "tauri.conf.json does not declare {expected} — keep it in sync \
             with prologue_core::db::APP_IDENTIFIER"
        );
    }
}

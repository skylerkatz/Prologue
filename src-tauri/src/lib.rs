mod cli_install;
mod commands;
mod watcher;

use prologue_core::db;
use std::sync::Mutex;
use tauri::Manager;

/// Add "Install 'prologue' Command Line Tool…" to the app submenu, after
/// the About item.
fn add_install_cli_menu_item(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};

    let menu = Menu::default(app.handle())?;
    if let Some(app_submenu) = menu.items()?.first().and_then(|i| i.as_submenu().cloned()) {
        let item = MenuItem::with_id(
            app,
            "install-cli",
            format!("Install '{}' Command Line Tool…", cli_install::CLI_NAME),
            true,
            None::<&str>,
        )?;
        app_submenu.insert(&item, 1)?;
    }
    app.set_menu(menu)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .setup(|app| {
            // Reviews and comments live in the app data dir — never inside
            // the reviewed repository.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("reviews.db"))?;
            app.manage(db::Db(Mutex::new(conn)));
            app.manage(watcher::RepoWatcher::default());
            // External writers (the prologue CLI) commit to reviews.db
            // behind the app's back; surface them as `comments-changed`.
            watcher::start_db_watching(app.handle().clone(), dir)?;
            add_install_cli_menu_item(app)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "install-cli" {
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
            commands::list_comments,
            commands::create_comment,
            commands::update_comment,
            commands::delete_comment,
            commands::update_comment_state,
            commands::reanchor_comments,
            commands::archive_stale_reviews,
            commands::list_archived_reviews,
            commands::export_review,
            cli_install::install_cli,
            watcher::start_watching,
            watcher::stop_watching
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

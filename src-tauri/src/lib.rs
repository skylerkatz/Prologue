mod anchor;
mod db;
mod diff;
mod export;
mod repo;
mod review;
#[cfg(test)]
mod testutil;

use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .setup(|app| {
            // Reviews and comments live in the app data dir — never inside
            // the reviewed repository.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("reviews.db"))?;
            app.manage(db::Db(Mutex::new(conn)));
            Ok(())
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
            repo::open_repo,
            repo::list_branches,
            diff::get_diff_summary,
            diff::get_file_diff,
            diff::get_context_lines,
            review::open_review,
            review::list_comments,
            review::create_comment,
            review::update_comment,
            review::delete_comment,
            review::update_comment_state,
            review::reanchor_comments,
            review::archive_stale_reviews,
            review::list_archived_reviews,
            export::export_review
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

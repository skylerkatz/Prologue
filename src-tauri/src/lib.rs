mod diff;
mod repo;
#[cfg(test)]
mod testutil;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build());

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
            diff::get_context_lines
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

//! What the Prologue hosts — the Tauri app and the headless web server —
//! share beyond `prologue-core`: the guide subprocess engine, the repo and
//! database watchers, and the event names the frontend listens for. Each
//! host only adapts these to its transport (Tauri commands/events, or
//! HTTP/SSE), so nothing here may depend on Tauri or a display.

pub mod events;
pub mod guide_engine;
pub mod watcher;

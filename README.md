# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## prologue CLI

The `prologue` CLI ships inside the app bundle as a Tauri sidecar
(`Contents/MacOS/prologue`, signed with the bundle). `scripts/build-sidecar.sh`
builds it and stages it at `src-tauri/binaries/prologue-<target-triple>`; the
tauri CLI runs that script automatically before `dev` and `build`. Plain
`cargo build`/`cargo test` runs don't stage it — `src-tauri/build.rs` drops an
empty placeholder there so those still compile.

**Install CLI**: the app-menu item "Install 'prologue' Command Line Tool…"
symlinks the bundled binary onto your PATH (`/usr/local/bin/prologue` when
writable, else `~/.local/bin` plus a PATH hint). A symlink, not a copy: app
updates propagate to the CLI automatically. The action is a no-op with a
friendly message unless the app runs from `/Applications` (a link into a
Gatekeeper-translocated or build-directory path would dangle). For development
and testing, the underlying `install_cli` command accepts `force: true` to
skip that guard — e.g. from the dev tools console:
`window.__TAURI__.core.invoke('install_cli', { force: true })`.

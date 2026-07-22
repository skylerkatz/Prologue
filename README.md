# Prologue

Prologue is a macOS app for reviewing local git diffs: pick a repository and a
base branch, read the diff with syntax highlighting, and leave review comments
that live in a local SQLite database (never inside the reviewed repository).
Reviews can be exported as prompts for coding agents, and the bundled
`prologue` CLI lets agents read and reply to comments from the terminal.

Built with Tauri 2, React, and TypeScript.

## Development

- `npm run tauri dev` — run the app (builds the CLI sidecar first)
- `npm run dev:bridge` — run the app with the MCP bridge: `--features mcp-bridge`
  compiles the bridge plugin and `--config src-tauri/tauri.mcp-bridge.conf.json`
  grants its capability. The two flags must travel together — one without the
  other yields a capability referencing an uncompiled plugin, or a bridge with
  no permission. `solo.yml`'s Tauri Development process uses this script.
- `npm run build` — type-check + production frontend build. The script pins
  `NODE_ENV=production` because a shell exporting `NODE_ENV=development` would
  otherwise make `vite build` ship the development React build.
- `npm run test` — frontend unit tests (Vitest, no DOM environment) covering
  the diff row model, intraline segments, file guards, recents, and pure
  helpers; Tauri plugins are mocked.
- `cargo test` / `cargo clippy --all-targets` from `src-tauri/` — Rust checks
  across the workspace (app, `prologue-core`, `prologue`)

App data (reviews.db) lives in
`~/Library/Application Support/com.skylerkatz.prologue/`; the app creates it
on first launch.

## Signing

Bundles are ad-hoc signed (`signingIdentity: "-"` in `tauri.conf.json`): fine
for local use, but Gatekeeper will block downloads on other machines. Before
distributing publicly, swap in a Developer ID Application certificate and
notarize.

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
`window.__TAURI__.core.invoke('install_cli', { force: true })`. Note the
`window.__TAURI__` global only exists under the mcp-bridge config
(`withGlobalTauri` is set only in `tauri.mcp-bridge.conf.json`), so run the
app via `npm run dev:bridge` to use this tip.

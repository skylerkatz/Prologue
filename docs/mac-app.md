# Prologue for macOS

Prologue also ships as a native macOS app (Tauri 2) that reviews any local
repository: pick a repository and a base branch, read the diff, and leave
comments. It shares the review engine, UI, and `prologue` CLI with the Amp
plugin's web mode. Download the `.dmg` from the
[releases page](https://github.com/skylerkatz/prologue/releases); the app
updates itself.

App data (reviews.db) lives in
`~/Library/Application Support/com.skylerkatz.prologue/`; the app creates it
on first launch.

## Development

- `npm run tauri dev` — run the app (builds the CLI sidecar first)
- `npm run dev:bridge` — run the app with the MCP bridge: `--features mcp-bridge`
  compiles the bridge plugin and `--config src-tauri/tauri.mcp-bridge.conf.json`
  grants its capability. The two flags must travel together — one without the
  other yields a capability referencing an uncompiled plugin, or a bridge with
  no permission. `solo.yml`'s Tauri Development process uses this script.

The frontend and Rust checks (`npm run build`, `npm test`, `cargo test`)
are in the [README](../README.md#development).

## Signing

Bundles are ad-hoc signed (`signingIdentity: "-"` in `tauri.conf.json`): fine
for local use, but Gatekeeper will block downloads on other machines. Before
distributing publicly, swap in a Developer ID Application certificate and
notarize.

## prologue CLI

The `prologue` CLI ships inside the app bundle as a Tauri sidecar
(`Contents/MacOS/prologue-cli`, signed with the bundle). The bundled name
must differ from the main app binary (`Prologue`) in more than case: on the
default case-insensitive APFS, a sidecar named `prologue` and the app binary
collapse into one file and the CLI silently becomes the GUI.
`scripts/build-sidecar.sh`
builds it and stages it at `src-tauri/binaries/prologue-cli-<target-triple>`; the
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

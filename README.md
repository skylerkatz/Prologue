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

## Web mode (headless server)

`prologue-server` runs the same UI and review logic in a browser, with no
Tauri and no display — e.g. inside a headless Linux Amp orb, opened
through an Amp portal. The Mac app is unaffected.

```sh
npm ci && npm run build                        # frontend → dist/
cd src-tauri && cargo build --release -p prologue-server -p prologue
cd .. && PORT=8080 src-tauri/target/release/prologue-server \
  --repo /path/to/repo --base main             # base is optional
```

- **Prebuilt:** every `v*` tag's GitHub release carries
  `prologue-web-x86_64-linux.tar.gz` (built by
  `.github/workflows/release-web.yml`): `prologue-server`, `prologue`, and
  `dist/` side by side. Unpack it anywhere and run
  `./prologue-server --repo …` — the server finds the `dist/` next to it.
- **Config:** `--repo` / `PROLOGUE_REPO` (opened on page load), `--base` /
  `PROLOGUE_BASE` (default: auto-detected, as in the app; a branch's stored
  review base still wins), `--port` / `PORT` (default 8080), `--host` /
  `PROLOGUE_HOST` (default 0.0.0.0), `--dist` / `PROLOGUE_DIST` (default:
  `dist` next to the binary, else `./dist`), `--data-dir` /
  `PROLOGUE_DATA_DIR` (where reviews.db lives).
- **Send to Amp:** with `PROLOGUE_AMP_WEBHOOK_URL` set (and `AMP_THREAD_ID`
  naming the thread), Export becomes "Send to Amp (N)", N being the open
  comments not sent yet. The server — never the browser; the URL is a
  credential — POSTs `{"threadID", "commentIds": ["C12", …], "count",
  "repo", "base"}`, ids only, no comment text. Sent ids are kept in
  `amp-sent.json` next to reviews.db; unsent open comments show a
  "pending" badge. The agent then reads them with
  `prologue show --open --json` (see below).
- **In an Amp orb**, run it as a supervised service with a portal (paths
  absolute — the service's working directory may differ):
  `amp orb service start prologue --portal --command
  "$PWD/src-tauri/target/release/prologue-server --repo /path/to/repo
  --dist $PWD/dist"`.
- **Transport:** `src/ipc.ts` uses Tauri `invoke` inside the app and
  `POST /api/invoke/<command>` (same command names and args) in a browser.
  Backend events (`repo-changed`, `comments-changed`) arrive over
  Server-Sent Events at `/api/events`; the client reconnects with backoff
  when the stream drops (Amp redeploys close it) and refreshes on
  reconnect, since events may have been missed meanwhile.
- **The `prologue` CLI** works unchanged on the same machine: the server
  defaults to the app data directory the CLI reads
  (`$XDG_DATA_HOME/com.skylerkatz.prologue`, default
  `~/.local/share/…` on Linux), so `prologue show` / `prologue reply`
  run from inside the repo need no `--db`. Replies appear in the browser
  live, via `comments-changed`. With a custom `--data-dir`, pass the CLI
  `--db <dir>/reviews.db`. For an agent's work list, `prologue show --open
  --json` gives every open thread with ids, file and line range, the code
  anchor, body, replies, `orphaned`, and `changed` (code changed since it
  was commented); `prologue reply C12 --body "…"` answers one. Resolving
  stays the reviewer's act, in the UI.
- **Degraded in the browser:** Export copies to the clipboard where the
  browser allows it and otherwise downloads the text as a file (export
  delivery lives in `deliverExport` in `src/platform.ts` — the place to add
  a webhook POST). Open Repository asks for a path on the server instead
  of showing a file picker. There is no native menu: Refresh is automatic,
  ⌘/Ctrl+⇧+H toggles resolved comments, ⌘/Ctrl+⇧+A opens archived
  reviews, and `?` shows shortcuts. Install CLI and the updater are Mac-app
  only. Guide generation works if the `claude` CLI is installed and logged
  in on the server.
- **No authentication** — anyone who can reach the port can read and write
  reviews. Expose it only through an authenticated proxy such as an Amp
  portal, or bind `--host 127.0.0.1`.

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

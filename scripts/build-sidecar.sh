#!/usr/bin/env bash
# Build the prologue CLI and stage it where Tauri's bundler expects external
# binaries: src-tauri/binaries/prologue-cli-<target-triple>. The bundler
# strips the triple suffix, producing Contents/MacOS/prologue-cli inside the
# .app. The name must differ from the main app binary (Prologue) in more than
# case: on the default case-insensitive APFS they'd be the same file and one
# would silently overwrite the other in the bundle.
set -euo pipefail

cd "$(dirname "$0")/.."

# Non-interactive shells on this machine don't have cargo on PATH.
command -v cargo >/dev/null 2>&1 || export PATH="$HOME/.cargo/bin:$PATH"

TRIPLE="${TAURI_ENV_TARGET_TRIPLE:-$(rustc --print host-tuple)}"

cargo build --release -p prologue --manifest-path src-tauri/Cargo.toml

mkdir -p src-tauri/binaries
# Drop the pre-rename staging name so stale copies can't be bundled.
rm -f "src-tauri/binaries/prologue-$TRIPLE"
# rm first: cp onto an existing file (e.g. build.rs's non-executable
# placeholder) keeps the destination's permissions.
rm -f "src-tauri/binaries/prologue-cli-$TRIPLE"
cp src-tauri/target/release/prologue "src-tauri/binaries/prologue-cli-$TRIPLE"
chmod 755 "src-tauri/binaries/prologue-cli-$TRIPLE"
echo "sidecar staged: src-tauri/binaries/prologue-cli-$TRIPLE"

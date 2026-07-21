#!/usr/bin/env bash
# Build the prologue CLI and stage it where Tauri's bundler expects external
# binaries: src-tauri/binaries/prologue-<target-triple>. The bundler strips
# the triple suffix, producing Contents/MacOS/prologue inside the .app.
set -euo pipefail

cd "$(dirname "$0")/.."

# Non-interactive shells on this machine don't have cargo on PATH.
command -v cargo >/dev/null 2>&1 || export PATH="$HOME/.cargo/bin:$PATH"

TRIPLE="${TAURI_ENV_TARGET_TRIPLE:-$(rustc --print host-tuple)}"

cargo build --release -p prologue --manifest-path src-tauri/Cargo.toml

mkdir -p src-tauri/binaries
# rm first: cp onto an existing file (e.g. build.rs's non-executable
# placeholder) keeps the destination's permissions.
rm -f "src-tauri/binaries/prologue-$TRIPLE"
cp src-tauri/target/release/prologue "src-tauri/binaries/prologue-$TRIPLE"
chmod 755 "src-tauri/binaries/prologue-$TRIPLE"
echo "sidecar staged: src-tauri/binaries/prologue-$TRIPLE"

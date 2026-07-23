#!/usr/bin/env bash
# Cut a Prologue release from this machine: bump the version, build a signed
# updater bundle, and publish the DMG + update manifest to GitHub Releases.
# Shipped apps poll
#   https://github.com/skylerkatz/Prologue/releases/latest/download/latest.json
# so publishing the release is all it takes to roll every install forward.
#
# usage: scripts/release.sh <version>     e.g. scripts/release.sh 0.2.0
set -euo pipefail

cd "$(dirname "$0")/.."

REPO="skylerkatz/Prologue"
KEY_FILE="$HOME/.tauri/prologue.key"

VERSION="${1:-}"
VERSION="${VERSION#v}"
if [[ -z "$VERSION" ]]; then
  echo "usage: scripts/release.sh <version>  (e.g. scripts/release.sh 0.2.0)" >&2
  exit 1
fi

if [[ ! -f "$KEY_FILE" ]]; then
  echo "missing updater signing key: $KEY_FILE" >&2
  echo "generate one with: npm run tauri signer generate -- -w $KEY_FILE" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "working tree is not clean — commit or stash first" >&2
  exit 1
fi

# Release notes are authored ahead of time (see .claude/skills/release-notes)
# and must already be committed — the clean-tree check above enforces that.
# The entry becomes the GitHub Release body and the updater manifest notes.
NOTES_FILE="$(mktemp -t prologue-notes)"
trap 'rm -f "$NOTES_FILE"' EXIT
if ! node -e '
  const fs = require("fs");
  const [version, out] = process.argv.slice(1);
  let list = [];
  try { list = JSON.parse(fs.readFileSync("src/release-notes.json", "utf8")); } catch {}
  const entry = list.find((e) => e.version === version);
  const notes = (entry?.notes ?? "").trim();
  if (!notes) process.exit(1);
  fs.writeFileSync(out, notes + "\n");
' "$VERSION" "$NOTES_FILE"; then
  echo "no release notes for v$VERSION in src/release-notes.json" >&2
  echo "run /release-notes $VERSION in Claude, review, and commit the entry first" >&2
  exit 1
fi

BRANCH="$(git branch --show-current)"
if [[ "$BRANCH" != "main" ]]; then
  echo "warning: releasing from branch '$BRANCH', not main" >&2
fi

# Non-interactive shells on this machine don't have cargo on PATH.
command -v cargo >/dev/null 2>&1 || export PATH="$HOME/.cargo/bin:$PATH"

# The version lives in two files; keep them in lockstep.
npm pkg set "version=$VERSION"
npm install --package-lock-only >/dev/null
node -e '
  const fs = require("fs");
  const path = "src-tauri/tauri.conf.json";
  const conf = JSON.parse(fs.readFileSync(path, "utf8"));
  conf.version = process.argv[1];
  fs.writeFileSync(path, JSON.stringify(conf, null, 2) + "\n");
' "$VERSION"

# The release overlay turns on updater artifacts (.app.tar.gz + .sig), which
# need the signing key — everyday `npm run tauri build` stays key-free.
TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
  npm run tauri build -- --config src-tauri/tauri.release.conf.json

BUNDLE="src-tauri/target/release/bundle"
DMG="$BUNDLE/dmg/Prologue_${VERSION}_aarch64.dmg"
TAR="$BUNDLE/macos/Prologue.app.tar.gz"
SIG="$TAR.sig"
for f in "$DMG" "$TAR" "$SIG"; do
  [[ -f "$f" ]] || { echo "expected build artifact missing: $f" >&2; exit 1; }
done

# The manifest the shipped updater polls. The signature is minisign over the
# .app.tar.gz, verified against the pubkey baked into tauri.conf.json.
MANIFEST="$BUNDLE/latest.json"
node -e '
  const fs = require("fs");
  const [version, sigPath, notesPath, out] = process.argv.slice(1);
  fs.writeFileSync(out, JSON.stringify({
    version,
    notes: fs.readFileSync(notesPath, "utf8").trim(),
    pub_date: new Date().toISOString(),
    platforms: {
      "darwin-aarch64": {
        signature: fs.readFileSync(sigPath, "utf8"),
        url: `https://github.com/skylerkatz/Prologue/releases/download/v${version}/Prologue.app.tar.gz`,
      },
    },
  }, null, 2) + "\n");
' "$VERSION" "$SIG" "$NOTES_FILE" "$MANIFEST"

# No bump commit when re-releasing the version already in the files.
if ! git diff --quiet package.json package-lock.json src-tauri/tauri.conf.json; then
  git add package.json package-lock.json src-tauri/tauri.conf.json
  git commit -m "release: v$VERSION"
fi
git push origin "$BRANCH"

gh release create "v$VERSION" \
  --repo "$REPO" \
  --target "$(git rev-parse HEAD)" \
  --title "Prologue v$VERSION" \
  --notes-file "$NOTES_FILE" \
  "$DMG" "$TAR" "$SIG" "$MANIFEST"

echo
echo "released v$VERSION → https://github.com/$REPO/releases/tag/v$VERSION"

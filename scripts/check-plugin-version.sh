#!/usr/bin/env bash
# Fail when amp-plugin/prologue changed since <base> but its PLUGIN_VERSION
# did not go up. prologue-server offers an update to any running plugin
# older than the one its release was built with, so a change without a bump
# would never reach installed copies.
#
# usage: scripts/check-plugin-version.sh <base commit>
set -euo pipefail

cd "$(dirname "$0")/.."

BASE="${1:-}"
PLUGIN=amp-plugin/prologue
if [[ -z "$BASE" || "$BASE" =~ ^0+$ ]] || ! git cat-file -e "$BASE^{commit}" 2>/dev/null; then
  echo "No base commit to compare against; skipping the plugin version check."
  exit 0
fi

if git diff --quiet "$BASE" HEAD -- "$PLUGIN"; then
  echo "$PLUGIN is unchanged since ${BASE:0:7}."
  exit 0
fi

version() {
  sed -n 's/^const PLUGIN_VERSION = \([0-9][0-9]*\).*/\1/p' | head -1
}
before="$(git show "$BASE:$PLUGIN/index.ts" 2>/dev/null | version)"
after="$(version < "$PLUGIN/index.ts")"
before="${before:-0}"

if [[ -z "$after" ]]; then
  echo "::error file=$PLUGIN/index.ts::$PLUGIN/index.ts must declare const PLUGIN_VERSION = <number>"
  exit 1
fi
if (( after <= before )); then
  echo "::error file=$PLUGIN/index.ts::$PLUGIN changed since ${BASE:0:7}, but PLUGIN_VERSION is still $after. Bump it so installed plugins are offered the update."
  exit 1
fi
echo "PLUGIN_VERSION went from $before to $after."

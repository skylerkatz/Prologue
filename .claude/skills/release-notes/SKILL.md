---
name: release-notes
description: Draft user-facing release notes for the next Prologue release and write them into src/release-notes.json. Use when asked to write, draft, or update release notes, prepare a changelog entry, or get a release ready for scripts/release.sh (which aborts without a notes entry for the version).
user-invocable: true
---

# Drafting Prologue release notes

Release notes live in `src/release-notes.json` — a newest-first array of
`{ "version", "date", "notes" }`. The file is bundled into the app (the
What's New screen renders it) and `scripts/release.sh` uses the entry for
the GitHub Release body and the updater manifest. The script **aborts if
no entry exists for the version being released**, so this skill runs
before cutting a release.

## Determine the version and the range

The target version is the argument (`/release-notes 0.3.0`). If omitted,
find what changed first, then suggest a version yourself: patch bump for
fixes only, minor bump when there's a `feat:` — and confirm with the user.

Release tags are created server-side by `gh release create`, so local
tags lag. Always:

```sh
git fetch --tags --quiet origin
LAST_TAG="$(git tag --list 'v*' --sort=-v:refname | head -n1)"
git log --no-merges --oneline "$LAST_TAG"..HEAD
```

## Study the changes — diffs, not just subjects

Commit subjects follow conventional commits (`feat:`, `fix:`, …) but they
describe the code, not the experience. For each substantive commit, read
the diff (`git show <sha>`) far enough to answer: *what does a Prologue
user see, do, or stop suffering because of this?* Write from that answer.

## Write the notes

Markdown, with these sections (omit any that would be empty):

```markdown
## New

- Jump straight to a file from the sidebar — viewed files auto-expand.

## Fixed

- The CLI no longer disappears after an app update.

## Other

- …
```

Rules:
- User-facing language. Name the feature as the user meets it, not the
  component that implements it. No commit prefixes, scopes, or shas.
- Skip internal work (refactors, tests, CI, release plumbing) unless it
  changes something a user notices.
- Combine related commits into one bullet; split one commit into two
  bullets if it shipped two visible things.
- Short bullets, no trailing periods needed, no marketing fluff.

## Update src/release-notes.json

Prepend the entry (newest-first), replacing any existing entry for the
same version — re-drafting must not duplicate. Date is today,
`YYYY-MM-DD`. Keep 2-space indentation and a trailing newline. If the
file doesn't exist, create it as an array with this one entry. The
`notes` value is the markdown above with real newlines encoded as `\n`.

## Finish

Show the user the rendered notes for review. Do **not** commit — the
release script requires a clean tree, so the user (or a later explicit
request) commits the entry before running `scripts/release.sh <version>`.

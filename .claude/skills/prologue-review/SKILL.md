---
name: prologue-review
description: Read and respond to Prologue (Diff Viewer) code reviews with the `prologue` CLI. Use when asked to address review comments on a branch, when an exported review prompt says to reply via prologue, or when adding review comments/replies from the terminal.
---

# Working a Prologue review with the `prologue` CLI

Prologue stores code reviews (threads of comments anchored to a branch's
diff) in a local database shared with the Prologue app. The `prologue`
binary reads reviews and adds comments or replies. It never resolves,
dismisses, reopens, or deletes anything — closing a thread is the human
reviewer's act, in the app. Don't look for a workaround; there isn't one.

Run it from inside the repository being reviewed: the review is inferred
from the cwd's repo and checked-out branch. Anywhere else, pass a review id
or `repo@branch` (find ids with `prologue reviews`).

## Read the review

```sh
prologue reviews                 # all active reviews (add --archived for closed ones)
prologue show                    # current branch's threads: roots, replies, states, anchors
prologue show --json             # same, machine-readable
prologue show --file src/a.rs    # only threads on that file
prologue export --format prompt-md   # the full "address this review" prompt
```

`show` marks threads `orphaned` when their code left the diff, and each line
comment carries a code anchor quoting the commented lines — trust the anchor
text over drifted line numbers.

## The loop: address each thread, then reply to it

For each open thread: make the code change it asks for (or decide none is
needed), then record what you did as a reply to that thread:

```sh
prologue reply C12 --body "extracted the duplicated query into a helper (abc123)"
prologue reply C13 --body "no change needed: the null case is handled upstream"
```

- Reply to every top-level comment exactly once, as you finish it. `C12` and
  `12` both work; replying to a reply attaches to the same thread.
- Replies are stamped with the head SHA at reply time, so commit first, then
  reply, and the reply records the commit that answers it.
- A resolved or dismissed thread refuses replies ("reopen it first") — that
  thread is settled; leave it alone unless the reviewer reopens it.

## Adding new comments (flagging your own findings)

```sh
prologue comment --body "overall: consider splitting this module"        # review-level
prologue comment --file src/a.rs --body "this file needs tests"          # file-level
prologue comment --file src/a.rs --line 42-45 --body "possible null deref"  # line-level
```

Line comments need coordinates that exist in the review's **current** diff.
Read them first:

```sh
prologue show --file src/a.rs --diff    # per-hunk old/new line numbers
```

`--side new` (the default) targets added/context lines; `--side old` targets
removed lines. If the comment is rejected because the lines aren't in a hunk
or the selection crosses hunks, the diff changed since you read it —
re-run `show --file … --diff` and retry with fresh numbers.

## Authorship

Everything you write lands as author `agent` by default and is badged in the
app. Pass `--author NAME` (or set `PROLOGUE_AUTHOR`) only when told to write
as someone else. Never use `reviewer` — that name means "written in the app
by the human".

---
name: prologue-review
description: Works through open Prologue code review comments with the prologue CLI. Read each comment, change the code or answer it, and reply on the comment. Use when the user sends review comments from Prologue ("Send to Amp"), or asks you to address, answer, or reply to Prologue review comments.
---

# Working through Prologue review comments

Prologue is the user's code review app. In an orb it runs as the `prologue` service in the
Portal pane. The reviewer leaves comments there, and you answer them with the `prologue` CLI.
Replies show up live in the reviewer's browser.

## Where to run the CLI

Run `prologue` from inside the reviewed repository. The CLI picks the review from the current
checkout's repository and branch, so no review id is needed. If the message names a repository
path, `cd` there first. The CLI is on PATH (`~/.local/bin/prologue`, set up by "Prologue: Start
Review"). If it is missing, ask the user to run Start Review.

Run every reply command with `PROLOGUE_AUTHOR=amp` set, so the app shows your replies as the
agent's.

## Workflow

1. **List the open comments.**
   ```sh
   prologue show --open --json
   ```
   This returns `{review, anchorsCurrent, threads}`. Each thread is
   `{root, replies, orphaned, changed}`. `root` has `id` (`C<id>` on the command line),
   `level` (`review`, `file`, or `line`), `filePath`, `side`, `startLine`, `endLine`,
   `codeAnchor` (the code as it was when commented), `commitSha` (HEAD when commented), `body`,
   and `author`. The reviewer's author is `reviewer`. Read the replies as part of the comment.
   - `prologue show --open` prints the same threads as text.
   - If the message lists ids such as `C631–C634`, handle exactly those.
2. **Read the code for each comment.** Open the file at the comment's lines. Check the
   surrounding code and any callers before you decide. `prologue show --file PATH --diff`
   prints the file's current diff hunks with old and new line numbers.
3. **Act on it.**
   - When the comment asks for a change, make it and verify it, for example by running the
     relevant tests.
   - When the comment asks a question, or you disagree, answer it without changing code.
   - When the comment is ambiguous, ask in the reply rather than guessing.
4. **Reply on the comment.**
   ```sh
   PROLOGUE_AUTHOR=amp prologue reply C631 --body "Renamed to parseHeader and updated both callers in src/http.ts."
   ```
   Any comment id in the thread works; the reply attaches to the thread's root. Say what you
   changed and where, or give your answer. Keep it to a few sentences; Markdown works. Reply
   once per comment, after the work is done. Do not add top-level comments with
   `prologue comment` unless the user asks you to.
5. **Never try to resolve a comment.** Resolving is the reviewer's decision, in the app, so the
   CLI has no resolve, dismiss, or reopen command on purpose. Don't look for a workaround such as
   editing the database. When you have fully addressed a comment, say so in the reply
   ("Done: …"), and the reviewer resolves it. `prologue reply` refuses closed threads, which
   `--open` already leaves out.
6. **Summarize.** Finish with a short summary for the user: one line per comment, saying what
   you changed or answered, and anything still waiting on them.

## When the code changed since the comment

A comment records the code it was left on (`codeAnchor`) and the commit (`commitSha`). The
code may have changed since then, through your edits or newer commits.

- `changed: true` (text: "code changed since commented") means the commented lines are still
  in the diff but differ from `codeAnchor`. Read the current code before you act.
- `orphaned: true` (text: "orphaned — last known location") means the lines no longer exist
  in the current diff. Its line numbers are where the code used to be.
- To see what changed under a comment, run `git diff <commitSha> -- <filePath>`, or compare
  `codeAnchor` with the current file.
- If the change already does what the comment asks, don't change it again. Reply with what
  fixed it, for example "Already addressed in 3f2a9c1: …".
- If the comment no longer applies because the code was removed or rewritten, say so in the
  reply. Explain where the logic went now, if anywhere.
- If you can't tell what the comment meant against the new code, ask in the reply.
- If `prologue show` warns (or `anchorsCurrent` is false) that it could not recompute anchors, the line numbers are the stored
  ones. Find the code by its `codeAnchor` text instead.

## Troubleshooting

- "No review" or a wrong review: make sure you are in the right repository and branch.
  `prologue reviews` lists the reviews for this repository. Pass an id or `repo@branch` to
  `show` (`prologue show 3`).
- The reviewer's browser updates by itself; there's no need to restart the `prologue` service
  after replying.

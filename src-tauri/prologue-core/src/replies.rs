//! Replies written by an agent that cannot reach the reviews database — an
//! agent running on a remote machine (e.g. an Amp orb). The remote prompt
//! export asks it to finish with one shell-safe line,
//! `prologue import-replies --base64 <replies JSON, base64>`, which the
//! reviewer pastes locally. The same JSON is also accepted on stdin, bare or
//! inside a fenced ```` ```prologue-replies ```` block (the last one wins),
//! e.g. from a whole transcript.
//!
//! The import is all-or-nothing: every entry is validated before anything is
//! written, and the writes share one transaction. Re-running an import is
//! safe — a reply whose thread already carries the same text (from any
//! author: a re-paste may name the agent differently) is skipped as already
//! imported.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::db_err;
use crate::diff::{self, DiffSpec};
use crate::repo::open_git_repo;
use crate::review::{
    create_comment_impl, get_comment, Comment, CommentLevel, CommentState, NewComment, Review,
    ReviewStatus,
};

/// The fence info string that marks a replies block.
pub const REPLIES_FENCE_INFO: &str = "prologue-replies";

/// The JSON inside a ```` ```prologue-replies ```` fence.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RepliesBlock {
    /// The review the replies belong to, as the export prompt stated it.
    #[serde(default)]
    pub review: Option<i64>,
    pub replies: Vec<ReplyEntry>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReplyEntry {
    /// `"C12"`, `"12"`, or `12`.
    pub id: CommentRef,
    pub body: String,
    /// The commit that addresses the comment; absent when nothing changed.
    #[serde(default)]
    pub sha: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum CommentRef {
    Number(i64),
    Text(String),
}

impl CommentRef {
    fn parse(&self) -> Result<i64, String> {
        match self {
            CommentRef::Number(n) => Ok(*n),
            CommentRef::Text(s) => s
                .trim()
                .strip_prefix(['C', 'c'])
                .unwrap_or(s.trim())
                .parse()
                .map_err(|_| format!("Cannot parse comment id '{s}' — expected an id like C12")),
        }
    }
}

/// The replies JSON carried by `prologue import-replies --base64 DATA`.
/// Base64 keeps the pasted command shell-safe whatever the replies say
/// (quotes, backticks, `$`); whitespace from a wrapped paste is ignored.
pub fn parse_base64_payload(data: &str) -> Result<RepliesBlock, String> {
    use base64::Engine as _;
    let compact: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact)
        .map_err(|e| format!("The --base64 data is not valid base64 ({e}) — was the line cut off?"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| "The --base64 data does not decode to text".to_owned())?;
    serde_json::from_str(&text)
        .map_err(|e| format!("The --base64 data is not valid replies JSON: {e}"))
}

/// Find the last complete ```` ```prologue-replies ```` block in `text` and
/// parse it. Other fenced blocks are skipped whole, so a fence quoted inside
/// one (a tool result, a file dump) is never mistaken for the real thing;
/// taking the last block means a revised block in a later message wins.
pub fn parse_last_replies_block(text: &str) -> Result<RepliesBlock, String> {
    let mut last: Option<String> = None;
    let mut saw_unclosed = false;
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some((fence_char, fence_len, info)) = fence_open(line) else {
            continue;
        };
        let mut body = String::new();
        let mut closed = false;
        for inner in lines.by_ref() {
            if is_fence_close(inner, fence_char, fence_len) {
                closed = true;
                break;
            }
            body.push_str(inner);
            body.push('\n');
        }
        if info == REPLIES_FENCE_INFO {
            if closed {
                last = Some(body);
            } else {
                saw_unclosed = true;
            }
        }
    }
    // No fence at all: the input may be the bare JSON (copied out of the
    // agent's block by hand).
    if last.is_none() && !saw_unclosed && text.trim_start().starts_with('{') {
        return serde_json::from_str(text.trim())
            .map_err(|e| format!("The input is not valid replies JSON: {e}"));
    }
    let Some(body) = last else {
        return Err(if saw_unclosed {
            format!("The ```{REPLIES_FENCE_INFO} block is never closed — the input looks truncated")
        } else {
            format!(
                "No ```{REPLIES_FENCE_INFO} block found in the input — pass the agent's \
                 final message or its whole transcript"
            )
        });
    };
    serde_json::from_str(&body)
        .map_err(|e| format!("The ```{REPLIES_FENCE_INFO} block is not valid replies JSON: {e}"))
}

/// A CommonMark fence opener: up to three spaces, then three or more
/// backticks or tildes, then the info string.
fn fence_open(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let fence_char = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = rest.len() - rest.trim_start_matches(fence_char).len();
    if len < 3 {
        return None;
    }
    let info = rest[len..].trim();
    if fence_char == '`' && info.contains('`') {
        return None;
    }
    Some((fence_char, len, info))
}

fn is_fence_close(line: &str, fence_char: char, open_len: usize) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let len = rest.len() - rest.trim_start_matches(fence_char).len();
    len >= open_len && rest[len..].trim().is_empty()
}

/// What an import did (or, on a dry run, would do).
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// The review the replies went to.
    pub review_id: i64,
    pub dry_run: bool,
    /// Replies written — or, on a dry run, the replies that would be.
    pub replies: Vec<PlannedReply>,
    /// Entries left alone, with why.
    pub skipped: Vec<SkippedReply>,
    /// Open threads the block had no reply for.
    pub unanswered: Vec<i64>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlannedReply {
    /// The thread root the reply attaches to.
    pub thread_id: i64,
    /// The created reply's id; None on a dry run.
    pub reply_id: Option<i64>,
    pub body: String,
    /// The full commit SHA the reply is stamped with; None means the
    /// review's head at import time.
    pub commit_sha: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkippedReply {
    pub thread_id: i64,
    pub reason: String,
}

/// Validate every entry of `block` against `review`, then (unless
/// `dry_run`) post them in one transaction. Any invalid entry fails the
/// whole import with every problem listed; nothing is written.
pub fn import_replies_impl(
    conn: &Connection,
    review: &Review,
    block: &RepliesBlock,
    author: &str,
    dry_run: bool,
) -> Result<ImportReport, String> {
    if let Some(stated) = block.review {
        if stated != review.id {
            return Err(format!(
                "The replies block is for review {stated}, not review {} — pass the review \
                 it names, or check you exported the right one",
                review.id
            ));
        }
    }
    if review.status != ReviewStatus::Active {
        return Err(format!("Review {} is archived and read-only", review.id));
    }
    if block.replies.is_empty() {
        return Err("The replies block has no replies".to_owned());
    }

    let repo = open_git_repo(&review.repo_path)?;
    let mut errors = Vec::new();
    let mut planned = Vec::new();
    let mut skipped = Vec::new();
    let mut seen_roots = HashSet::new();

    for entry in &block.replies {
        let id = match entry.id.parse() {
            Ok(id) => id,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let root = match get_comment(conn, id).and_then(|c| match c.parent_id {
            None => Ok(c),
            Some(root_id) => get_comment(conn, root_id),
        }) {
            Ok(root) => root,
            Err(_) => {
                errors.push(format!("C{id}: no such comment"));
                continue;
            }
        };
        if root.review_id != review.id {
            errors.push(format!("C{id}: belongs to review {}, not {}", root.review_id, review.id));
            continue;
        }
        if !seen_roots.insert(root.id) {
            errors.push(format!("C{}: more than one reply in the block", root.id));
            continue;
        }
        let body = entry.body.trim();
        if body.is_empty() {
            errors.push(format!("C{}: empty reply body", root.id));
            continue;
        }
        let commit_sha = match entry.sha.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(sha) => match resolve_sha(&repo, sha) {
                Ok(full) => Some(full),
                Err(e) => {
                    errors.push(format!("C{}: {e}", root.id));
                    continue;
                }
            },
        };
        if root.state != CommentState::Open {
            skipped.push(SkippedReply {
                thread_id: root.id,
                reason: format!("thread is {}", root.state.as_str()),
            });
            continue;
        }
        if reply_exists(conn, root.id, body)? {
            skipped.push(SkippedReply {
                thread_id: root.id,
                reason: "already imported".to_owned(),
            });
            continue;
        }
        planned.push(PlannedReply {
            thread_id: root.id,
            reply_id: None,
            body: body.to_owned(),
            commit_sha,
        });
    }
    if !errors.is_empty() {
        return Err(format!(
            "Nothing imported — fix these and re-run:\n  {}",
            errors.join("\n  ")
        ));
    }

    let unanswered = open_roots(conn, review.id)?
        .into_iter()
        .filter(|id| !seen_roots.contains(id))
        .collect();

    if !dry_run && !planned.is_empty() {
        let tx = conn.unchecked_transaction().map_err(db_err)?;
        for reply in &mut planned {
            let created = post_reply(&tx, review, reply, author)?;
            reply.reply_id = Some(created.id);
            reply.commit_sha = Some(created.commit_sha);
        }
        tx.commit().map_err(db_err)?;
    }

    Ok(ImportReport {
        review_id: review.id,
        dry_run,
        replies: planned,
        skipped,
        unanswered,
    })
}

/// A hex commit id (abbreviated or full) that exists in the local repo,
/// expanded to its full SHA. Ref names and revision expressions are refused:
/// the field records which commit answered the comment, nothing else.
fn resolve_sha(repo: &git2::Repository, sha: &str) -> Result<String, String> {
    if !(4..=40).contains(&sha.len()) || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("'{sha}' is not a commit SHA"));
    }
    diff::resolve_commit(repo, sha).map(|c| c.id().to_string()).map_err(|_| {
        format!("commit {sha} is not in the local repository — fetch or sync the agent's work first")
    })
}

fn reply_exists(conn: &Connection, root_id: i64, body: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT 1 FROM comments WHERE parent_id = ?1 AND body = ?2 LIMIT 1",
        (root_id, body),
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
    .map_err(db_err)
}

fn open_roots(conn: &Connection, review_id: i64) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM comments
             WHERE review_id = ?1 AND parent_id IS NULL AND state = ?2 ORDER BY id",
        )
        .map_err(db_err)?;
    let ids = stmt
        .query_map((review_id, CommentState::Open.as_str()), |r| r.get(0))
        .map_err(db_err)?
        .collect::<Result<Vec<i64>, _>>()
        .map_err(db_err)?;
    Ok(ids)
}

fn post_reply(
    conn: &Connection,
    review: &Review,
    reply: &PlannedReply,
    author: &str,
) -> Result<Comment, String> {
    // A reply stamps the commit its spec's head resolves to — pointing the
    // head at the agent's commit records that commit instead of the
    // review's current tip. Replies use nothing else from the spec.
    let mut spec = DiffSpec::from(review);
    if let Some(sha) = &reply.commit_sha {
        spec.head = sha.clone();
    }
    create_comment_impl(
        conn,
        &spec,
        NewComment {
            review_id: review.id,
            level: CommentLevel::Review, // ignored: replies inherit the root's level
            file_path: None,
            side: None,
            start_line: None,
            end_line: None,
            parent_id: Some(reply.thread_id),
            body: reply.body.clone(),
            author: Some(author.to_owned()),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::DiffMode;
    use crate::review::{open_review_impl, update_comment_state_impl};
    use crate::testutil::{open_test_db as test_db, FixtureRepo};

    fn block(text: &str) -> RepliesBlock {
        parse_last_replies_block(text).unwrap()
    }

    #[test]
    fn finds_the_last_block_and_skips_other_fences() {
        let text = "\
## User

Reply in a fenced block like:

```json
{\"review\": 1, \"replies\": [{\"id\": \"C1\", \"body\": \"example\"}]}
```

## Assistant

**Tool Result:**

````
```prologue-replies
{\"replies\": [{\"id\": \"C9\", \"body\": \"quoted inside a tool result\"}]}
```
````

```prologue-replies
{\"review\": 7, \"replies\": [{\"id\": \"C1\", \"body\": \"first draft\"}]}
```

Revised:

~~~prologue-replies
{\"review\": 7, \"replies\": [{\"id\": \"C1\", \"body\": \"final\", \"sha\": \"abc1234\"}, {\"id\": 2, \"body\": \"no change\"}]}
~~~
";
        let parsed = block(text);
        assert_eq!(parsed.review, Some(7));
        assert_eq!(parsed.replies.len(), 2);
        assert_eq!(parsed.replies[0].body, "final");
        assert_eq!(parsed.replies[0].sha.as_deref(), Some("abc1234"));
        assert_eq!(parsed.replies[1].id, CommentRef::Number(2));
    }

    #[test]
    fn missing_unclosed_and_malformed_blocks_are_errors() {
        let err = parse_last_replies_block("no block here").unwrap_err();
        assert!(err.contains("No ```prologue-replies block"), "{err}");
        let err = parse_last_replies_block("```prologue-replies\n{\"replies\": [").unwrap_err();
        assert!(err.contains("never closed"), "{err}");
        let err = parse_last_replies_block("```prologue-replies\nnot json\n```\n").unwrap_err();
        assert!(err.contains("not valid replies JSON"), "{err}");
    }

    #[test]
    fn base64_payloads_decode_and_survive_shell_hostile_text() {
        use base64::Engine as _;
        let json = "{\"review\": 3, \"replies\": [{\"id\": \"C1\", \"body\": \"it's `done` for $HOME\"}]}";
        let encoded = base64::engine::general_purpose::STANDARD.encode(json);
        let parsed = parse_base64_payload(&encoded).unwrap();
        assert_eq!(parsed.review, Some(3));
        assert_eq!(parsed.replies[0].body, "it's `done` for $HOME");
        // A wrapped paste still decodes.
        let (a, b) = encoded.split_at(10);
        assert_eq!(parse_base64_payload(&format!("{a}\n  {b}")).unwrap(), parsed);
        // A cut-off line is reported, not half-imported.
        let err = parse_base64_payload(&encoded[..encoded.len() - 5]).unwrap_err();
        assert!(err.contains("--base64 data"), "{err}");
        let err = parse_base64_payload("not base64!").unwrap_err();
        assert!(err.contains("not valid base64"), "{err}");
    }

    #[test]
    fn bare_json_without_a_fence_is_accepted() {
        let parsed =
            block("\n  {\"review\": 3, \"replies\": [{\"id\": \"C1\", \"body\": \"done\"}]}\n");
        assert_eq!(parsed.review, Some(3));
        assert_eq!(parsed.replies[0].body, "done");
        let err = parse_last_replies_block("{\"replies\": ").unwrap_err();
        assert!(err.contains("not valid replies JSON"), "{err}");
    }

    struct Setup {
        _dir: tempfile::TempDir,
        conn: Connection,
        fixture: FixtureRepo,
        review: Review,
        c1: Comment,
        c2: Comment,
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db(&dir);
        let fixture = FixtureRepo::standard_review_fixture();
        let review =
            open_review_impl(&conn, &fixture.path(), "feature", "main", DiffMode::Committed)
                .unwrap();
        let root = |body: &str| {
            create_comment_impl(
                &conn,
                &DiffSpec::from(&review),
                NewComment {
                    review_id: review.id,
                    level: CommentLevel::Review,
                    file_path: None,
                    side: None,
                    start_line: None,
                    end_line: None,
                    parent_id: None,
                    body: body.to_owned(),
                    author: None,
                },
            )
            .unwrap()
        };
        let c1 = root("rename this");
        let c2 = root("needs a test");
        Setup { _dir: dir, conn, fixture, review, c1, c2 }
    }

    fn replies_json(review: i64, entries: &str) -> RepliesBlock {
        block(&format!("```prologue-replies\n{{\"review\": {review}, \"replies\": [{entries}]}}\n```\n"))
    }

    #[test]
    fn imports_replies_stamped_with_the_agents_commit() {
        let s = setup();
        let agent_commit = s.fixture.commit_file("code.txt", "renamed\n", "rename");
        let short = &agent_commit.to_string()[..9];
        let b = replies_json(
            s.review.id,
            &format!(
                "{{\"id\": \"C{}\", \"body\": \" renamed it \", \"sha\": \"{short}\"}}",
                s.c1.id
            ),
        );

        let dry = import_replies_impl(&s.conn, &s.review, &b, "amp", true).unwrap();
        assert!(dry.dry_run);
        assert_eq!(dry.replies.len(), 1);
        assert_eq!(dry.replies[0].reply_id, None);
        assert_eq!(dry.unanswered, vec![s.c2.id]);
        let count = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM comments WHERE parent_id IS NOT NULL", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(count(&s.conn), 0, "a dry run writes nothing");

        let report = import_replies_impl(&s.conn, &s.review, &b, "amp", false).unwrap();
        let reply = get_comment(&s.conn, report.replies[0].reply_id.unwrap()).unwrap();
        assert_eq!(reply.parent_id, Some(s.c1.id));
        assert_eq!(reply.body, "renamed it");
        assert_eq!(reply.author, "amp");
        assert_eq!(reply.commit_sha, agent_commit.to_string());

        // Re-running is a no-op: the same reply is recognized, even under
        // another author name.
        let again = import_replies_impl(&s.conn, &s.review, &b, "agent", false).unwrap();
        assert!(again.replies.is_empty());
        assert_eq!(again.skipped[0].reason, "already imported");
        assert_eq!(count(&s.conn), 1);
    }

    #[test]
    fn replies_without_a_sha_stamp_the_review_head() {
        let s = setup();
        let b = replies_json(s.review.id, &format!("{{\"id\": {}, \"body\": \"no change\"}}", s.c2.id));
        let report = import_replies_impl(&s.conn, &s.review, &b, "agent", false).unwrap();
        let reply = get_comment(&s.conn, report.replies[0].reply_id.unwrap()).unwrap();
        let head = s.fixture.repo.revparse_single("feature").unwrap().id().to_string();
        assert_eq!(reply.commit_sha, head);
    }

    #[test]
    fn any_invalid_entry_fails_the_whole_import() {
        let s = setup();
        let b = replies_json(
            s.review.id,
            &format!(
                "{{\"id\": \"C{}\", \"body\": \"ok\"}}, \
                 {{\"id\": \"C999\", \"body\": \"x\"}}, \
                 {{\"id\": \"C{}\", \"body\": \"x\", \"sha\": \"deadbeefdeadbeef\"}}",
                s.c1.id, s.c2.id
            ),
        );
        let err = import_replies_impl(&s.conn, &s.review, &b, "agent", false).unwrap_err();
        assert!(err.contains("C999: no such comment"), "{err}");
        assert!(err.contains("not in the local repository"), "{err}");
        // Ref names are refused: the field records a commit, nothing else.
        let b = replies_json(
            s.review.id,
            &format!("{{\"id\": {}, \"body\": \"x\", \"sha\": \"main\"}}", s.c1.id),
        );
        let err = import_replies_impl(&s.conn, &s.review, &b, "agent", false).unwrap_err();
        assert!(err.contains("'main' is not a commit SHA"), "{err}");
        let replies: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM comments WHERE parent_id IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(replies, 0, "nothing is written when any entry is invalid");
    }

    #[test]
    fn duplicate_threads_wrong_review_and_closed_threads() {
        let s = setup();
        // Two entries for one thread.
        let b = replies_json(
            s.review.id,
            &format!("{{\"id\": {0}, \"body\": \"a\"}}, {{\"id\": {0}, \"body\": \"b\"}}", s.c1.id),
        );
        let err = import_replies_impl(&s.conn, &s.review, &b, "agent", false).unwrap_err();
        assert!(err.contains("more than one reply"), "{err}");

        // A block stating another review.
        let b = replies_json(s.review.id + 1, &format!("{{\"id\": {}, \"body\": \"a\"}}", s.c1.id));
        let err = import_replies_impl(&s.conn, &s.review, &b, "agent", false).unwrap_err();
        assert!(err.contains("is for review"), "{err}");

        // A thread the reviewer closed meanwhile is skipped, not an error.
        update_comment_state_impl(&s.conn, s.c1.id, CommentState::Resolved).unwrap();
        let b = replies_json(
            s.review.id,
            &format!(
                "{{\"id\": {}, \"body\": \"a\"}}, {{\"id\": {}, \"body\": \"b\"}}",
                s.c1.id, s.c2.id
            ),
        );
        let report = import_replies_impl(&s.conn, &s.review, &b, "agent", false).unwrap();
        assert_eq!(report.replies.len(), 1);
        assert_eq!(report.skipped[0].thread_id, s.c1.id);
        assert_eq!(report.skipped[0].reason, "thread is resolved");
        assert!(report.unanswered.is_empty());
    }
}

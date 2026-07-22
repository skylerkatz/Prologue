// Generated from prologue-core's Rust types by the ts_export test.
// Do not edit — run `cargo test -p prologue-core` in src-tauri to refresh.

export type RepoInfo = { path: string, name: string, };
export type BranchList = { branches: Array<string>, current: string, defaultBase: string, };
export type DiffMode = "committed" | "staged" | "all";
export type FileStatus = "added" | "modified" | "deleted" | "renamed";
export type FileSummary = { path: string, 
/**
 * Previous path; present only for renames.
 */
oldPath: string | null, status: FileStatus, additions: number, deletions: number, binary: boolean, 
/**
 * Content identity of both diff sides: "<old_blob_oid>:<new_blob_oid>:<mode>".
 * A zero OID means the side is absent (add/delete). Reviewed-file marks
 * store this value and compare it against later summaries to detect
 * "changed since review"; raw content, so it never varies with the
 * hide-whitespace toggle.
 */
fingerprint: string, };
export type DiffSummary = { baseRef: string, headRef: string, 
/**
 * SHA of merge-base(base, head) — the commit the diff is computed against.
 */
mergeBase: string, files: Array<FileSummary>, totalAdditions: number, totalDeletions: number, };
export type LineKind = "context" | "addition" | "deletion";
export type IntralineRange = { start: number, end: number, };
export type DiffLine = { kind: LineKind, oldLineno: number | null, newLineno: number | null, content: string, 
/**
 * Word-level changed spans (UTF-16 units) when this line pairs with a
 * counterpart in its hunk; omitted from JSON when absent, so the shape
 * stays backward-compatible.
 */
intraline?: Array<IntralineRange>, };
export type Hunk = { header: string, oldStart: number, oldLines: number, newStart: number, newLines: number, lines: Array<DiffLine>, };
export type FileDiff = { path: string, oldPath: string | null, status: FileStatus, binary: boolean, hunks: Array<Hunk>, 
/**
 * Total line count of the new side, bounding expand-context below the
 * last hunk; `None` for deleted or binary files (nothing to expand).
 */
newTotalLines: number | null, };
export type ContextLines = { 
/**
 * 1-based new-side line number of the first returned line.
 */
start: number, lines: Array<string>, 
/**
 * Total line count of the new-side file, so the frontend knows how far
 * the gap after the last hunk extends.
 */
totalLines: number, };
export type ReviewStatus = "active" | "archived";
export type Review = { 
/**
 * SQLite rowid — far below 2^53, a plain JS number on the wire.
 */
id: number, repoPath: string, branch: string, baseRef: string, mode: DiffMode, status: ReviewStatus, createdAt: string, updatedAt: string, };
export type OpenReviewResult = { review: Review | null, branchMerged: boolean, };
export type ReviewedFile = { filePath: string, fingerprint: string, reviewedAt: string, };
export type ArchivedReview = { commentCount: number, 
/**
 * SQLite rowid — far below 2^53, a plain JS number on the wire.
 */
id: number, repoPath: string, branch: string, baseRef: string, mode: DiffMode, status: ReviewStatus, createdAt: string, updatedAt: string, };
export type CommentLevel = "review" | "file" | "line";
export type CommentSide = "old" | "new";
export type CommentState = "open" | "resolved" | "dismissed";
export type CodeAnchor = { hunkHeader: string, contextBefore: Array<string>, lines: Array<string>, contextAfter: Array<string>, };
export type Comment = { 
/**
 * SQLite rowid — far below 2^53, a plain JS number on the wire.
 */
id: number, reviewId: number, level: CommentLevel, filePath: string | null, side: CommentSide | null, startLine: number | null, endLine: number | null, codeAnchor: CodeAnchor | null, commitSha: string, state: CommentState, body: string, 
/**
 * Thread root this comment replies to; None for roots. Threads are one
 * level deep — a reply's parent is always a root. Replies inherit the
 * root's file/side/lines/anchor context (their own stay NULL), and
 * their lifecycle is the root's (`state` is meaningless on replies).
 */
parentId: number | null, 
/**
 * Who wrote it: 'reviewer' for the app's own writes, anything else for
 * external writers (e.g. 'agent' via the prologue CLI). The UI badges
 * non-reviewer authors.
 */
author: string, createdAt: string, updatedAt: string, };
export type NewComment = { reviewId: number, level: CommentLevel, filePath?: string, side?: CommentSide, startLine?: number, endLine?: number, 
/**
 * Set to any comment in a thread to reply; the reply attaches to the
 * thread ROOT (replying to a reply joins the same flat thread). All
 * positional fields above are ignored for replies — a reply inherits
 * its context from the root.
 */
parentId?: number, body: string, 
/**
 * Who is writing. The app's IPC payloads never set it (None →
 * 'reviewer'); external writers name themselves, e.g. 'agent'.
 */
author?: string, };
export type AnchorStatus = "anchored" | "changed" | "orphaned";
export type ReanchorResult = { commentId: number, status: AnchorStatus, startLine: number | null, endLine: number | null, };
export type ExportFormat = "markdown" | "json" | "prompt-markdown" | "prompt-json";
export type GuideSection = { title: string, summary: string, files: Array<string>, };
export type Guide = { 
/**
 * SQLite rowid — far below 2^53, a plain JS number on the wire.
 */
id: number, reviewId: number, baseRef: string, headRef: string, mode: DiffMode, 
/**
 * File path → [`FileSummary`] fingerprint at generation time; comparing
 * against the current summary detects files "changed since the guide".
 *
 * [`FileSummary`]: crate::diff::FileSummary
 */
fingerprints: { [key in string]?: string }, model: string, 
/**
 * Logged for the record only; no UI surfaces it.
 */
costUsd: number | null, createdAt: string, sections: Array<GuideSection>, };

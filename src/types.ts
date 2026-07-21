export interface RepoInfo {
  path: string;
  name: string;
}

export interface BranchList {
  branches: string[];
  current: string;
  defaultBase: string;
}

export type WorkingTreeMode = "committed" | "staged" | "all";

export type FileStatus = "added" | "modified" | "deleted" | "renamed";

export interface FileSummary {
  path: string;
  /** Previous path; set only for renames. */
  oldPath: string | null;
  status: FileStatus;
  additions: number;
  deletions: number;
  binary: boolean;
}

export interface DiffSummary {
  baseRef: string;
  headRef: string;
  /** SHA of merge-base(base, head) — what the diff is computed against. */
  mergeBase: string;
  files: FileSummary[];
  totalAdditions: number;
  totalDeletions: number;
}

export type LineKind = "context" | "addition" | "deletion";

/** A changed span within a line, in UTF-16 code units (`end` exclusive) —
 * directly usable as JavaScript string indices. */
export interface IntralineRange {
  start: number;
  end: number;
}

export interface DiffLine {
  kind: LineKind;
  oldLineno: number | null;
  newLineno: number | null;
  content: string;
  /** Word-level changed spans, present only on paired deletion/addition
   * lines (computed in Rust; omitted otherwise). */
  intraline?: IntralineRange[];
}

export interface Hunk {
  header: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: DiffLine[];
}

export interface FileDiff {
  path: string;
  oldPath: string | null;
  status: FileStatus;
  binary: boolean;
  hunks: Hunk[];
  /**
   * Total line count of the new side, bounding expand-context below the last
   * hunk; null for deleted or binary files (nothing to expand).
   */
  newTotalLines: number | null;
}

/** Unchanged new-side lines fetched for an expand-context click. */
export interface ContextLines {
  /** 1-based new-side line number of the first returned line. */
  start: number;
  lines: string[];
  /** Total line count of the new-side file. */
  totalLines: number;
}

/** A review session: one active review per (repo, branch). */
export interface Review {
  id: number;
  repoPath: string;
  branch: string;
  baseRef: string;
  mode: WorkingTreeMode;
  status: "active" | "archived";
  createdAt: string;
  updatedAt: string;
}

export type CommentLevel = "review" | "file" | "line";

export type CommentSide = "old" | "new";

export type CommentState = "open" | "resolved" | "dismissed";

/** Verbatim code captured at comment time to re-locate the spot after edits. */
export interface CodeAnchor {
  hunkHeader: string;
  contextBefore: string[];
  lines: string[];
  contextAfter: string[];
}

export interface Comment {
  /** SQLite row id, displayed as C<id>. */
  id: number;
  reviewId: number;
  level: CommentLevel;
  filePath: string | null;
  side: CommentSide | null;
  startLine: number | null;
  endLine: number | null;
  codeAnchor: CodeAnchor | null;
  /** Head commit SHA at comment time. */
  commitSha: string;
  state: CommentState;
  body: string;
  /**
   * Thread root this comment replies to; null for roots. Threads are one
   * level deep. Replies inherit the root's file/side/lines context (their
   * own stay null) and have no lifecycle of their own — the root's state
   * governs the whole thread.
   */
  parentId: number | null;
  /**
   * Who wrote it: "reviewer" for comments made in the app, anything else
   * for external writers (e.g. "agent" via the prologue CLI). Non-reviewer
   * authors get a badge.
   */
  author: string;
  createdAt: string;
  updatedAt: string;
}

/** What `open_review` produced; `review` is null for a merged branch that
 * was never reviewed, and archived (read-only) when the branch is merged. */
export interface OpenReviewResult {
  review: Review | null;
  branchMerged: boolean;
}

/** Where a line comment's anchor landed after a diff refresh. */
export type AnchorStatus = "anchored" | "changed" | "orphaned";

export interface ReanchorResult {
  commentId: number;
  status: AnchorStatus;
  startLine: number | null;
  endLine: number | null;
}

/** An archived review row for the read-only browser. */
export interface ArchivedReview extends Review {
  commentCount: number;
}

/** Caller-provided fields of a new comment; anchor and SHA are captured in Rust. */
export interface NewCommentInput {
  level: CommentLevel;
  filePath?: string;
  side?: CommentSide;
  startLine?: number;
  endLine?: number;
  /** Reply to this comment's thread; Rust resolves it to the thread root
   * and ignores the positional fields above. */
  parentId?: number;
  body: string;
}

/** Replies grouped under their thread root, in chronological (id) order. */
export type RepliesByRoot = ReadonlyMap<number, Comment[]>;

export function groupReplies(comments: Comment[]): RepliesByRoot {
  const map = new Map<number, Comment[]>();
  for (const comment of comments) {
    if (comment.parentId === null) {
      continue;
    }
    const bucket = map.get(comment.parentId);
    if (bucket === undefined) {
      map.set(comment.parentId, [comment]);
    } else {
      bucket.push(comment);
    }
  }
  return map;
}

/** Clipboard export flavors; formatting happens in Rust. */
export type ExportFormat =
  | "markdown"
  | "json"
  | "prompt-markdown"
  | "prompt-json";

export const WORKING_TREE_MODES: ReadonlyArray<{
  value: WorkingTreeMode;
  label: string;
}> = [
  { value: "committed", label: "Committed only" },
  { value: "staged", label: "Include staged" },
  { value: "all", label: "Staged + unstaged" },
];

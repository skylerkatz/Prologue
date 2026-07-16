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

export interface DiffLine {
  kind: LineKind;
  oldLineno: number | null;
  newLineno: number | null;
  content: string;
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
  body: string;
}

export const WORKING_TREE_MODES: ReadonlyArray<{
  value: WorkingTreeMode;
  label: string;
}> = [
  { value: "committed", label: "Committed only" },
  { value: "staged", label: "Include staged" },
  { value: "all", label: "Staged + unstaged" },
];

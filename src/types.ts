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

export const WORKING_TREE_MODES: ReadonlyArray<{
  value: WorkingTreeMode;
  label: string;
}> = [
  { value: "committed", label: "Committed only" },
  { value: "staged", label: "Include staged" },
  { value: "all", label: "Staged + unstaged" },
];

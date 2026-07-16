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
}

export const WORKING_TREE_MODES: ReadonlyArray<{
  value: WorkingTreeMode;
  label: string;
}> = [
  { value: "committed", label: "Committed only" },
  { value: "staged", label: "Include staged" },
  { value: "all", label: "Staged + unstaged" },
];

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

export const WORKING_TREE_MODES: ReadonlyArray<{
  value: WorkingTreeMode;
  label: string;
}> = [
  { value: "committed", label: "Committed only" },
  { value: "staged", label: "Include staged" },
  { value: "all", label: "Staged + unstaged" },
];

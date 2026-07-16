import { invoke } from "@tauri-apps/api/core";
import type {
  BranchList,
  DiffSummary,
  FileDiff,
  RepoInfo,
  WorkingTreeMode,
} from "./types";

// Typed wrappers around the Tauri commands.

export function openRepo(path: string): Promise<RepoInfo> {
  return invoke("open_repo", { path });
}

export function listBranches(repoPath: string): Promise<BranchList> {
  return invoke("list_branches", { repoPath });
}

export function getDiffSummary(
  repoPath: string,
  base: string,
  head: string,
  mode: WorkingTreeMode,
): Promise<DiffSummary> {
  return invoke("get_diff_summary", { repoPath, base, head, mode });
}

export function getFileDiff(
  repoPath: string,
  base: string,
  head: string,
  mode: WorkingTreeMode,
  path: string,
): Promise<FileDiff> {
  return invoke("get_file_diff", { repoPath, base, head, mode, path });
}

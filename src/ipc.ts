import { invoke } from "@tauri-apps/api/core";
import type { BranchList, RepoInfo } from "./types";

// Typed wrappers around the Tauri commands. M2 replaces the Rust bodies
// (git2-backed branches, diff summary, per-file diffs) behind these same calls.

export function openRepo(path: string): Promise<RepoInfo> {
  return invoke("open_repo", { path });
}

export function listBranches(repoPath: string): Promise<BranchList> {
  return invoke("list_branches", { repoPath });
}

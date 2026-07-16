import type { BranchList, RepoInfo, WorkingTreeMode } from "../types";
import { WORKING_TREE_MODES } from "../types";
import { ModeToggle } from "./ModeToggle";

interface ReviewShellProps {
  repo: RepoInfo;
  branchList: BranchList;
  branch: string;
  baseBranch: string;
  mode: WorkingTreeMode;
  onBranchChange: (branch: string) => void;
  onBaseBranchChange: (base: string) => void;
  onModeChange: (mode: WorkingTreeMode) => void;
  onSwitchRepo: () => void;
}

export function ReviewShell({
  repo,
  branchList,
  branch,
  baseBranch,
  mode,
  onBranchChange,
  onBaseBranchChange,
  onModeChange,
  onSwitchRepo,
}: ReviewShellProps) {
  const modeLabel =
    WORKING_TREE_MODES.find((m) => m.value === mode)?.label ?? mode;

  return (
    <div className="review-shell">
      <header className="toolbar">
        <button
          type="button"
          className="repo-button"
          title={repo.path}
          onClick={onSwitchRepo}
        >
          {repo.name}
        </button>
        <label className="control">
          <span>Base</span>
          <select
            value={baseBranch}
            onChange={(e) => onBaseBranchChange(e.currentTarget.value)}
          >
            {branchList.branches.map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </select>
        </label>
        <span className="arrow" aria-hidden="true">
          ←
        </span>
        <label className="control">
          <span>Branch</span>
          <select
            value={branch}
            onChange={(e) => onBranchChange(e.currentTarget.value)}
          >
            {branchList.branches.map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </select>
        </label>
        <div className="toolbar-spacer" />
        <ModeToggle mode={mode} onChange={onModeChange} />
      </header>
      <main className="diff-placeholder">
        <p>
          Diff for <code>{baseBranch}</code> ← <code>{branch}</code> (
          {modeLabel.toLowerCase()}) will render here in M2.
        </p>
      </main>
    </div>
  );
}

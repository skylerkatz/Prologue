import { useEffect, useState } from "react";
import { getDiffSummary } from "../ipc";
import type {
  BranchList,
  DiffSummary,
  RepoInfo,
  WorkingTreeMode,
} from "../types";
import { FileList } from "./FileList";
import { ModeToggle } from "./ModeToggle";

interface ReviewShellProps {
  repo: RepoInfo;
  branchList: BranchList;
  branch: string;
  baseBranch: string;
  mode: WorkingTreeMode;
  refreshKey: number;
  onBranchChange: (branch: string) => void;
  onBaseBranchChange: (base: string) => void;
  onModeChange: (mode: WorkingTreeMode) => void;
  onRefresh: () => void;
  onSwitchRepo: () => void;
}

export function ReviewShell({
  repo,
  branchList,
  branch,
  baseBranch,
  mode,
  refreshKey,
  onBranchChange,
  onBaseBranchChange,
  onModeChange,
  onRefresh,
  onSwitchRepo,
}: ReviewShellProps) {
  const [summary, setSummary] = useState<DiffSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!branch || !baseBranch) {
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    getDiffSummary(repo.path, baseBranch, branch, mode)
      .then((next) => {
        if (!cancelled) {
          setSummary(next);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setSummary(null);
          setError(typeof e === "string" ? e : String(e));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [repo.path, branch, baseBranch, mode, refreshKey]);

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
        <button
          type="button"
          className="refresh-button"
          title="Refresh branches and diff"
          onClick={onRefresh}
          disabled={loading}
        >
          ↻ Refresh
        </button>
      </header>
      <main className="diff-main">
        {error !== null ? (
          <div className="diff-empty">
            <p className="error">{error}</p>
          </div>
        ) : summary !== null ? (
          <FileList summary={summary} />
        ) : (
          <div className="diff-empty">
            <p>{loading ? "Computing diff…" : "Select branches to diff."}</p>
          </div>
        )}
      </main>
    </div>
  );
}

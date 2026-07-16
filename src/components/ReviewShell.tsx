import { useEffect, useRef, useState } from "react";
import { getDiffSummary } from "../ipc";
import type {
  BranchList,
  DiffSummary,
  RepoInfo,
  WorkingTreeMode,
} from "../types";
import { DiffView } from "./DiffView";
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

/**
 * A summary pinned to the parameters it was computed with, so the diff view
 * never fetches file hunks against refs the summary doesn't describe.
 */
interface DiffViewState {
  summary: DiffSummary;
  base: string;
  head: string;
  mode: WorkingTreeMode;
  /** Bumped per fetch; remounts DiffView so lazy-loaded hunks reset too. */
  generation: number;
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
  const [view, setView] = useState<DiffViewState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [scrollTarget, setScrollTarget] = useState<{
    path: string;
    nonce: number;
  } | null>(null);
  const generation = useRef(0);

  useEffect(() => {
    if (!branch || !baseBranch) {
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    getDiffSummary(repo.path, baseBranch, branch, mode)
      .then((summary) => {
        if (!cancelled) {
          generation.current += 1;
          setView({
            summary,
            base: baseBranch,
            head: branch,
            mode,
            generation: generation.current,
          });
          setScrollTarget(null);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setView(null);
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
        ) : view === null ? (
          <div className="diff-empty">
            <p>{loading ? "Computing diff…" : "Select branches to diff."}</p>
          </div>
        ) : view.summary.files.length === 0 ? (
          <div className="diff-empty">
            <p>
              No changes between <code>{view.summary.baseRef}</code> and{" "}
              <code>{view.summary.headRef}</code>.
            </p>
          </div>
        ) : (
          <div className="review-body">
            <aside className="file-sidebar">
              <FileList
                summary={view.summary}
                onSelect={(path) =>
                  setScrollTarget((prev) => ({
                    path,
                    nonce: (prev?.nonce ?? 0) + 1,
                  }))
                }
              />
            </aside>
            <DiffView
              key={view.generation}
              repoPath={repo.path}
              base={view.base}
              head={view.head}
              mode={view.mode}
              summary={view.summary}
              scrollTarget={scrollTarget}
            />
          </div>
        )}
      </main>
    </div>
  );
}

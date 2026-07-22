import { useEffect, useMemo, useState } from "react";
import type { CSSProperties } from "react";
import { useTauriEvent } from "../useTauriEvent";
import {
  MENU_SHOW_SHORTCUTS_EVENT,
  MENU_VIEW_ARCHIVED_EVENT,
} from "../generated/events";
import type { BranchList, RepoInfo, DiffMode } from "../types";
import { ArchivedReviews } from "./ArchivedReviews";
import { BranchSelect } from "./BranchSelect";
import { DiffView } from "./DiffView";
import { ExportMenu, type ExportTarget } from "./ExportMenu";
import { FileJump } from "./FileJump";
import { FileList } from "./FileList";
import { GuideMenu } from "./GuideMenu";
import { ModeToggle } from "./ModeToggle";
import { OrphanedComments } from "./OrphanedComments";
import { ReviewCommentsPanel } from "./ReviewCommentsPanel";
import { ShortcutHelp } from "./ShortcutHelp";
import { WhitespaceToggle } from "./WhitespaceToggle";
import { guideOrderedFiles } from "../diff/guideOrder";
import { useCommentMutations } from "./useCommentMutations";
import { useGuide } from "./useGuide";
import { useResizableSidebar } from "./useResizableSidebar";
import { useReviewDerived } from "./useReviewDerived";
import { useReviewSession } from "./useReviewSession";

interface ReviewShellProps {
  repo: RepoInfo;
  branchList: BranchList;
  branch: string;
  baseBranch: string;
  mode: DiffMode;
  hideWhitespace: boolean;
  hideResolved: boolean;
  refreshKey: number;
  onBranchChange: (branch: string) => void;
  onBaseBranchChange: (base: string) => void;
  onModeChange: (mode: DiffMode) => void;
  onHideWhitespaceChange: (hide: boolean) => void;
  onSwitchRepo: () => void;
}

/**
 * Layout for an open repo: toolbar, file sidebar, and the diff pane with
 * its comment surfaces. Fetching lives in useReviewSession and the mutation
 * callbacks in useCommentMutations; this component only derives the
 * per-surface slices and renders.
 */
export function ReviewShell({
  repo,
  branchList,
  branch,
  baseBranch,
  mode,
  hideWhitespace,
  hideResolved,
  refreshKey,
  onBranchChange,
  onBaseBranchChange,
  onModeChange,
  onHideWhitespaceChange,
  onSwitchRepo,
}: ReviewShellProps) {
  const session = useReviewSession(
    repo.path,
    branch,
    baseBranch,
    mode,
    hideWhitespace,
    refreshKey,
  );
  const { view, review, branchMerged, comments, anchorStatuses, error, loading } =
    session;
  const {
    reviewStates,
    repliesByRoot,
    reviewComments,
    orphanedComments,
    diffComments,
    openCount,
    openCounts,
  } = useReviewDerived(session, hideResolved);
  const guideState = useGuide(session);
  const {
    onToggleReviewed,
    onSetFilesReviewed,
    onCreate,
    onUpdate,
    onDelete,
    onCreateReply,
    onSetState,
    onCreateReviewComment,
  } = useCommentMutations(session);
  const sidebar = useResizableSidebar();

  const [showArchive, setShowArchive] = useState(false);
  const [showFileJump, setShowFileJump] = useState(false);
  const [showHelp, setShowHelp] = useState(false);
  // The click's view generation invalidates the target on refresh: DiffView
  // scrolls on mount for any non-null target (it remounts when the shell
  // swaps it out for an empty state and back), and it must never chase a
  // click made against a previous diff.
  const [scrollTarget, setScrollTarget] = useState<{
    path: string;
    nonce: number;
    generation: number;
  } | null>(null);

  // View > Archived Reviews… (native menu) — replaces the old toolbar button.
  useTauriEvent(MENU_VIEW_ARCHIVED_EVENT, () => {
    setShowFileJump(false);
    setShowArchive(true);
  });

  // Help > Keyboard Shortcuts — same overlay as the `?` key.
  useTauriEvent(MENU_SHOW_SHORTCUTS_EVENT, () => {
    setShowFileJump(false);
    setShowHelp(true);
  });

  // `?` toggles the shortcut cheat sheet; Esc closes it. Esc only reaches
  // here when the sheet is topmost: FileJump swallows its own Esc, and ⌘P
  // is suppressed while the sheet is up, so the two never stack.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) {
        return;
      }
      // Never hijack typing: composers, branch selects, the ⌘P palette.
      const target = e.target;
      if (
        target instanceof HTMLElement &&
        (target.tagName === "TEXTAREA" ||
          target.tagName === "INPUT" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }
      if (e.key === "?") {
        // Opening waits its turn behind the other overlays; closing with a
        // second `?` always works.
        if (!showHelp && (showFileJump || showArchive)) {
          return;
        }
        e.preventDefault();
        setShowHelp((open) => !open);
      } else if (e.key === "Escape" && showHelp) {
        e.preventDefault();
        setShowHelp(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [showHelp, showFileJump, showArchive]);

  // ⌘P (or Ctrl+P) opens the file-jump palette over the diff.
  const canJump =
    error === null &&
    view !== null &&
    review !== null &&
    !branchMerged &&
    view.summary.files.length > 0;
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (
        !(e.metaKey || e.ctrlKey) ||
        e.altKey ||
        e.shiftKey ||
        e.key.toLowerCase() !== "p"
      ) {
        return;
      }
      // Never hijack typing: composers, branch selects, the palette itself.
      const target = e.target;
      if (
        target instanceof HTMLElement &&
        (target.tagName === "TEXTAREA" ||
          target.tagName === "INPUT" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }
      if (!canJump || showArchive || showHelp) {
        return;
      }
      // Swallow the webview's print shortcut even when already open.
      e.preventDefault();
      setShowFileJump(true);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [canJump, showArchive, showHelp]);

  /** The summary the diff pane (and ⌘P) displays: while the guide view is
   * active, the same files reordered to follow the sidebar's section order —
   * both derive from useGuide's resolved sections, so the panes can never
   * disagree. Identical to `view.summary` (same object) otherwise, keeping
   * the non-guide path untouched. */
  const displaySummary = useMemo(() => {
    if (view === null || !guideState.grouped || guideState.sections === null) {
      return view?.summary ?? null;
    }
    return {
      ...view.summary,
      files: guideOrderedFiles(guideState.sections),
    };
  }, [view, guideState.grouped, guideState.sections]);

  /** What the Export menu would export: the displayed diff's pinned params
   * plus its active review; null (disabled) when there is neither. */
  const exportTarget = useMemo<ExportTarget | null>(
    () =>
      view !== null &&
      review !== null &&
      review.status === "active" &&
      !branchMerged
        ? {
            repoPath: repo.path,
            base: view.base,
            head: view.head,
            mode: view.mode,
            reviewId: review.id,
          }
        : null,
    [view, review, branchMerged, repo.path],
  );

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
        <div className="control">
          <span>Base</span>
          <BranchSelect
            value={baseBranch}
            branches={branchList.branches}
            onChange={onBaseBranchChange}
          />
        </div>
        <span className="arrow" aria-hidden="true">
          ←
        </span>
        <div className="control">
          <span>Branch</span>
          <BranchSelect
            value={branch}
            branches={branchList.branches}
            onChange={onBranchChange}
          />
        </div>
        <div className="toolbar-spacer" />
        <ModeToggle mode={mode} onChange={onModeChange} />
        <WhitespaceToggle
          hidden={hideWhitespace}
          onChange={onHideWhitespaceChange}
        />
        <GuideMenu
          {...guideState}
          hasTarget={exportTarget !== null}
          emptyDiff={view !== null && view.summary.files.length === 0}
        />
        <ExportMenu target={exportTarget} openCount={openCount} />
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
        ) : branchMerged ? (
          <div className="diff-empty">
            <p>
              <code>{branch}</code> is merged into <code>{baseBranch}</code>
              {review !== null
                ? " — its review is archived and read-only."
                : "."}
            </p>
            {review !== null && (
              <button
                type="button"
                className="refresh-button"
                onClick={() => setShowArchive(true)}
              >
                View archived reviews
              </button>
            )}
          </div>
        ) : review === null ? (
          <div className="diff-empty">
            <p>No review for this branch.</p>
          </div>
        ) : view.summary.files.length === 0 && comments.length === 0 ? (
          <div className="diff-empty">
            <p>
              No changes between <code>{view.summary.baseRef}</code> and{" "}
              <code>{view.summary.headRef}</code>.
            </p>
          </div>
        ) : (
          <div
            className="review-body"
            ref={sidebar.containerRef}
            style={
              { "--sidebar-width": `${sidebar.width}px` } as CSSProperties
            }
          >
            <aside className="file-sidebar">
              <FileList
                summary={view.summary}
                repoPath={repo.path}
                openCounts={openCounts}
                reviewStates={reviewStates}
                sections={guideState.sections}
                grouped={guideState.grouped}
                onToggleGrouped={guideState.onToggleGrouped}
                onSetFilesReviewed={onSetFilesReviewed}
                onSelect={(path) =>
                  setScrollTarget((prev) => ({
                    path,
                    nonce: (prev?.nonce ?? 0) + 1,
                    generation: view.generation,
                  }))
                }
              />
            </aside>
            <div
              className={
                sidebar.dragging
                  ? "sidebar-divider dragging"
                  : "sidebar-divider"
              }
              role="separator"
              aria-orientation="vertical"
              aria-label="Resize sidebar"
              {...sidebar.dividerProps}
            />
            <div className="diff-pane">
              <ReviewCommentsPanel
                comments={reviewComments}
                repliesByRoot={repliesByRoot}
                onCreate={onCreateReviewComment}
                onCreateReply={onCreateReply}
                onUpdate={onUpdate}
                onDelete={onDelete}
                onSetState={onSetState}
              />
              <DiffView
                topContent={
                  <OrphanedComments
                    comments={orphanedComments}
                    repliesByRoot={repliesByRoot}
                    onCreateReply={onCreateReply}
                    onUpdate={onUpdate}
                    onDelete={onDelete}
                    onSetState={onSetState}
                  />
                }
                repoPath={repo.path}
                base={view.base}
                head={view.head}
                mode={view.mode}
                ignoreWhitespace={view.ignoreWhitespace}
                summary={displaySummary ?? view.summary}
                scrollTarget={
                  scrollTarget?.generation === view.generation
                    ? scrollTarget
                    : null
                }
                comments={diffComments}
                replies={repliesByRoot}
                anchorStatuses={anchorStatuses}
                reviewStates={reviewStates}
                onToggleReviewed={onToggleReviewed}
                onCreateComment={onCreate}
                onUpdateComment={onUpdate}
                onDeleteComment={onDelete}
                onSetCommentState={onSetState}
              />
            </div>
          </div>
        )}
      </main>
      {showFileJump && view !== null && (
        <FileJump
          files={(displaySummary ?? view.summary).files}
          onSelect={(path) => {
            setShowFileJump(false);
            setScrollTarget((prev) => ({
              path,
              nonce: (prev?.nonce ?? 0) + 1,
              generation: view.generation,
            }));
          }}
          onClose={() => setShowFileJump(false)}
        />
      )}
      {showArchive && (
        <ArchivedReviews
          repoPath={repo.path}
          onClose={() => setShowArchive(false)}
        />
      )}
      {showHelp && <ShortcutHelp onClose={() => setShowHelp(false)} />}
    </div>
  );
}

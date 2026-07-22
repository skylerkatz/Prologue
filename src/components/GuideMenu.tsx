import { useEffect, useRef, useState } from "react";
import type { GuideState } from "./useGuide";

interface GuideMenuProps extends GuideState {
  /** An active review is displayed (same gate as Export). */
  hasTarget: boolean;
  /** The displayed diff has no changed files — nothing to guide. */
  emptyDiff: boolean;
}

/**
 * Toolbar dropdown for the review guide: generate (runs the user's `claude`
 * CLI over the diff), cancel while running, and — once a guide exists — the
 * grouped/flat sidebar toggle. Generation is explicit consent: pressing
 * Generate is what sends the diff to Anthropic, so the tooltip says so.
 */
export function GuideMenu({
  hasTarget,
  emptyDiff,
  guide,
  isStale,
  generating,
  error,
  clearError,
  cliAvailable,
  grouped,
  onToggleGrouped,
  onGenerate,
  onCancel,
}: GuideMenuProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    const closeOnOutsideClick = (e: MouseEvent) => {
      if (
        rootRef.current !== null &&
        !rootRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", closeOnOutsideClick);
    return () => document.removeEventListener("mousedown", closeOnOutsideClick);
  }, [open]);

  // Backend failure messages are user-readable; show them verbatim and let
  // them linger longer than a copy confirmation would.
  useEffect(() => {
    if (error === null) {
      return;
    }
    const timer = window.setTimeout(clearError, 8000);
    return () => window.clearTimeout(timer);
  }, [error, clearError]);

  // With no guide yet, the button is only useful when Generate could run;
  // once a guide (or a run) exists it stays enabled for Cancel/the toggle.
  const disabled =
    !generating && guide === null && (!hasTarget || emptyDiff || !cliAvailable);
  const title = generating
    ? "Generating the review guide…"
    : !hasTarget
      ? "Guides need an active review"
      : emptyDiff
        ? "No changes to guide"
        : !cliAvailable && guide === null
          ? "Install Claude Code to generate guides"
          : guide !== null
            ? isStale
              ? "Review guide options — the diff has changed since the guide was generated"
              : "Review guide options"
            : "Generate a review guide — sends the diff to Anthropic through your Claude Code account";
  // Regenerate rides the exact gates Generate does; a stored guide keeps
  // the button itself enabled, so the item carries its own reason.
  const regenerateBlocked = !hasTarget
    ? "Guides need an active review"
    : emptyDiff
      ? "No changes to guide"
      : !cliAvailable
        ? "Install Claude Code to generate guides"
        : null;

  return (
    <div className="export-menu" ref={rootRef}>
      <button
        type="button"
        className="refresh-button"
        disabled={disabled}
        title={title}
        onClick={() => setOpen((prev) => !prev)}
      >
        {generating ? "Generating…" : "Guide ▾"}
        {!generating && guide !== null && isStale && (
          <span className="guide-stale-dot" aria-hidden="true" />
        )}
      </button>
      {open && (
        <div className="export-options" role="menu">
          {generating ? (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setOpen(false);
                onCancel();
              }}
            >
              Cancel
            </button>
          ) : guide !== null ? (
            <>
              {isStale && (
                <p className="menu-note stale" role="status">
                  Guide is stale — the diff has changed since it was
                  generated.
                </p>
              )}
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setOpen(false);
                  onToggleGrouped();
                }}
              >
                {grouped ? "Show flat file list" : "Group files by section"}
              </button>
              <button
                type="button"
                role="menuitem"
                disabled={regenerateBlocked !== null}
                title={
                  regenerateBlocked ??
                  "Replaces the guide — sends the diff to Anthropic through your Claude Code account"
                }
                onClick={() => {
                  setOpen(false);
                  onGenerate();
                }}
              >
                Regenerate guide
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setOpen(false);
                  onGenerate();
                }}
              >
                Generate guide
              </button>
              <p className="menu-note">
                Runs your <code>claude</code> CLI — the diff is sent to
                Anthropic through your account.
              </p>
            </>
          )}
        </div>
      )}
      {error !== null && (
        <div className="copy-toast error" role="status">
          {error}
        </div>
      )}
    </div>
  );
}

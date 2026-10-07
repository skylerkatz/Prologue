import { useEffect } from "react";
import type { GuideState } from "./useGuide";

type GuideButtonProps = Pick<
  GuideState,
  | "guide"
  | "isStale"
  | "generating"
  | "error"
  | "clearError"
  | "cliAvailable"
  | "onGenerate"
> & {
  /** An active review is displayed (same gate as Export). */
  hasTarget: boolean;
  /** The displayed diff has no changed files — nothing to guide. */
  emptyDiff: boolean;
};

const CONSENT = "sends the diff to Anthropic through your Claude Code account";

/**
 * Toolbar button for the review guide: click generates (or regenerates) it by
 * running the user's `claude` CLI over the diff; disabled while that runs,
 * and Esc cancels (ReviewShell's global keydown). Generation is explicit
 * consent: clicking is what sends the diff to Anthropic, so the tooltip says
 * so.
 */
export function GuideButton({
  hasTarget,
  emptyDiff,
  guide,
  isStale,
  generating,
  error,
  clearError,
  cliAvailable,
  onGenerate,
}: GuideButtonProps) {
  // Backend failure messages are user-readable; show them verbatim and let
  // them linger longer than a copy confirmation would.
  useEffect(() => {
    if (error === null) {
      return;
    }
    const timer = window.setTimeout(clearError, 8000);
    return () => window.clearTimeout(timer);
  }, [error, clearError]);

  // Any gate disables the button, and its tooltip says why.
  const blocked = generating
    ? "Generating the review guide — press Esc to cancel"
    : !hasTarget
      ? "Guides need an active review"
      : emptyDiff
        ? "No changes to guide"
        : !cliAvailable
          ? "Install Claude Code to generate guides"
          : null;
  const title =
    blocked ??
    (guide === null
      ? `Generate a review guide — ${CONSENT}`
      : isStale
        ? `The diff has changed since the guide was generated — click to regenerate (${CONSENT})`
        : `Regenerate the review guide — ${CONSENT}`);

  return (
    <>
      <button
        type="button"
        className="refresh-button"
        disabled={blocked !== null}
        title={title}
        onClick={onGenerate}
      >
        {generating ? "Generating…" : "Guide"}
        {!generating && isStale && (
          <span className="guide-stale-dot" aria-hidden="true" />
        )}
      </button>
      {error !== null && (
        <div className="copy-toast error" role="status">
          {error}
        </div>
      )}
    </>
  );
}

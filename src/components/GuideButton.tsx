import { useEffect } from "react";
import type { GuideEngine } from "../ipc";
import type { GuideState } from "./useGuide";
import { Toast } from "./useToast";

type GuideButtonProps = Pick<
  GuideState,
  | "guide"
  | "isStale"
  | "generating"
  | "error"
  | "clearError"
  | "engine"
  | "onGenerate"
> & {
  /** An active review is displayed (same gate as Export). */
  hasTarget: boolean;
  /** The displayed diff has no changed files — nothing to guide. */
  emptyDiff: boolean;
};

/** Where clicking sends the diff, per engine. */
const CONSENT: Record<GuideEngine, string> = {
  "claude-code": "sends the diff to Anthropic through your Claude Code account",
  amp: "sends the diff to Amp through your Amp account",
};

/**
 * Toolbar button for the review guide: click generates (or regenerates) it by
 * running the user's `claude` or `amp` CLI over the diff; disabled while that
 * runs, and Esc cancels (ReviewShell's global keydown). Generation is
 * explicit consent: clicking is what sends the diff to Anthropic or Amp, so
 * the tooltip names which one.
 */
export function GuideButton({
  hasTarget,
  emptyDiff,
  guide,
  isStale,
  generating,
  error,
  clearError,
  engine,
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
        : engine === null
          ? "Install Claude Code or Amp to generate guides"
          : null;
  const consent = engine === null ? "" : CONSENT[engine];
  const title =
    blocked ??
    (guide === null
      ? `Generate a review guide — ${consent}`
      : isStale
        ? `The diff has changed since the guide was generated — click to regenerate (${consent})`
        : `Regenerate the review guide — ${consent}`);

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
      {error !== null && <Toast text={error} error />}
    </>
  );
}

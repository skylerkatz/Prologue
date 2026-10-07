import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { errorText, exportReview } from "../ipc";
import type { DiffMode } from "../types";
import { Toast, useToast } from "./useToast";

/** The displayed diff the export must describe; null when there is nothing
 * exportable (no active review). */
export interface ExportTarget {
  repoPath: string;
  base: string;
  head: string;
  mode: DiffMode;
  reviewId: number;
}

interface ExportButtonProps {
  target: ExportTarget | null;
  /** Open comments on the review; zero disables the button. */
  openCount: number;
}

/**
 * Toolbar button copying the review's open comments to the clipboard as
 * JSON, or with the agent prompt prepended on ⌘-click. Formatting happens in
 * Rust; this only invokes, copies, and confirms with a toast.
 */
export function ExportButton({ target, openCount }: ExportButtonProps) {
  const { toast, showToast } = useToast();

  const copy = async (withPrompt: boolean) => {
    if (target === null) {
      return;
    }
    try {
      const text = await exportReview(
        target.repoPath,
        target.base,
        target.head,
        target.mode,
        target.reviewId,
        withPrompt ? "prompt-json" : "json",
      );
      await writeText(text);
      showToast(
        `Copied ${withPrompt ? "agent prompt + JSON" : "JSON"} to clipboard`,
        2500,
      );
    } catch (e) {
      showToast(errorText(e), 5000, true);
    }
  };

  const disabled = target === null || openCount === 0;
  return (
    <>
      <button
        type="button"
        className="refresh-button"
        disabled={disabled}
        title={
          target === null
            ? "Exporting needs an active review"
            : openCount === 0
              ? "No open comments to export"
              : "Copy open comments as JSON · ⌘-click to include the agent prompt"
        }
        // ⌘ only: on macOS Ctrl-click is a context-menu click.
        onClick={(e) => void copy(e.metaKey)}
      >
        Export
      </button>
      {toast !== null && <Toast {...toast} />}
    </>
  );
}

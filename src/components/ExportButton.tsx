import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { errorText, exportReview } from "../ipc";
import type { DiffMode, ExportFormat } from "../types";
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

type ExportKind = "json" | "prompt" | "remote-prompt";

const EXPORTS: Record<ExportKind, { format: ExportFormat; label: string }> = {
  json: { format: "json", label: "JSON" },
  prompt: { format: "prompt-json", label: "agent prompt + JSON" },
  // For agents that can't reach this Mac (e.g. Amp orbs): they answer with
  // a prologue-replies block for `prologue import-replies`.
  "remote-prompt": {
    format: "remote-prompt-json",
    label: "remote-agent prompt + JSON",
  },
};

/**
 * Toolbar button copying the review's open comments to the clipboard as
 * JSON, with the agent prompt prepended on ⌘-click, or with the remote-agent
 * prompt on ⇧-click. Formatting happens in Rust; this only invokes, copies,
 * and confirms with a toast.
 */
export function ExportButton({ target, openCount }: ExportButtonProps) {
  const { toast, showToast } = useToast();

  const copy = async (kind: ExportKind) => {
    if (target === null) {
      return;
    }
    const { format, label } = EXPORTS[kind];
    try {
      const text = await exportReview(
        target.repoPath,
        target.base,
        target.head,
        target.mode,
        target.reviewId,
        format,
      );
      await writeText(text);
      showToast(`Copied ${label} to clipboard`, 2500);
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
              : "Copy open comments as JSON · ⌘-click to include the agent prompt · ⇧-click for a remote agent (replies come back via prologue import-replies)"
        }
        // ⌘/⇧ only: on macOS Ctrl-click is a context-menu click.
        onClick={(e) =>
          void copy(
            e.shiftKey ? "remote-prompt" : e.metaKey ? "prompt" : "json",
          )
        }
      >
        Export
      </button>
      {toast !== null && <Toast {...toast} />}
    </>
  );
}

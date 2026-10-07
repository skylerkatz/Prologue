import { useEffect, useRef, useState } from "react";
import type { MouseEvent } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { errorText, exportReview } from "../ipc";
import type { ExportFormat, DiffMode } from "../types";

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
  const [toast, setToast] = useState<{ text: string; error: boolean } | null>(
    null,
  );
  const toastTimer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(toastTimer.current), []);

  const showToast = (text: string, error: boolean) => {
    window.clearTimeout(toastTimer.current);
    setToast({ text, error });
    toastTimer.current = window.setTimeout(
      () => setToast(null),
      error ? 5000 : 2500,
    );
  };

  const copy = async (format: ExportFormat, label: string) => {
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
        format,
      );
      await writeText(text);
      showToast(`Copied ${label} to clipboard`, false);
    } catch (e) {
      showToast(errorText(e), true);
    }
  };

  // ⌘ only: on macOS Ctrl-click is a context-menu click.
  const onClick = (e: MouseEvent<HTMLButtonElement>) =>
    void (e.metaKey
      ? copy("prompt-json", "agent prompt + JSON")
      : copy("json", "JSON"));

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
        onClick={onClick}
      >
        Export
      </button>
      {toast !== null && (
        <div
          className={toast.error ? "copy-toast error" : "copy-toast"}
          role="status"
        >
          {toast.text}
        </div>
      )}
    </>
  );
}

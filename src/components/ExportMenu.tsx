import { useEffect, useRef, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { exportReview } from "../ipc";
import type { ExportFormat, WorkingTreeMode } from "../types";

const OPTIONS: ReadonlyArray<{ format: ExportFormat; label: string }> = [
  { format: "markdown", label: "Markdown" },
  { format: "json", label: "JSON" },
  { format: "prompt-markdown", label: "Agent prompt + Markdown" },
  { format: "prompt-json", label: "Agent prompt + JSON" },
];

/** The displayed diff the export must describe; null when there is nothing
 * exportable (no active review). */
export interface ExportTarget {
  repoPath: string;
  base: string;
  head: string;
  mode: WorkingTreeMode;
  reviewId: number;
}

interface ExportMenuProps {
  target: ExportTarget | null;
  /** Open comments on the review; zero disables the menu. */
  openCount: number;
}

/**
 * Toolbar dropdown copying the review's open comments to the clipboard in
 * one of the four export formats. Formatting happens in Rust; this only
 * invokes, copies, and confirms with a toast.
 */
export function ExportMenu({ target, openCount }: ExportMenuProps) {
  const [open, setOpen] = useState(false);
  const [toast, setToast] = useState<{ text: string; error: boolean } | null>(
    null,
  );
  const rootRef = useRef<HTMLDivElement>(null);
  const toastTimer = useRef<number | undefined>(undefined);

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
    setOpen(false);
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
      showToast(typeof e === "string" ? e : String(e), true);
    }
  };

  const disabled = target === null || openCount === 0;
  return (
    <div className="export-menu" ref={rootRef}>
      <button
        type="button"
        className="refresh-button"
        disabled={disabled}
        title={
          target === null
            ? "Exporting needs an active review"
            : openCount === 0
              ? "No open comments to export"
              : "Copy the review's open comments to the clipboard"
        }
        onClick={() => setOpen((prev) => !prev)}
      >
        Export ▾
      </button>
      {open && (
        <div className="export-options" role="menu">
          {OPTIONS.map(({ format, label }) => (
            <button
              key={format}
              type="button"
              role="menuitem"
              onClick={() => void copy(format, label)}
            >
              {label}
            </button>
          ))}
        </div>
      )}
      {toast !== null && (
        <div
          className={toast.error ? "copy-toast error" : "copy-toast"}
          role="status"
        >
          {toast.text}
        </div>
      )}
    </div>
  );
}

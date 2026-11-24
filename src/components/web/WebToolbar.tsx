import type { ReactNode } from "react";

interface WebToolbarProps {
  /** The diff's base picker. */
  base: ReactNode;
  /** How the diff is computed: working-tree mode, whitespace, guide. */
  viewControls: ReactNode;
  /** Send to Amp (or Export, on a server without a webhook). */
  send: ReactNode;
  /** Narrow layouts keep the file list in a drawer; this opens it. */
  onOpenFiles: (() => void) | null;
  fileCount: number | null;
}

/**
 * The web UI's header. One row at full width; in a narrow Portal pane the
 * view controls drop to a second row, and a Files button for the drawer
 * leads the first.
 */
export function WebToolbar({
  base,
  viewControls,
  send,
  onOpenFiles,
  fileCount,
}: WebToolbarProps) {
  return (
    <header className="web-toolbar">
      {onOpenFiles !== null && (
        <button
          type="button"
          className="web-files-button"
          onClick={onOpenFiles}
          aria-label="Show changed files"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <path d="M9 3v18" />
          </svg>
          Files
          {fileCount !== null && (
            <span className="web-files-count">{fileCount}</span>
          )}
        </button>
      )}
      {base}
      <div className="web-toolbar-view">{viewControls}</div>
      <div className="toolbar-spacer" />
      {send}
    </header>
  );
}

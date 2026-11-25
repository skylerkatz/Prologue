import { useEffect, useRef, useState } from "react";
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
  /** Phone widths: one slim row (Files, Send, ⋯) with everything else,
   * plus `onAddReviewComment`, in the ⋯ menu. */
  compact?: boolean;
  /** Compact only: slid out of view while the diff scrolls down. */
  hidden?: boolean;
  onAddReviewComment?: (() => void) | null;
}

/**
 * The web UI's header. One row at full width; in a narrow Portal pane the
 * view controls drop to a second row, and a Files button for the drawer
 * leads the first. On a phone it collapses to one slim row and a menu.
 */
export function WebToolbar({
  base,
  viewControls,
  send,
  onOpenFiles,
  fileCount,
  compact = false,
  hidden = false,
  onAddReviewComment = null,
}: WebToolbarProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);

  // Outside taps and Esc close the menu. The document listener runs before
  // ReviewShell's window one; preventDefault marks the Esc as taken.
  useEffect(() => {
    if (!menuOpen) {
      return;
    }
    const onPointerDown = (e: PointerEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        e.preventDefault();
        setMenuOpen(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [menuOpen]);

  useEffect(() => {
    if (!compact) {
      setMenuOpen(false);
    }
  }, [compact]);

  const filesButton = onOpenFiles !== null && (
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
  );

  if (compact) {
    return (
      <header
        className={hidden && !menuOpen ? "web-toolbar compact hidden" : "web-toolbar compact"}
      >
        {filesButton}
        <div className="toolbar-spacer" />
        {send}
        <div className="web-menu" ref={menuRef}>
          <button
            type="button"
            className="web-menu-button"
            aria-label="More review options"
            aria-haspopup="true"
            aria-expanded={menuOpen}
            onClick={() => setMenuOpen((open) => !open)}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true">
              <circle cx="5" cy="12" r="1.8" fill="currentColor" />
              <circle cx="12" cy="12" r="1.8" fill="currentColor" />
              <circle cx="19" cy="12" r="1.8" fill="currentColor" />
            </svg>
          </button>
          {menuOpen && (
            <div className="web-menu-panel" role="group" aria-label="Review options">
              {base}
              {viewControls}
              {onAddReviewComment !== null && (
                <button
                  type="button"
                  className="refresh-button"
                  onClick={() => {
                    setMenuOpen(false);
                    onAddReviewComment();
                  }}
                >
                  + Add review comment
                </button>
              )}
            </div>
          )}
        </div>
      </header>
    );
  }

  return (
    <header className="web-toolbar">
      {filesButton}
      {base}
      <div className="web-toolbar-view">{viewControls}</div>
      <div className="toolbar-spacer" />
      {send}
    </header>
  );
}

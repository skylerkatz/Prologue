import { useEffect, useState } from "react";
import type { ReactNode } from "react";

/** Below this width (a Portal pane beside the transcript) the file list
 * moves into a drawer so the diff keeps the whole width. */
export const NARROW_LAYOUT_QUERY = "(max-width: 899px)";

/** Phone widths: the code gets the screen; the toolbar collapses to one
 * row that slides away while scrolling down. */
export const PHONE_LAYOUT_QUERY = "(max-width: 599px)";

export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(
    () => window.matchMedia(query).matches,
  );
  useEffect(() => {
    const list = window.matchMedia(query);
    const update = () => setMatches(list.matches);
    update();
    list.addEventListener("change", update);
    return () => list.removeEventListener("change", update);
  }, [query]);
  return matches;
}

interface WebFileDrawerProps {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
}

/**
 * Slide-over panel holding the file list (and guide) at narrow widths. It
 * stays mounted while closed so the list keeps its scroll position and
 * toggles; Esc and the scrim close it.
 */
export function WebFileDrawer({ open, onClose, children }: WebFileDrawerProps) {
  // A document listener runs before ReviewShell's window one, and
  // preventDefault tells that one this Esc is taken.
  useEffect(() => {
    if (!open) {
      return;
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        e.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  return (
    <div className={open ? "web-drawer open" : "web-drawer"} inert={!open}>
      <div className="web-drawer-scrim" onClick={onClose} />
      <aside
        className="web-drawer-panel"
        role="dialog"
        aria-label="Changed files"
      >
        <div className="web-drawer-header">
          <span>Changed files</span>
          <button
            type="button"
            className="web-drawer-close"
            onClick={onClose}
            aria-label="Close file list"
          >
            ×
          </button>
        </div>
        <div className="web-drawer-body">{children}</div>
      </aside>
    </div>
  );
}

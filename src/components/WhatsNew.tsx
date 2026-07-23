import { useEffect } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import releaseNotes from "../release-notes.json";

interface ReleaseNote {
  version: string;
  date: string;
  notes: string;
}

// Newest-first; scripts/release.sh requires an entry per released version,
// so the top entry is normally the running version.
const NOTES = releaseNotes as ReleaseNote[];

/** Whether the bundled notes cover a version — App gates auto-show on it. */
export function hasNotesFor(version: string): boolean {
  return NOTES.some((n) => n.version === version);
}

interface WhatsNewProps {
  onClose: () => void;
}

/**
 * Release notes for the running version and its predecessors, reachable
 * via Prologue > What's New… and shown once automatically after an
 * update. Read-only overlay in the ArchivedReviews mold.
 */
export function WhatsNew({ onClose }: WhatsNewProps) {
  useEffect(() => {
    const closeOnEscape = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  return (
    <div
      className="whats-new-overlay"
      role="dialog"
      aria-label="What's new"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          onClose();
        }
      }}
    >
      <div className="whats-new-panel">
        <header className="whats-new-header">
          <span className="whats-new-title">What's new</span>
          <span className="comment-header-spacer" />
          <button type="button" className="comment-action" onClick={onClose}>
            Close
          </button>
        </header>
        {NOTES.map((entry) => (
          <section key={entry.version} className="whats-new-entry">
            <h3 className="whats-new-version">
              v{entry.version}
              <span className="whats-new-date">{entry.date}</span>
            </h3>
            <div className="markdown-preview whats-new-notes">
              <Markdown remarkPlugins={[remarkGfm]}>{entry.notes}</Markdown>
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}

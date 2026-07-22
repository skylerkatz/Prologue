import { useEffect, useMemo, useRef, useState } from "react";
import type { FileStatus, FileSummary } from "../types";
import { matchFiles } from "../diff/fileMatch";

const STATUS_LABELS: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
};

interface FileJumpProps {
  files: readonly FileSummary[];
  onSelect: (path: string) => void;
  onClose: () => void;
}

/**
 * ⌘P overlay: a floating palette over the diff pane that fuzzy-matches the
 * changed files and jumps to the picked one. Same interaction conventions
 * as BranchSelect: ↑/↓ move, ↩ picks, Esc closes, outside mousedown
 * dismisses. Mounted only while open; ReviewShell owns the shortcut.
 */
export function FileJump({ files, onSelect, onClose }: FileJumpProps) {
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const matches = useMemo(() => matchFiles(files, query), [files, query]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    const closeOnOutsideClick = (e: MouseEvent) => {
      if (
        rootRef.current !== null &&
        !rootRef.current.contains(e.target as Node)
      ) {
        onClose();
      }
    };
    document.addEventListener("mousedown", closeOnOutsideClick);
    return () => document.removeEventListener("mousedown", closeOnOutsideClick);
  }, [onClose]);

  useEffect(() => {
    listRef.current?.children[highlight]?.scrollIntoView({ block: "nearest" });
  }, [highlight, matches]);

  const onInputKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlight((h) => Math.min(h + 1, matches.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlight((h) => Math.max(h - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const picked = matches[highlight];
      if (picked !== undefined) {
        onSelect(picked.path);
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      // Keep Esc local to the palette (the archive overlay also listens).
      e.stopPropagation();
      onClose();
    }
  };

  return (
    <div className="file-jump" ref={rootRef}>
      <input
        ref={inputRef}
        type="text"
        className="file-jump-filter"
        placeholder="Jump to changed file…"
        value={query}
        onChange={(e) => {
          setQuery(e.currentTarget.value);
          setHighlight(0);
        }}
        onKeyDown={onInputKeyDown}
      />
      <div className="file-jump-list" role="listbox" ref={listRef}>
        {matches.map((f, i) => (
          <button
            key={f.path}
            type="button"
            role="option"
            aria-selected={i === highlight}
            className={
              "file-jump-option" + (i === highlight ? " highlighted" : "")
            }
            title={f.path}
            onMouseEnter={() => setHighlight(i)}
            onClick={() => onSelect(f.path)}
          >
            <span
              className={`status-badge status-${f.status}`}
              title={f.status}
              aria-label={f.status}
            >
              {STATUS_LABELS[f.status]}
            </span>
            {/* Left-truncating path, same rtl+bdi pattern as FileList rows. */}
            <span className="file-path">
              <bdi>{f.path}</bdi>
            </span>
            {f.binary ? (
              <span className="file-counts file-binary">BIN</span>
            ) : (
              <span className="file-counts">
                <span className="count-added">+{f.additions}</span>
                <span className="count-deleted">−{f.deletions}</span>
              </span>
            )}
          </button>
        ))}
        {matches.length === 0 && (
          <p className="file-jump-empty">No files match.</p>
        )}
      </div>
    </div>
  );
}

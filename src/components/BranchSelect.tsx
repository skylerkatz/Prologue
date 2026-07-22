import { useEffect, useMemo, useRef, useState } from "react";

interface BranchSelectProps {
  /** Currently selected branch (shown on the trigger). */
  value: string;
  branches: readonly string[];
  onChange: (branch: string) => void;
}

/**
 * Searchable replacement for the toolbar branch <select>: a mono trigger
 * button opening a menu with a filter input and a keyboard-navigable list.
 * Filtering is case-insensitive substring; ↑/↓ move, ↩ picks, Esc closes.
 */
export function BranchSelect({ value, branches, onChange }: BranchSelectProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle === "") {
      return branches;
    }
    return branches.filter((b) => b.toLowerCase().includes(needle));
  }, [branches, query]);

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

  useEffect(() => {
    if (open) {
      inputRef.current?.focus();
    }
  }, [open]);

  useEffect(() => {
    listRef.current?.children[highlight]?.scrollIntoView({ block: "nearest" });
  }, [highlight, filtered]);

  const openMenu = () => {
    const at = branches.indexOf(value);
    setQuery("");
    setHighlight(at === -1 ? 0 : at);
    setOpen(true);
  };

  const pick = (branch: string) => {
    setOpen(false);
    buttonRef.current?.focus();
    if (branch !== value) {
      onChange(branch);
    }
  };

  const onInputKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlight((h) => Math.min(h + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlight((h) => Math.max(h - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const branch = filtered[highlight];
      if (branch !== undefined) {
        pick(branch);
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      // Keep Esc local to the menu (the archive overlay also listens).
      e.stopPropagation();
      setOpen(false);
      buttonRef.current?.focus();
    }
  };

  return (
    <div className="branch-select" ref={rootRef}>
      <button
        type="button"
        ref={buttonRef}
        className="branch-select-button"
        title={value}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => (open ? setOpen(false) : openMenu())}
      >
        <span className="branch-select-value">{value}</span>
        <span aria-hidden="true">▾</span>
      </button>
      {open && (
        <div className="branch-select-menu">
          <input
            ref={inputRef}
            type="text"
            className="branch-select-filter"
            placeholder="Filter branches…"
            value={query}
            onChange={(e) => {
              setQuery(e.currentTarget.value);
              setHighlight(0);
            }}
            onKeyDown={onInputKeyDown}
          />
          <div className="branch-select-list" role="listbox" ref={listRef}>
            {filtered.map((b, i) => (
              <button
                key={b}
                type="button"
                role="option"
                aria-selected={b === value}
                className={[
                  "branch-select-option",
                  b === value ? "current" : "",
                  i === highlight ? "highlighted" : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
                title={b}
                onMouseEnter={() => setHighlight(i)}
                onClick={() => pick(b)}
              >
                {b}
              </button>
            ))}
            {filtered.length === 0 && (
              <p className="branch-select-empty">No branches match.</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

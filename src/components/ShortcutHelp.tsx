import { useEffect, useRef } from "react";

interface ShortcutRow {
  keys: string[];
  label: string;
}

const SECTIONS: { title: string; rows: ShortcutRow[] }[] = [
  {
    title: "Navigate",
    rows: [
      { keys: ["j", "k"], label: "Next / previous file" },
      { keys: ["J", "K"], label: "Next / previous unviewed file" },
      { keys: ["n", "p"], label: "Next / previous open comment" },
      { keys: ["⌘P"], label: "Jump to a changed file" },
    ],
  },
  {
    title: "Review",
    rows: [
      { keys: ["c"], label: "Comment on the selected lines or current file" },
      { keys: ["v"], label: "Toggle viewed on the current file" },
      { keys: ["⌘↩"], label: "Submit the comment you're writing" },
      { keys: ["Esc"], label: "Cancel the comment you're writing" },
    ],
  },
  {
    title: "View",
    rows: [
      { keys: ["⌘R"], label: "Refresh" },
      { keys: ["⇧⌘A"], label: "Archived reviews" },
      { keys: ["⇧⌘H"], label: "Hide resolved comments" },
      { keys: ["?"], label: "Show or hide these shortcuts" },
    ],
  },
];

interface ShortcutHelpProps {
  onClose: () => void;
}

/**
 * The `?` cheat sheet: a floating card listing every keyboard binding.
 * Also reachable via Help > Keyboard Shortcuts. ReviewShell owns the
 * toggle (`?` / Esc / the menu event); here only outside mousedown
 * dismisses, like FileJump.
 */
export function ShortcutHelp({ onClose }: ShortcutHelpProps) {
  const rootRef = useRef<HTMLDivElement>(null);

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

  return (
    <div
      className="shortcut-help"
      role="dialog"
      aria-label="Keyboard shortcuts"
      ref={rootRef}
    >
      <header className="shortcut-help-header">
        <span className="shortcut-help-title">Keyboard shortcuts</span>
        <span className="comment-header-spacer" />
        <button type="button" className="comment-action" onClick={onClose}>
          Close
        </button>
      </header>
      {SECTIONS.map((section) => (
        <section key={section.title}>
          <h3 className="shortcut-section-title">{section.title}</h3>
          {section.rows.map((row) => (
            <div key={row.label} className="shortcut-row">
              <span className="shortcut-keys">
                {row.keys.map((key) => (
                  <kbd key={key}>{key}</kbd>
                ))}
              </span>
              <span className="shortcut-label">{row.label}</span>
            </div>
          ))}
        </section>
      ))}
    </div>
  );
}

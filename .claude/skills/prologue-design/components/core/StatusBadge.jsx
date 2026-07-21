import React from "react";

const STATUS = {
  added:    { letter: "A", bg: "var(--badge-added-bg)", color: "var(--badge-added-text)" },
  modified: { letter: "M", bg: "var(--badge-modified-bg)", color: "var(--badge-modified-text)" },
  deleted:  { letter: "D", bg: "var(--badge-deleted-bg)", color: "var(--badge-deleted-text)" },
  renamed:  { letter: "R", bg: "var(--badge-renamed-bg)", color: "var(--badge-renamed-text)" },
};

/** File-status letter badge (A/M/D/R) — tinted square, colored letter. */
export function StatusBadge({ status = "modified" }) {
  const s = STATUS[status] ?? STATUS.modified;
  return (
    <span title={status} aria-label={status} style={{
      flex: "none", width: 20, height: 20, display: "inline-flex", alignItems: "center", justifyContent: "center",
      borderRadius: "var(--radius-sm)", background: s.bg, color: s.color,
      fontFamily: "var(--font-mono)", fontSize: 11.5, fontWeight: 700,
    }}>{s.letter}</span>
  );
}

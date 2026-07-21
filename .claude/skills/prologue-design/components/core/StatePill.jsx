import React from "react";

const KINDS = {
  resolved:  { label: "Resolved",  color: "var(--added)",     border: "var(--added)" },
  dismissed: { label: "Dismissed", color: "var(--text-muted)", border: "var(--border-strong)" },
  readonly:  { label: "read-only", color: "var(--text-muted)", border: "var(--border-strong)" },
  changed:   { label: "\u26a0 code changed since commented", color: "var(--renamed)", border: "var(--renamed)" },
};

/** Small outlined pill for comment lifecycle + archive states. */
export function StatePill({ kind = "resolved", title }) {
  const k = KINDS[kind] ?? KINDS.resolved;
  return (
    <span title={title} style={{
      fontFamily: "var(--font-sans)", fontSize: 11, fontWeight: 600, whiteSpace: "nowrap",
      padding: "1px 8px", borderRadius: 999, border: `1px solid ${k.border}`,
      color: k.color, background: "var(--bg-card)",
    }}>{k.label}</span>
  );
}

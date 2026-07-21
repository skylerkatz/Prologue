import React from "react";

/** Segmented working-tree mode toggle. Selected segment shows an orange dot. */
export function ModeToggle({ value, options = [
  { value: "committed", label: "Committed only" },
  { value: "staged", label: "Include staged" },
  { value: "all", label: "Staged + unstaged" },
], onChange }) {
  return (
    <div role="radiogroup" style={{ display: "flex", border: "1px solid var(--border-strong)", borderRadius: "var(--radius-md)", overflow: "hidden", background: "var(--bg-subtle)" }}>
      {options.map((o, i) => {
        const selected = o.value === value;
        return (
          <button key={o.value} type="button" role="radio" aria-checked={selected}
            onClick={() => onChange?.(o.value)}
            style={{
              display: "flex", alignItems: "center", gap: 6,
              fontFamily: "var(--font-sans)", fontSize: 12.5, fontWeight: selected ? 600 : 400,
              color: selected ? "var(--text-strong)" : "var(--text-muted)",
              background: selected ? "var(--bg-card)" : "transparent",
              border: "none", borderLeft: i > 0 ? "1px solid var(--border)" : "none",
              padding: "5px 12px", cursor: "pointer", whiteSpace: "nowrap",
            }}>
            {selected && <span aria-hidden="true" style={{ width: 7, height: 7, borderRadius: 999, background: "var(--accent)" }} />}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

import React from "react";

/** Labeled dropdown (Base / Branch pickers). Value renders in mono. */
export function Select({ label, value, options = [], onChange, maxWidth = 256 }) {
  return (
    <label style={{ display: "flex", alignItems: "center", gap: 6, fontFamily: "var(--font-sans)" }}>
      {label && <span style={{ color: "var(--text-muted)", fontSize: 12 }}>{label}</span>}
      <select
        value={value}
        onChange={(e) => onChange?.(e.currentTarget.value)}
        style={{
          fontFamily: "var(--font-mono)", fontSize: 13, color: "var(--text)",
          background: "var(--bg-card)", border: "1px solid var(--border-strong)",
          borderRadius: "var(--radius-md)", padding: "4px 8px", maxWidth,
        }}
      >
        {options.map((o) => <option key={o} value={o}>{o}</option>)}
      </select>
    </label>
  );
}

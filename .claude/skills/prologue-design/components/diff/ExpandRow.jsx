import React from "react";

/** Hidden-context row: "59 unchanged lines" with Show 20 / Expand all links. */
export function ExpandRow({ count, onShowTop, onShowBottom, onExpandAll }) {
  const link = { background: "none", border: "none", padding: "1px 6px", borderRadius: 4, fontSize: 12, color: "var(--accent-strong)", fontWeight: 600, cursor: "pointer", fontFamily: "var(--font-sans)" };
  return (
    <div style={{
      display: "flex", alignItems: "center", justifyContent: "center", gap: 12,
      minHeight: 26, padding: "0 12px", background: "var(--bg-subtle)",
      fontSize: 12, color: "var(--text-muted)", fontFamily: "var(--font-sans)",
    }}>
      {onShowBottom && <button type="button" style={link} onClick={onShowBottom}>{"\u2193"} Show 20</button>}
      <span>{count} unchanged lines</span>
      {onShowTop && <button type="button" style={link} onClick={onShowTop}>{"\u2191"} Show 20</button>}
      {onExpandAll && <button type="button" style={link} onClick={onExpandAll}>Expand all</button>}
    </div>
  );
}

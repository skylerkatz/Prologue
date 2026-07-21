import React from "react";

/** Open-comment count: teal circle, cream mono numeral. */
export function CountPill({ count, title }) {
  if (!count) return null;
  return (
    <span title={title} style={{
      flex: "none", minWidth: 19, height: 19, padding: "0 5px", boxSizing: "border-box",
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      borderRadius: 999, background: "var(--teal-800)", color: "var(--text-on-dark)",
      fontFamily: "var(--font-mono)", fontSize: 11, fontWeight: 700,
    }}>{count}</span>
  );
}

import React from "react";

/** Signed added/deleted counts: +n green, −n red, mono. Pass binary for BIN. */
export function DiffCounts({ additions = 0, deletions = 0, binary = false, size = 12.5 }) {
  return (
    <span style={{ display: "inline-flex", gap: 8, fontFamily: "var(--font-mono)", fontSize: size, fontWeight: 600 }}>
      {binary ? <span style={{ color: "var(--text-muted)" }}>BIN</span> : <>
        <span style={{ color: "var(--added)" }}>+{additions}</span>
        <span style={{ color: "var(--removed)" }}>{"\u2212"}{deletions}</span>
      </>}
    </span>
  );
}

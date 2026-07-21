import React from "react";
import { StatusBadge } from "../core/StatusBadge.jsx";
import { CountPill } from "../core/CountPill.jsx";
import { DiffCounts } from "../core/DiffCounts.jsx";

/**
 * Sidebar file row. Paths truncate from the LEFT (\u2026llingController.php).
 * Selected rows get the warm highlight + a 3px orange bookmark on the left edge.
 */
export function FileRow({ path, status = "modified", additions = 0, deletions = 0, binary = false, commentCount = 0, selected = false, onSelect }) {
  return (
    <button type="button" onClick={onSelect} title={path} style={{
      display: "flex", alignItems: "center", gap: 8, width: "100%",
      padding: "6px 10px", background: selected ? "var(--surface-selected)" : "none",
      border: "none", borderRadius: "var(--radius-md)",
      boxShadow: selected ? "inset 3px 0 0 var(--accent)" : "none",
      textAlign: "left", cursor: "pointer",
    }}>
      <StatusBadge status={status} />
      <span style={{
        flex: 1, minWidth: 0, fontFamily: "var(--font-mono)", fontSize: 12.5, color: "var(--text)",
        overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", direction: "rtl", textAlign: "left",
      }}><bdi>{path}</bdi></span>
      <CountPill count={commentCount} />
      <DiffCounts additions={additions} deletions={deletions} binary={binary} size={12} />
    </button>
  );
}

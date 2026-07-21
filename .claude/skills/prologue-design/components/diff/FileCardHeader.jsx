import React from "react";
import { StatusBadge } from "../core/StatusBadge.jsx";
import { DiffCounts } from "../core/DiffCounts.jsx";
import { Button } from "../core/Button.jsx";

/** Header row of a file's diff card: disclosure, badge, bold mono path, counts, + Add comment. */
export function FileCardHeader({ path, status = "modified", additions = 0, deletions = 0, expanded = true, focused = false, onToggle, onAddComment }) {
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 10, padding: "9px 14px",
      background: "var(--bg-card)", borderBottom: expanded ? "1px solid var(--border)" : "none",
      borderRadius: expanded ? "8px 8px 0 0" : 8,
      boxShadow: focused ? "inset 3px 0 0 var(--accent)" : "none",
    }}>
      <button type="button" onClick={onToggle} aria-label={expanded ? "Collapse file" : "Expand file"} style={{
        flex: "none", width: 20, height: 20, background: "none", border: "none", cursor: "pointer",
        color: "var(--text-muted)", fontSize: 10, padding: 0,
      }}>{expanded ? "\u25be" : "\u25b8"}</button>
      <StatusBadge status={status} />
      <span style={{ flex: 1, minWidth: 0, fontFamily: "var(--font-mono)", fontSize: 13, fontWeight: 700, color: "var(--text-strong)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{path}</span>
      <DiffCounts additions={additions} deletions={deletions} />
      {onAddComment && <Button variant="outline" size="sm" onClick={onAddComment}>+ Add comment</Button>}
    </div>
  );
}

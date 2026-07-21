import React from "react";

const KIND = {
  context:  { bg: "var(--bg-card)", gutter: "transparent", sign: "" },
  addition: { bg: "var(--diff-add-bg)", gutter: "var(--diff-add-gutter)", sign: "+", signColor: "var(--added)" },
  deletion: { bg: "var(--diff-del-bg)", gutter: "var(--diff-del-gutter)", sign: "\u2212", signColor: "var(--removed)" },
};

/**
 * One diff line: [old# | new# | sign | code]. Selected lines flip the
 * gutter to solid orange with white numerals. children may carry
 * syntax-highlighted spans; intraline marks use the gutter tint at 2px radius.
 */
export function DiffLine({ kind = "context", oldNo, newNo, selected = false, commentable = true, children }) {
  const k = KIND[kind] ?? KIND.context;
  const gutterStyle = (n) => ({
    textAlign: "right", paddingRight: 8, lineHeight: "21px",
    background: selected ? "var(--accent)" : k.gutter,
    color: selected ? "#fff" : "var(--text-muted)",
    cursor: commentable ? "pointer" : "default", userSelect: "none",
  });
  return (
    <div style={{
      display: "grid", gridTemplateColumns: "var(--gutter-col) var(--gutter-col) 18px 1fr",
      fontFamily: "var(--font-mono)", fontSize: 12, lineHeight: "21px", minHeight: 21,
      background: selected ? "var(--line-selected-bg)" : k.bg,
    }}>
      <span style={gutterStyle(oldNo)}>{oldNo ?? ""}</span>
      <span style={gutterStyle(newNo)}>{newNo ?? ""}</span>
      <span style={{ textAlign: "center", color: k.signColor ?? "var(--text-muted)" }}>{k.sign}</span>
      <span style={{ whiteSpace: "pre-wrap", wordBreak: "break-all", tabSize: 4, paddingRight: 12, userSelect: "text", cursor: "text", color: "var(--text)" }}>{children}</span>
    </div>
  );
}

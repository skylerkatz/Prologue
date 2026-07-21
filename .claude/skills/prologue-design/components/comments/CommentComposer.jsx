import React, { useState } from "react";
import { Button } from "../core/Button.jsx";

/** Comment input: textarea + Comment/Cancel + \u2318\u21a9 hint. */
export function CommentComposer({ placeholder = "Comment on the selected lines\u2026", submitLabel = "Comment", initial = "", onSubmit, onCancel }) {
  const [text, setText] = useState(initial);
  const [focus, setFocus] = useState(false);
  return (
    <div style={{ padding: "8px 10px 10px", fontFamily: "var(--font-sans)" }}>
      <textarea
        rows={3}
        placeholder={placeholder}
        value={text}
        autoFocus
        onFocus={() => setFocus(true)}
        onBlur={() => setFocus(false)}
        onChange={(e) => setText(e.currentTarget.value)}
        style={{
          width: "100%", boxSizing: "border-box", resize: "vertical",
          font: "inherit", fontSize: 13, color: "var(--text)",
          background: "var(--bg-card)",
          border: "1px solid " + (focus ? "var(--accent)" : "var(--border-strong)"),
          outline: "none", borderRadius: "var(--radius-md)", padding: "7px 9px",
        }}
      />
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 7 }}>
        <Button variant="primary" size="sm" disabled={text.trim() === ""} onClick={() => onSubmit?.(text)}>{submitLabel}</Button>
        <Button variant="secondary" size="sm" onClick={onCancel}>Cancel</Button>
        <span style={{ marginLeft: "auto", fontSize: 11.5, color: "var(--text-muted)" }}>{"\u2318\u21a9"} to submit</span>
      </div>
    </div>
  );
}

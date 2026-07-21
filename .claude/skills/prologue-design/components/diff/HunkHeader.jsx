import React from "react";

/** @@ range header \u2014 the diff's one cool-blue tint. */
export function HunkHeader({ text }) {
  return (
    <div style={{
      fontFamily: "var(--font-mono)", fontSize: 12, lineHeight: "26px", minHeight: 26,
      padding: "0 12px", background: "var(--hunk-bg)", color: "var(--hunk-text)",
      whiteSpace: "pre-wrap", wordBreak: "break-all",
    }}>{text}</div>
  );
}

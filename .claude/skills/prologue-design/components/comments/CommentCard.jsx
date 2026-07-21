import React from "react";
import { Button } from "../core/Button.jsx";
import { StatePill } from "../core/StatePill.jsx";

/** Teal avatar circle carrying the mono comment ID. */
export function CommentAvatar({ id, size = 26 }) {
  return (
    <span style={{
      flex: "none", width: size, height: size, borderRadius: 999,
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      background: "var(--teal-800)", color: "var(--text-on-dark)",
      fontFamily: "var(--font-mono)", fontSize: size * 0.42, fontWeight: 700,
    }}>C{id}</span>
  );
}

/**
 * One comment. Roots get the 3px orange left rail; replies indent 24px on
 * a subtle background with no rail. Author is always "You".
 */
export function CommentCard({ id, body, location, time, state = "open", codeChanged = false, isReply = false, readOnly = false, actions, children }) {
  const closed = state !== "open";
  return (
    <article style={{
      background: isReply ? "var(--bg-subtle)" : "var(--bg-card)",
      border: "1px solid var(--border)",
      borderLeft: isReply ? "1px solid var(--border)" : "3px solid var(--comment-accent-border)",
      borderRadius: "var(--radius-lg)",
      marginLeft: isReply ? "var(--diff-indent)" : 0,
      opacity: closed ? 0.7 : 1,
      fontFamily: "var(--font-sans)", maxWidth: 864,
    }}>
      <header style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 12px 0" }}>
        <CommentAvatar id={id} size={isReply ? 22 : 26} />
        <span style={{ fontWeight: 700, fontSize: 13, color: "var(--text-strong)" }}>You</span>
        {location && <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{location}</span>}
        <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{time}</span>
        {closed && <StatePill kind={state} />}
        {codeChanged && <StatePill kind="changed" title="The commented code changed since this comment was made \u2014 re-review it." />}
        <span style={{ flex: 1 }} />
        {!readOnly && (actions ?? (
          <span style={{ display: "flex", gap: 2 }}>
            <Button variant="ghost" size="sm">Edit</Button>
            <Button variant="ghost" size="sm">Delete</Button>
          </span>
        ))}
      </header>
      <p style={{ margin: 0, padding: "6px 12px 12px", fontSize: 13.5, lineHeight: 1.5, color: "var(--text)", whiteSpace: "pre-wrap", userSelect: "text", cursor: "text" }}>{body}</p>
      {children}
    </article>
  );
}

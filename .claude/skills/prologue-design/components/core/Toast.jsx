import React from "react";

/** Bottom-center confirmation toast (clipboard copies, errors). */
export function Toast({ children, error = false, fixed = true }) {
  return (
    <div role="status" style={{
      position: fixed ? "fixed" : "static", bottom: fixed ? 20 : undefined, left: fixed ? "50%" : undefined,
      transform: fixed ? "translateX(-50%)" : undefined, zIndex: 40,
      padding: "8px 16px", border: `1px solid ${error ? "var(--danger)" : "var(--border)"}`,
      borderRadius: "var(--radius-md)", background: "var(--bg-card)",
      color: error ? "var(--danger)" : "var(--text)",
      fontFamily: "var(--font-sans)", fontSize: 13, boxShadow: "var(--shadow-menu)", whiteSpace: "nowrap",
    }}>{children}</div>
  );
}

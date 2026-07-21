import React from "react";

/** macOS title bar: traffic lights, ribbon mark, Lora wordmark, branch. */
export function TitleBar({ branch, height = 44 }) {
  return (
    <header style={{
      height, display: "flex", alignItems: "center", justifyContent: "center", position: "relative",
      background: "var(--bg-titlebar)", borderRadius: "10px 10px 0 0", flex: "none",
    }}>
      <div style={{ position: "absolute", left: 16, display: "flex", gap: 8 }}>
        {["#ff5f57", "#febc2e", "#28c840"].map((c) => (
          <span key={c} style={{ width: 12, height: 12, borderRadius: 999, background: c }} />
        ))}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 9 }}>
        <svg width="13" height="19" viewBox="182 0 148 336" aria-hidden="true">
          <path d="M182 0 H330 V336 L256 284 L182 336 Z" fill="#F6A33C" />
        </svg>
        <span style={{ fontFamily: "var(--font-serif)", fontSize: 17, fontWeight: 600, color: "var(--text-on-dark)" }}>Prologue</span>
        {branch && <span style={{ fontFamily: "var(--font-sans)", fontSize: 13.5, color: "var(--text-on-dark)", opacity: 0.62 }}>{"\u2014"} {branch}</span>}
      </div>
    </header>
  );
}

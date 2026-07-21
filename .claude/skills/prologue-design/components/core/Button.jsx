import React, { useState } from "react";

const VARIANTS = {
  primary:  { bg: "var(--accent)", hoverBg: "var(--accent-strong)", color: "var(--text-on-accent)", border: "1px solid transparent", weight: 600 },
  secondary:{ bg: "var(--bg-card)", hoverBorder: "var(--accent)", color: "var(--text)", border: "1px solid var(--border)", weight: 500 },
  outline:  { bg: "var(--bg-card)", hoverBorder: "var(--accent)", color: "var(--accent-strong)", border: "1px solid var(--border)", weight: 600 },
  ghost:    { bg: "transparent", hoverBg: "var(--orange-50)", hoverColor: "var(--accent-strong)", color: "var(--text-muted)", border: "1px solid transparent", weight: 500 },
  "ghost-danger": { bg: "transparent", hoverBg: "var(--orange-50)", color: "var(--danger)", border: "1px solid transparent", weight: 500 },
};

const SIZES = {
  sm: { padding: "2px 9px",  fontSize: "12px", borderRadius: "var(--radius-md)" },
  md: { padding: "5px 13px", fontSize: "13px", borderRadius: "var(--radius-md)" },
};

/** Prologue button. variant: primary | secondary | outline | ghost | ghost-danger. */
export function Button({ variant = "secondary", size = "md", disabled = false, children, onClick, title, style }) {
  const [hover, setHover] = useState(false);
  const v = VARIANTS[variant] ?? VARIANTS.secondary;
  const s = SIZES[size] ?? SIZES.md;
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        fontFamily: "var(--font-sans)", fontWeight: v.weight, cursor: disabled ? "default" : "pointer",
        background: hover && !disabled && v.hoverBg ? v.hoverBg : v.bg,
        color: hover && !disabled && v.hoverColor ? v.hoverColor : v.color,
        border: v.border,
        borderColor: hover && !disabled && v.hoverBorder ? v.hoverBorder : undefined,
        opacity: disabled ? 0.55 : 1,
        whiteSpace: "nowrap", lineHeight: 1.45,
        ...s, ...style,
      }}
    >
      {children}
    </button>
  );
}

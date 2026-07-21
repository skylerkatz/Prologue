/** @startingPoint section="Core" subtitle="Prologue button variants" viewport="700x220" */
export interface ButtonProps {
  /** primary (orange fill) | secondary (bordered) | outline (bordered, orange text) | ghost (quiet text action) | ghost-danger */
  variant?: "primary" | "secondary" | "outline" | "ghost" | "ghost-danger";
  size?: "sm" | "md";
  disabled?: boolean;
  title?: string;
  onClick?: () => void;
  style?: React.CSSProperties;
  children: React.ReactNode;
}

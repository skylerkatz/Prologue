export interface DiffLineProps {
  kind?: "context" | "addition" | "deletion";
  oldNo?: number;
  newNo?: number;
  /** Part of a gutter-drag selection: orange gutters + warm row tint. */
  selected?: boolean;
  commentable?: boolean;
  children?: React.ReactNode;
}

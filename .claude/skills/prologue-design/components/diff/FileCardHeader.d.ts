export interface FileCardHeaderProps {
  path: string;
  status?: "added" | "modified" | "deleted" | "renamed";
  additions?: number;
  deletions?: number;
  expanded?: boolean;
  /** j/k keyboard cursor: inset 3px orange bar. */
  focused?: boolean;
  onToggle?: () => void;
  onAddComment?: () => void;
}

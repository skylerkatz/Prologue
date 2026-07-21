export interface FileRowProps {
  path: string;
  status?: "added" | "modified" | "deleted" | "renamed";
  additions?: number;
  deletions?: number;
  binary?: boolean;
  commentCount?: number;
  selected?: boolean;
  onSelect?: () => void;
}

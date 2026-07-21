export interface CommentCardProps {
  /** Numeric id; renders as mono "C{id}" in the teal avatar. */
  id: number;
  body: string;
  /** e.g. "Lines 95\u201398" — omit for file/review-level comments. */
  location?: string;
  time?: string;
  state?: "open" | "resolved" | "dismissed";
  codeChanged?: boolean;
  isReply?: boolean;
  readOnly?: boolean;
  /** Custom header actions; default is Edit/Delete ghosts. */
  actions?: React.ReactNode;
  children?: React.ReactNode;
}

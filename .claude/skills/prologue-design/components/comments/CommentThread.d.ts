import type { CommentCardProps } from "./CommentCard";
export interface CommentThreadProps {
  root: CommentCardProps;
  replies?: CommentCardProps[];
  onReply?: () => void;
  readOnly?: boolean;
}

export interface CommentComposerProps {
  placeholder?: string;
  submitLabel?: string;
  initial?: string;
  onSubmit?: (body: string) => void;
  onCancel?: () => void;
}

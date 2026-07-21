export interface ModeToggleProps {
  value: string;
  options?: Array<{ value: string; label: string }>;
  onChange?: (value: string) => void;
}

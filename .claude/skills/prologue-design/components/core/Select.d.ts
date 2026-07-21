export interface SelectProps {
  label?: string;
  value: string;
  options: string[];
  onChange?: (value: string) => void;
  maxWidth?: number;
}

import { WORKING_TREE_MODES, type DiffMode } from "../types";

interface ModeToggleProps {
  mode: DiffMode;
  onChange: (mode: DiffMode) => void;
}

export function ModeToggle({ mode, onChange }: ModeToggleProps) {
  return (
    <div className="mode-toggle" role="radiogroup" aria-label="Working tree mode">
      {WORKING_TREE_MODES.map(({ value, label }) => (
        <button
          key={value}
          type="button"
          role="radio"
          aria-checked={mode === value}
          className={mode === value ? "mode-option selected" : "mode-option"}
          onClick={() => onChange(value)}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

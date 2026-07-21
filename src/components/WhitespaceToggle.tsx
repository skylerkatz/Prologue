interface WhitespaceToggleProps {
  hidden: boolean;
  onChange: (hidden: boolean) => void;
}

/** Recompute the diff ignoring whitespace changes (git `-w` semantics). */
export function WhitespaceToggle({ hidden, onChange }: WhitespaceToggleProps) {
  return (
    <div className="mode-toggle">
      <button
        type="button"
        role="switch"
        aria-checked={hidden}
        className={hidden ? "mode-option selected" : "mode-option"}
        title="Recompute the diff ignoring whitespace changes"
        onClick={() => onChange(!hidden)}
      >
        Hide whitespace
      </button>
    </div>
  );
}

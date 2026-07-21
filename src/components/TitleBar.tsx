interface TitleBarProps {
  /** Branch under review; null on the welcome page. */
  branch: string | null;
}

/**
 * Custom teal title bar over the native overlay title bar: the ribbon mark
 * (extracted from the app icon), the Lora wordmark, and the branch. The
 * native traffic lights render on top of it; the whole bar is a drag region.
 */
export function TitleBar({ branch }: TitleBarProps) {
  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="titlebar-brand">
        <svg width="13" height="19" viewBox="182 0 148 336" aria-hidden="true">
          <path d="M182 0 H330 V336 L256 284 L182 336 Z" fill="#F6A33C" />
        </svg>
        <span className="titlebar-wordmark">Prologue</span>
        {branch !== null && (
          <span className="titlebar-branch">— {branch}</span>
        )}
      </div>
    </header>
  );
}

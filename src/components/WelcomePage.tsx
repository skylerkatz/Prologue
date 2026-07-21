interface WelcomePageProps {
  recents: string[];
  error: string | null;
  onPickRepo: () => void;
  onOpenRecent: (path: string) => void;
  onRemoveRecent: (path: string) => void;
}

export function WelcomePage({
  recents,
  error,
  onPickRepo,
  onOpenRecent,
  onRemoveRecent,
}: WelcomePageProps) {
  return (
    <main className="welcome">
      <h1>Prologue</h1>
      <p className="tagline">Review local branches like a GitHub PR.</p>
      <button type="button" className="primary" onClick={onPickRepo}>
        Open Repository…
      </button>
      {error && <p className="error">{error}</p>}
      {recents.length > 0 && (
        <section className="recents">
          <h2>Recent repositories</h2>
          <ul>
            {recents.map((path) => (
              <li key={path}>
                <button
                  type="button"
                  className="recent-path"
                  title={path}
                  onClick={() => onOpenRecent(path)}
                >
                  <span className="recent-name">
                    {path.split("/").filter(Boolean).pop() ?? path}
                  </span>
                  <span className="recent-dir">{path}</span>
                </button>
                <button
                  type="button"
                  className="recent-remove"
                  aria-label={`Remove ${path} from recents`}
                  onClick={() => onRemoveRecent(path)}
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </main>
  );
}

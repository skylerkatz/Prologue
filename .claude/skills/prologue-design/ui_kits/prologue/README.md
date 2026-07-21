# Prologue UI kit

Interactive recreation of the Prologue app per the target mockup. One page, three surfaces:

- **Review screen** (default): toolbar (repo pill, Base ← Branch selects, mode toggle, Export ▾, Archived, ↻ Refresh), file sidebar, "Review comments" panel, per-file diff cards with hunks, inline comment thread with one-level reply, and line composer.
- **Welcome page**: click the red traffic light to toggle. Repo picker + recent repositories.
- **Archive overlay**: click "Archived". Read-only list → detail.

Try it: click a line number to open the composer on that line; add a review comment; Reply/Resolve/Delete a thread; Export ▾ → toast; "Hide whitespace" collapses whitespace-only lines in every file (see the blade file's indent change) and persists across sessions via `localStorage["prologue.hideWhitespace"]` — in production, persist it app-wide (settings store), not per review.

**Whitespace toggle placement rationale**: it lives in the toolbar next to the working-tree mode toggle because both are diff-wide *view* settings; active state uses the same orange-dot treatment as the selected mode segment. Hidden lines leave an italic "N whitespace-only lines hidden" band so the diff never silently omits changes.

Implementation: `app.js` (JSX via in-browser Babel — kept `.js` because the project server compiles `.jsx` files to ES modules). Cosmetic twins of `components/`; all values come from `styles.css` tokens.

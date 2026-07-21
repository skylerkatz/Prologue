# Prologue Design System

**Prologue** is a macOS desktop app (Tauri + Rust backend + React frontend) for reviewing and commenting on **local git diffs** — a private review pass with your agent *before* opening a real PR on GitHub. The name and mark: an orange bookmark ribbon on a deep-teal field — the review you write before the story (the PR) begins.

This design system is the handoff spec for restyling the existing app (working name "Diff Viewer") into the Prologue brand. It contains tokens, guidelines, reusable React components, and full-screen UI kit recreations.

## Sources
- Local codebase: `diff-viewer/` (Tauri; React UI in `src/`, all styling in `src/App.css`, components in `src/components/`). Class names referenced throughout map 1:1 to that file.
- Target mockup: `uploads/prologue-chrome-screenshot.png` (the spec — where it differs from the current app, the mockup wins).
- Current-state screenshot: `uploads/CleanShot 2026-07-21 at 10.13.49@2x.png`.
- App icon: `assets/prologue-logo.svg`.

## Product surfaces
One product, four surfaces (all in the UI kit at `ui_kits/prologue/`):
1. **Review screen** — toolbar, file sidebar, review-comments panel, virtualized diff with inline comment threads.
2. **Welcome page** — repo picker + recent repositories.
3. **Archived reviews** — read-only overlay dialog.
4. **Export menu + toast** — clipboard export of open comments (Markdown/JSON/agent-prompt variants).

## Content fundamentals
- **Voice**: terse, technical, confident. Sentence case everywhere except uppercase micro-headers (`3 FILES CHANGED`, `RECENT REPOSITORIES`).
- **Person**: the reviewer is "You" (comments are authored locally; there are no other users). Tooltips address the user directly: "Copy the review's open comments to the clipboard".
- **Comment IDs**: every comment gets a mono ID (`C1`, `C2`…) used as its avatar glyph and export handle.
- **Labels are verbs or nouns, never both**: `Reply`, `Resolve`, `Dismiss`, `Edit`, `Delete`, `Reopen`, `Refresh`, `Export`, `+ Add comment`.
- **Destructive actions confirm inline**, never via dialog: "Really delete? Its 3 replies will be deleted too" / "Keep".
- **Keyboard hints** use glyphs: `⌘↩ to submit`. Ellipsis (`…`) for in-progress: "Saving…", "Computing diff…".
- **No emoji.** The one symbolic character is `⚠` on the "code changed since commented" flag.
- Counts always signed and colored: `+7 −3` (green/red, mono, U+2212 minus).

## Visual foundations
- **Palette**: warm cream chrome (`--bg-app #faf7f0`) with white content cards; deep teal (`#0e2a2e–#173e42`) reserved for the title bar, repo pill, and avatars; amber-orange (`#f6a33c–#e07b1a`) is the single accent — primary buttons, selection, comment rails, the ribbon. Diff tints stay conventional soft green/red; the hunk header is the only cool tint (`#e6f0f4`).
- **Type**: Lora (serif) for the wordmark + section headings only; IBM Plex Sans for UI copy; IBM Plex Mono for anything git — paths, branches, diff code, comment IDs, timestamps in headers. Mono is the dominant voice.
- **Backgrounds**: flat fills, no imagery, no gradients in the UI (gradients live only inside the app icon).
- **Borders**: 1px `--border #e3ddd0` on every card and control; borders do the separating, shadows are whispers (`--shadow-card`) except floating menus/dialogs (`--shadow-menu`).
- **Radii**: 4px badges → 6px buttons/menus → 8px cards → 10px panels. Pills (`999px`) for count badges and state chips.
- **Selection**: orange, not blue — selected file row gets `--surface-selected` plus a 3px orange ribbon bookmark on its left edge; selected diff lines get `--line-selected-bg`.
- **Comment cards**: white, 1px border, 8px radius, **3px orange left rail**; header row = teal avatar circle with mono ID, bold "You", meta, spacer, quiet text actions. Replies indent once (24px) on `--bg-subtle` and never nest further.
- **Hover states**: background shifts to `--bg-subtle` (rows, menu items) or border darkens to `--accent` (outlined buttons). Primary buttons darken to `--accent-strong`. No scale/transform effects.
- **Focus**: `--accent` border on inputs; no glow rings.
- **Motion**: essentially none — instant state changes; toasts fade in/out only. This is a dense pro tool.
- **Density**: 13px UI base, 12px/21px diff lines, controls ~28–32px tall. Content max-width for comment cards ~864px.
- **Transparency/blur**: only the archive overlay scrim (`--overlay-scrim`). No frosted glass.

## Iconography
- **No icon font, no icon set.** The app uses typed glyphs as icons, all sourced from the codebase: `←` (base←branch direction), `▾`/`▸` (disclosure, menus), `↻` (refresh), `×` (remove), `↳` (reply), `⚠` (code-changed flag), `+` (add actions). Keep it that way — do not introduce an icon library.
- Status letters `A M D R` in tinted rounded squares are the file-status system (see `StatusBadge`).
- The only image asset is the app icon `assets/prologue-logo.svg` (ribbon mark). In the title bar the ribbon renders beside the Lora wordmark; extract the ribbon path from the icon rather than redrawing it.

## Font substitution note
The mockup's fonts were matched to Google Fonts: **Lora** (wordmark/headings), **IBM Plex Sans** (UI), **IBM Plex Mono** (code). Loaded via CDN in `tokens/fonts.css`. If you have licensed originals, replace the `@import` with `@font-face` rules.

## Index
- `styles.css` — global entry; imports everything under `tokens/`.
- `tokens/` — `colors.css`, `typography.css`, `spacing.css`, `fonts.css`.
- `assets/prologue-logo.svg` — app icon.
- `guidelines/` — foundation specimen cards (colors, type, spacing, diff tints).
- `components/core/` — Button, Select, ModeToggle, StatusBadge, CountPill, StatePill, Toast, TitleBar.
- `components/comments/` — CommentCard, CommentComposer, CommentThread.
- `components/diff/` — DiffLine, HunkHeader, ExpandRow, FileCardHeader, FileRow.
- `ui_kits/prologue/` — full review screen + welcome + archive + export (interactive).
- `SKILL.md` — agent-facing entry point.

## Intentional additions
None — the component inventory mirrors `src/components/` + `App.css` of the codebase.

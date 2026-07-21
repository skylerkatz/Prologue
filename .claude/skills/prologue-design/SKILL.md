---
name: prologue-design
description: Use this skill to generate well-branded interfaces and assets for Prologue (macOS git-diff review app — Tauri + Rust + React), either for production or throwaway prototypes/mocks/etc. Contains essential design guidelines, colors, type, fonts, assets, and UI kit components for prototyping.
user-invocable: true
---

Read the README.md file within this skill, and explore the other available files.

Key facts: warm cream chrome + deep teal brand + single amber-orange accent; Lora for wordmark/section headings, IBM Plex Sans for UI, IBM Plex Mono for anything git; glyph-only iconography (no icon set); light mode only. Tokens in `tokens/*.css` (entry `styles.css`); component specs in `components/*/`; full-screen reference in `ui_kits/prologue/`.

For production work on the diff-viewer codebase: styling lives in `src/App.css` — map the CSS custom properties there onto the tokens in `tokens/colors.css` (the names were chosen to align: `--bg`, `--border`, `--accent`, `--diff-add-bg`, …), swap the font stacks per `tokens/typography.css`, and follow readme.md's VISUAL FOUNDATIONS for the mockup-specific changes (orange comment rails, teal C-id avatars, "You" author line, one-level nested replies, orange selection instead of blue, tinted A/M/D/R badges, segmented mode toggle with orange dot, teal title bar with ribbon + Lora wordmark).

If creating visual artifacts (slides, mocks, throwaway prototypes, etc), copy assets out and create static HTML files for the user to view. If working on production code, you can copy assets and read the rules here to become an expert in designing with this brand.
If the user invokes this skill without any other guidance, ask them what they want to build or design, ask some questions, and act as an expert designer who outputs HTML artifacts _or_ production code, depending on the need.

# Changelog

All notable changes to Sh_Images are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/); versions follow the
repo's release tags.

## Unreleased

### V3 — Gallery organization

- **Added** recent folders: the last 5 opened folders persist in
  `settings.json` (schema 2 → 3, seeded from `last_dir` on migration) and
  render as clickable chips on the Welcome screen; the Continue button
  keeps opening the most recent one. Chips are labeled `parent\name`.
- **Added** slideshow: auto-advance the viewer every 3 seconds (looping),
  toggled by `Space` or a play/pause chip in the bottom overlay; stops on
  folder switch and is mutually exclusive with crop mode.
- **Added** grid multi-selection: Ctrl+click / Shift+click / Shift+arrows /
  Ctrl+Space toggle and extend, Ctrl+A fills, Escape clears; `Ctrl+C`
  copies absolute paths of the selection. Plain click still opens the
  viewer; selection clears on folder swap and sort change.
- **Added** gallery sort by name / created / modified / size / type with
  ascending/descending direction, controlled from a topbar dropdown next
  to the theme picker (`Name ↑` chip; opens a criterion + direction menu).
- **Added** global sort persistence in `settings.json` (`sort_by`,
  `sort_dir`); schema version bumped 1 → 2 with per-field serde defaults,
  so an existing V1 file migrates without losing keys (`last_dir`, theme).
- **Changed** the scan path: `scan_dir` now delegates to
  `scan_entries` (path + size + modified + created in one enumeration
  pass — on Windows the metadata rides the directory listing).
- **Changed** the selection now re-anchors by path when the sort
  changes — the grid keeps the same image selected instead of jumping
  to whatever lands on the old index.

### V2 — Gallery Pro UI (merged)

- Grid hover plates, theme-adaptive hover fill (`hover_fill`), embedded
  SVG icon set, dissolving Viewer topbar, welcome screen refresh,
  strong welcome-button hover (`hover_fill_strong`).

### V1 — Core

- Folder scan with natural sort, GPUI viewer with fit/zoom/pan, LRU
  decode cache, crop tool with copy/save, theme system with hot reload,
  atomic settings persistence.

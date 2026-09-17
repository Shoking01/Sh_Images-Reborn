# Changelog

All notable changes to Sh_Images are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/); versions follow the
repo's release tags.

## Unreleased

### i18n S2 — Settings surfaces (picker + section labels + action labels)

- **Added** language picker in Settings-General (English/Español
  autonyms): commits via atomic save and live-switches every settings
  surface through `cx.notify()`, no restart. Failed saves keep the
  previous language.
- **Changed** `ActionDescriptor.label` to `label_key: StrKey`: all 18
  `ACTIONS` labels resolve through the table (`action_label(id, lang)`
  with id fallback); section names, Theme/hidden-files/recents rows,
  capture prompt, conflict text, and reset confirm all render from
  `Settings.language`.
- **Note**: no new Spanish strings beyond the S1-reviewed table —
  picker autonyms are proper-noun-exempt (identical in both languages).

### i18n S1 — Foundation (string table + language setting)

- **Added** compile-time i18n table in `sh-core::i18n`: `Language`
  (`En` default / `Es` neutral Spanish, serialized lowercase), one `StrKey`
  per inventoried user-facing string (52 keys), exhaustive `get`/`t`
  lookup with English fallback, `PluralForm` + `plural(n)`, `BatchVerb`,
  and named-argument templates (`batch_report`, `batch_bar_delete`,
  `batch_bar_move`, `recents_header`, `conflict_text`, `no_images_in`,
  `selected_suffix`). No UI surface is swapped yet — that lands in S2–S5.
- **Added** `Settings.language` with schema 4 → 5 migration: v4 files
  load as English with `last_dir`/theme/keymap intact; v5 Spanish
  round-trips; corrupt files fall back to defaults untouched until save.
- **Note**: Spanish wording is best-effort neutral (usted-neutral,
  region-free) and requires native-speaker review before merge.

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
- **Added** batch move/delete over the grid selection: `Delete` stages a
  recycle-bin delete, `M` picks a destination folder; both confirm through
  a crop-style bar (Enter confirms, Esc cancels). Moves skip collisions
  with a report, never overwrite; leftovers stay selected.
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

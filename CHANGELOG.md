# Changelog

All notable changes to Sh_Images are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/); versions follow the
repo's release tags.

## Unreleased

### Viewer — one-tap zoom preset chips

- **Added** four click-only zoom preset chips (Fit / 50% / 100% / 200%)
  in the viewer bottom overlay: one tap lands the viewer on the exact
  scale (100% = actual pixels), centered on the viewport midpoint.
  Sub-floor requests (e.g. 50% on a small image) snap back to fit per the
  existing clamp contract.
- **Added** `Session::set_zoom_preset`: every preset funnels through the
  same `zoom_at` → `clamp_zoom` path as the wheel (floor snap-back band
  pinned as `FIT_SNAP_REL_EPS`); the Fit chip reuses the exact
  `toggle_fit_100` fit semantics.
- **Added** shared overlay `action_button` helper (optional pill chrome):
  prev/next/slideshow migrate bare (pixel-identical), preset chips opt
  into the topbar density-control look.
- **Added** `ZoomPresetFit` / `ZoomPreset50` / `ZoomPreset100` /
  `ZoomPreset200` i18n keys (EN + neutral ES: Fit/Ajustar; numerals
  locale-neutral by design) with anti-drift coverage (59 keys).
- **Note**: click-only by design — no shortcuts, no settings knob, no
  persistence; preset selection is transient session state.

### Zoomable grid — S/M/L thumbnail density presets

- **Added** gallery grid density presets S / M / L with an explicit
  per-preset integer geometry table (S: 120/125/100×75; M: today's
  180/170/160×120 verbatim; L: 240/215/220×165). M is the default and
  reproduces the previous layout pixel-for-pixel.
- **Added** segmented S/M/L control in the grid topbar beside the sort
  chip: switching is instant and layout-only (zero re-decodes — the
  single `THUMB_MAX_DIM = 256` cap is unchanged for all presets),
  re-clamps a stale scroll offset into the new preset's range, and keeps
  the cursor visible without re-sorting.
- **Added** `grid_size` persistence in `settings.json` (schema 5 → 6 via
  per-field serde default → M): v5 files migrate with all prefs
  (including `language`) intact; v6 round-trips; corrupt files fall back
  to defaults untouched until save.
- **Added** `GridSizeSmall` / `GridSizeMedium` / `GridSizeLarge` i18n keys
  (EN + neutral ES: Small/Pequeño, Medium/Mediano, Large/Grande) with
  anti-drift coverage (55 keys).
- **Note**: no new Spanish strings beyond the three chip words above.

### i18n S5 — Batch templates + user-facing error copy

- **Changed** `format_report` to `(lang: Language, verb: BatchVerb, …)` in
  `sh-core/src/batch.rs`, delegating its body to the `i18n` templates —
  Spanish word order is free to differ; `None` on full success preserved.
  The `confirm_pending` call site passes the typed verb + stored language.
- **Changed** `batch_bar_message` to the `batch_bar_delete` /
  `batch_bar_move` templates (one/other plural handled there) and the
  `"No images in {dir}"` error copy to the `no_images_in` template
  (both call sites: `open_folder` + `confirm_pending`).
- **Confirmed** `ShImagesError` Display prefixes remain English
  (log-only, per the error-copy decision) and untouched.

### i18n S4 — Viewer + Topbar + Overlay surfaces

- **Changed** the Viewer/Topbar render arms to resolve through the table
  from `Settings.language`: topbar `Back` / `Open folder` buttons, crop
  confirm bar `Copy` / `Save…` / `Cancel`, and batch confirm bar
  `Delete` / `Move` / `Cancel`. Labels are pre-built at the `app.rs`
  call site (`ui/topbar.rs` carries no user-facing literals) — the S4
  task mapping resolved to that layout at re-verification time.
- **Changed** the viewer empty-state hint (`Drop an image to open it`,
  the `ViewerEmptyHint` key the S3 slice left behind) to resolve through
  the language handed to `render_viewer` via a new `ViewerParams.lang`
  field.
- **Note**: no new Spanish strings beyond the S1-reviewed table. The
  topbar center slot (`name — 3/12`) and overlay zoom `%` text are
  dynamic/numeric and stay untranslated; the overlay carries no static
  hints.

### i18n S3 — Welcome + Grid surfaces

- **Changed** Welcome + Grid render paths to resolve through the table
  from `Settings.language`: hero tagline, drop-zone hint, `Continue` /
  `Open folder…` buttons, grid empty-state default, sort criterion +
  direction labels + `Sort by` header, sort chip
  (`sort_chip_label(lang, …)`; `↑`/`↓` glyphs stay locale-neutral), and
  the topbar selection-count suffix (now delegates to the
  `selected_suffix` template). Sort labels are `sh-app` literals keyed
  into `sh-core::i18n` — `navigation` carries no display strings.
- **Note**: no new Spanish strings beyond the S1-reviewed table.

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

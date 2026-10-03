# Changelog

All notable changes to Sh_Images are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/); versions follow the
repo's release tags.

## Unreleased

### Fixed

- **Fixed** the Grid painting its placeholder for every cell when the app was
  opened with a folder argument. `main.rs` scans a CLI folder synchronously and
  hands `App::new` an already-populated session, so none of the async commit
  handlers that arm the thumbnail batch ever ran on that path. The grid came up
  full of empty gray boxes and stayed that way until the user navigated
  somewhere that happened to re-arm the batch. `App::new` now arms it for the
  session it is born with, which is why it takes a `Context` rather than an
  `App`; every list swap still re-arms it, because the batch reads
  `session.images` at arm time. Guarded by
  `startup_with_a_prefilled_session_arms_the_thumbnail_batch`.

### Top bar built on gpui-component

- **Added** `gpui-component` (Longbridge, Apache-2.0) as the component layer over
  GPUI, and moved all six top-bar controls to its `Button`: Back, Open folder,
  sort chip, the S/M/L density segments, the settings gear and Crop. They now
  carry the kit's own idle/hover/pressed states instead of the hand-tuned
  `btn_bg` / `btn_hover` / `btn_pressed` trio, and gain the accessibility
  metadata the hand-built `div`s had none of.
- **Note** the element ids and the harness contracts are unchanged
  (`topbar-back`, `topbar-sort`, `("grid-size", n)`, `topbar-crop`), so the
  layout tests read them exactly as before — no assertion was edited.
- **Added** `kit_theme.rs`, which projects the app's own JSON theme onto the four
  kit slots the top bar reads. gpui-component components read a global `Theme`
  rather than the app's colors, so without this there would be two sources of
  truth: editing a theme file would restyle every `div()` while the components
  kept the previous palette. The bridge runs on startup and on every theme
  apply, including hot reload, so the JSON file stays the single source.
- **Note** hover and pressed colors are derived from the theme's own surface and
  text at 0.10 / 0.20, in gamma-encoded sRGB — the same arithmetic and the same
  ratios as the tint they replace, so the visual weight is unchanged.
- **Note** the mode handed to the kit is taken from the theme FILE NAME, falling
  back to the background's luminance only for a neutrally named file. The theme
  JSON declares no mode, and inferring it purely from luminance would let a
  mid-gray custom theme flip the whole component set on a rounding decision.
- **Fixed** `Application::with_assets` REPLACES the previously registered
  source instead of composing with it, so registering the kit's bundle as a
  second source silently disabled `AppAssets`. Every icon the app owns then
  vanished: Crop reserved its 32px box and drew no glyph, and the gear, back
  arrow, chevrons, folder, eye, close, play and pause would have gone the same
  way. `AppAssets` now owns the app's SVGs and falls back to the kit's bundle,
  so both sets resolve from the single source `main` registers. Found by
  comparing the pixel row of the bar in Grid against Viewer: the gear had
  moved 40px — 32px of Crop plus its 8px gap — while the region Crop occupied
  contained exactly zero non-background pixels.
- **Note** the settings gear carries an explicit `with_size` and an
  `accessibility_label`. Neither is what made it visible: an icon-only
  `Button` on the default variant sizes itself to `size_8` (32px) and
  `.compact()` is not consulted on that path, so an earlier reading of "it
  collapses to zero width" was the same blank-glyph symptom blamed on the
  wrong cause. The size is a deliberate 28px, and the label is required
  because an icon-only control with no text announces as unnamed.
- **Fixed** the density segments' active state. `Selectable::toggled` only
  announces the pressed state to assistive tech; `Selectable::selected` is what
  paints. With `toggled` alone the active preset was correct for screen readers
  and invisible to everyone else — measured, resting and active segments both
  painted `25252B`. Both are now set, matching the sort chip and the crop
  toggle.
- **Fixed** the theme bridge leaving every unmapped kit color transparent.
  `ThemeColor::default()` is not a palette — all 134 fields are transparent
  black — so building the mapped palette from it meant any kit component outside
  the top bar would paint nothing, silently. The mapping is now applied over the
  kit's own resolved palette, and `unmapped_slots_survive_the_mapping` locks it.
- **Added** an aesthetic pass on the top bar: a translucent bar surface with a
  hairline edge, rounded control chips, and a resting/hover/pressed ladder
  (`0.07 / 0.16 / 0.28`) spaced for headroom. The resting chip used to land 2/255
  below the bar it sits on and was invisible; it now measures 16/255 above it.
- **Note** `WindowOptions::window_background` is left `Opaque`. `Blurred`
  (acrylic) and `MicaBackdrop` are both implemented in gpui-pre's Windows
  backend, but with a saturated window behind the app neither composited — the
  backdrop does not reach the wgpu swapchain. There is also no per-element
  backdrop blur: `blur_radius` exists only on `BoxShadow`.
- **Changed** the UI framework dependency from `gpui 0.2.2` to `gpui-pre 0.3.7`,
  the upstream Zed snapshot gpui-component builds on. Four API breaks, all
  mechanical: `Application::new()` is now an explicit platform argument,
  `Window::focus` takes `&mut App`, `KeyDownEvent` gained
  `prefer_character_input`, and a `VisualTestContext::update` closure must name
  its `&mut App` parameter. The color API is unchanged.

### Main thread — no more blocking I/O on the frame loop

- **Fixed** three folder/batch paths that did filesystem work inline on the
  UI thread, against AGENTS.md §7.1. Confirming a batch delete or move no
  longer blocks the frame loop while it trashes, moves and re-scans; neither
  opening a folder nor opening a file does either.
- **Fixed** opening a folder scanned the same directory TWICE, once in the
  folder entry point and again behind the file open it delegated to. It is now
  one scan per open.
- **Changed** two behaviours that came with moving the work off-thread, both
  documented at the call sites: `no images in <folder>` is now shown when the
  scan actually returns empty rather than while it is in flight (claiming it
  earlier would be a claim about work still running), and the folder grid
  shows its neutral empty state during a load instead of showing the previous
  folder's thumbnails under chrome that was just reset.
- **Changed** back-to-back folder opens within one frame collapse to the last
  request, so an intermediate folder no longer enters the recents list. A
  frame is ~8 ms, so this needs two opens inside the same frame to observe.

### Settings — `show_hidden_files` now does something

- **Fixed** the hidden-files toggle being a switch that lied: it flipped and
  persisted a boolean that no scan ever read, so the grid kept showing whatever
  the previous value produced, through a restart. The flag now reaches every
  folder listing, and flipping the toggle re-scans the folder on screen — which
  is the part that makes it mean something. Flipping only the boolean would
  have moved the bug rather than removed it.
- **Added** a Windows hidden-attribute check alongside the dot-prefix rule, so
  a plain-named file Explorer greys out is treated as hidden. The dot prefix
  remains the universal rule and the only one that survives a FAT32 stick, a
  zip extract and an ext4 mount identically.
- **Note** a file named explicitly on the command line is still opened even
  when it is hidden. The toggle keeps one meaning — what a folder *listing*
  shows — so `sh-images .photos/holiday.png` does not silently vanish.

### Settings — `max_decode_dimension` is now honored

- **Fixed** `max_decode_dimension` being a dead setting: it was written to
  `settings.json` and never read by anything, so lowering it changed no
  behavior at all. It now caps the two paths that decode a full frame —
  the transparency probe run on every navigation (including the
  neighbor prefetch) and the crop behind "Copy"/"Save…". Lowering it
  measurably cuts the allocation per arrow keypress on large images.
- **Changed** crop export under a cap: the exported region is a crop of the
  CAPPED frame, so its resolution follows the setting (a 100x100 region of
  a 6000px image exports at ~9x9 under a 512px cap). With the default
  8192 cap, and any image at or under it, output is unchanged — the
  setting is doing what it says rather than silently downgrading quality.
- **Note** a bounded transparency probe can miss a transparent feature
  smaller than one downscale pixel and render it opaque. That trade is
  deliberate: a full-frame decode per navigation is the larger cost, and
  the affected case (a sub-pixel dot in a poster-sized PNG) is cosmetic.
- **Note** `probe_dimensions` is deliberately NOT capped — it feeds
  fit/zoom math, so capping it would zoom against a frame the viewer is
  not showing. Thumbnails keep their own fixed 256px cap, uncoupled from
  this setting.

### Settings — removed `cache_memory_limit_mb`

- **Removed** the `cache_memory_limit_mb` setting. It had no UI row and no
  reader: GPUI's `img()` element already keeps its own path-keyed texture
  cache, so the value never controlled anything a user could observe.
  Existing `settings.json` files that still carry the key keep loading
  with every other preference intact — no schema bump and no migration
  step, because an unknown key is ignored on read.
- **Removed** the unused `sh_core::cache::DecodeCache` LRU. It duplicated
  the caching GPUI already does and was reachable only from its own tests.

### Appearance — user themes are now discoverable

- **Added** discovery of user themes: the Appearance picker now lists every
  `.json` file in the config `themes/` directory alongside the four
  built-ins, so a theme can be shared as a single JSON file and picked
  without hand-editing `settings.json`. The directory is re-scanned each
  time Settings opens (never during render, so the scan stays off the
  frame loop); until the scan lands, the built-ins render as before.
- **Added** hot reload for a picked user theme: selecting one points the
  watcher at that file, so editing it in an editor applies live exactly
  like a built-in does.
- **Fixed** the built-in/user-copy duplicate: first launch writes a copy of
  the active built-in into the themes directory so it stays editable, so
  naive discovery would show that theme twice. Rows are now deduped by
  theme name and the USER'S FILE WINS, keeping one row pointed at the
  editable file. Two different themes that declare the same `name` also
  collapse to one row — rename either file to separate them.
- **Changed** an invalid theme file no longer breaks the picker: the file
  is listed dimmed and takes no click, instead of vanishing (where "not
  found" and "broken" would look identical) or taking the whole picker
  down with it. The failure is still logged, and the hot-reload watcher
  still reports it.

### Release readiness — current viewer and release evidence

- **Added** Settings → Appearance rows for filmstrip, transparency board,
  slideshow interval, and reduced motion. Values persist in settings schema
  v10; filmstrip and board default on, the interval defaults to 3 seconds
  (1–60), and reduced motion defaults on.
- **Changed** slideshow from a permanent three-second loop to a cancellable,
  re-armable App task. An interval change while playback is active starts a
  fresh validated delay; folder changes, crop, leaving Viewer, or stopping
  cancel the pending task.
- **Added** the viewer filmstrip: viewer-only, fixed 104 px bottom strip,
  a current-image window of up to ±24 cells, centered active cell,
  cached thumbnails with neutral placeholders, and click-to-navigate.
- **Added** the transparency board controlled by the Checkerboard setting.
  It appears only after a cached alpha probe confirms transparency; the
  current GPUI 0.2.2 implementation is one fixed-gray underlay, not a tiled
  pixel-golden surface.
- **Added** reduced-motion-aware 150 ms ease-out hover transitions for
  bounded control backgrounds. Reduced motion makes those changes instant;
  no layout, grid, or window animation is introduced.
- **Changed** Viewer controls to use the stronger theme-aware hover treatment:
  topbar and viewer-surface Back/Settings/Crop, Open, Sort, density, Info,
  Previous, Slideshow, Next, and enabled zoom presets. Grid and non-interactive
  Viewer surfaces keep their existing behavior; reduced motion remains instant.
- **Added** grid viewport culling: only rows intersecting the manual-scroll
  viewport build thumbnail content and click listeners, while off-screen
  cells retain fixed-size layout placeholders.
- **Added** deterministic structural tests and stable selectors for Welcome,
  Grid, Viewer, and Settings. These check bounds and visibility; screenshots
  remain manual evidence.
- **Changed** Settings keyboard flow: `Ctrl+,` opens Settings, `Esc` returns
  to the originating view, `Tab`/`Shift+Tab` cycles five Appearance keyboard
  stops (four rows; interval has separate decrement/increment stops), and
  unmodified `Enter`/`Space` activates the focused control.
- **Changed** PR CI to run fmt, Clippy, and workspace tests; the dedicated
  Windows Release Build workflow runs the release build on pushes to `main` and
  manual dispatch.
- **Note**: Theme Editor is deferred to the next release. This release does
  not claim pixel-golden coverage or unmeasured FPS improvements.

### Packaging, branding, and public documentation

- **Added** application icon branding: `assets/branding/sh-images-icon.svg` is
  the master vector mark and `assets/branding/sh-images.ico` is the
  multi-resolution Windows icon, compiled into the executable by
  `embed-resource` so the window and the taskbar carry the Sh Images identity.
- **Added** the Inno Setup 6 installer at `installer/sh-images.iss`: a per-user
  install to `{localappdata}\Programs\Sh Images` with
  `PrivilegesRequired=lowest`, so no administrator rights and no UAC prompt.
  The fixed `AppId` is the upgrade identity, the MIT `LICENSE` is shown during
  setup and installed next to the executable, `[Icons]` creates a Start Menu
  group plus an optional desktop shortcut, and `CreateUninstallRegKey` gives
  users an Add/Remove Programs entry. **No file associations are registered** —
  the `[Registry]` block is intentionally empty, because per-user `HKCR` handler
  entries take over the user's default handler and deserve their own reviewed
  change.
- **Added** the release artifact pipeline in
  `.github/workflows/release.yml`: one `cargo build --release -p sh-app` feeds
  both a portable ZIP and the installer, so the published artifacts cannot
  disagree about which executable they ship. A silent install/uninstall smoke
  test asserts the installed payload, the uninstaller, and the Add/Remove
  Programs key, and `SHA256SUMS.txt` is computed last so it covers every
  published file.
- **Added** public project documentation: a Download/Install, Upgrading,
  unsigned-installer, and checksum-verification path in `README.md`;
  `NOTICE` covering the Lucide and Feather icon path data embedded in the
  binary and pointing at `Cargo.lock` for the linked crate set; and
  `CONTRIBUTING.md`, `SECURITY.md`, and `CODE_OF_CONDUCT.md`.
- **Changed** `docs/RELEASE_QA.md` corrects two statements that had become
  false. The CI and documentation review was performed with `actionlint`
  1.7.12 plus a real YAML parse, not with the unavailable tooling the record
  claimed. Hosted Windows CI has run and completed, so its timings are now
  recorded from the actual runs instead of being listed as pending.
- **Note**: the update model is reinstall-over. There is no in-app updater, and
  no code in this project's own crates opens a network connection; a user
  upgrades by running the new installer over the existing installation. User
  data lives in `%APPDATA%\sh_images`, outside the install directory, so
  settings, themes, and recents survive every install, upgrade, and uninstall.
- **Note**: the installer is not code-signed, so Windows SmartScreen shows
  "Unknown publisher" and the user has to choose *More info* → *Run anyway*.
  Every user-facing document says so instead of implying the download is safe
  by default.
- **Note**: no release exists. The `0.1.0` in `[workspace.package]` is the
  version the build stamps, not a published version, and the version for the
  first official release is still an open decision.

### Fixed

- **Fixed** a panic when cropping an image larger than the decode cap. The
  crop rectangle was validated against the uncapped image header and then cut
  from a downscaled buffer, so a region low in a >8192px image handed
  out-of-bounds coordinates to the crop routine, which panics. This was
  reachable before this release; the rectangle is now projected into the
  decoded frame's coordinate space, so it cannot drift from the decode.
- **Fixed** Viewer Back coexistence: a persistent localized Back control stays

  in the viewer main area while the dissolved topbar's lower arrows/overlay
  come and go, and direct Previous/Next plus filmstrip clicks refresh the idle
  interaction clock. The solid topbar keeps its existing Back control.
- **Fixed** filmstrip visibility: the ±24 row is positioned from the live viewport so the current thumbnail stays centered and visible at folder boundaries and narrow windows.
- **Fixed** dead zoom preset chips: a preset whose scale sits at or below
  the fit floor (e.g. 100% on a small image) now renders disabled (dimmed,
  non-clickable) instead of snapping back to fit on tap; the Fit chip
  carries the active marking there. Chips stay enabled while dimensions
  are still probing.
- **Fixed** varying open zoom: navigate/open completion now fits against
  the full window (stable viewport), independent of the transient
  idle/topbar state, so the same image always opens at the same zoom.
- **Fixed** oversized Tab overlay: the bottom bar is now one compact
  fixed-height row (40px, the topbar scale — no wrap, clipped overflow),
  and while it is armed the topbar dissolves, so total chrome never
  stacks two solid bars. Tab still toggles the bottom bar; idle still
  fades it to zero chrome.
- **Fixed** dead info button: it lived inside the auto-hiding bottom bar
  and had no hitbox with Tab OFF or after idle. It now lives top-right
  in the viewer whenever an image is shown — reachable in every Tab/idle
  state. With Tab ON it rides the floating chips row as its third slot
  (the standalone float used to be buried under the gear/crop chips);
  Tab OFF keeps the standalone float below the bar. Toggle + popover
  behavior unchanged.
- **Fixed** image jolt on Tab toggle: the fit viewport no longer depends
  on chrome. The topbar floats over the image in the Viewer (zero layout
  space, full-window fit area in every Tab state) and toggling Tab fires
  no refit — the image stays pixel-static across toggles. Grid keeps the
  in-flow bar. Tradeoff (deliberate UX change): the solid topbar now
  covers the top 40px of the image with Tab OFF instead of squeezing the
  fit area.

### Viewer — one-tap info popover

- **Added** a click-only info chip in the viewer bottom overlay: one tap
  opens a dismissible popover showing exactly three facts for the current
  image — pixel dimensions, file size, and format (no-EXIF slice).
- **Added** `probe_file_info` + `format_file_size` in `sh-core`: dimensions
  resolve from the image header alone (no full decode, cheap on 4K/8K),
  size from one `metadata` stat, format as the canonical uppercase name.
- **Added** five i18n keys (EN + neutral ES: Dimensions/Dimensiones,
  Size/Tamaño, Format/Formato, Show image info, Could not read image info)
  with anti-drift coverage (63 keys).
- **Note**: dismiss on button re-tap, `Esc`, or outside-click;
  navigate-while-open updates the facts in place; visibility is transient
  (no settings knob, no persistence); corrupt/missing files show the
  localized error instead of crashing.

### Viewer — one-tap zoom preset chips

- **Added** three click-only zoom preset chips (Fit / 100% / 200%)
  in the viewer bottom overlay: one tap lands the viewer on the exact
  scale (100% = actual pixels), centered on the viewport midpoint.
  Sub-floor requests (e.g. 100% on a small image) snap back to fit per the
  existing clamp contract.
- **Added** `Session::set_zoom_preset`: every preset funnels through the
  same `zoom_at` → `clamp_zoom` path as the wheel (floor snap-back band
  pinned as `FIT_SNAP_REL_EPS`); the Fit chip reuses the exact
  `toggle_fit_100` fit semantics.
- **Added** shared overlay `action_button` helper (optional pill chrome):
  prev/next/slideshow migrate bare (pixel-identical), preset chips opt
  into the topbar density-control look.
- **Added** `ZoomPresetFit` / `ZoomPreset100` /
  `ZoomPreset200` i18n keys (EN + neutral ES: Fit/Ajustar; numerals
  locale-neutral by design) with anti-drift coverage (58 keys).
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
- **Added** slideshow: auto-advance the viewer at the persisted interval
  (default 3 seconds, configurable from 1–60), toggled by `Space` or a
  play/pause chip in the bottom overlay; stops on folder switch, crop, or
  leaving Viewer and is mutually exclusive with crop mode.
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

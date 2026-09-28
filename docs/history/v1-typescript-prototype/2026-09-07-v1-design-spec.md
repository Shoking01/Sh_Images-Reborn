# Sh_Images — V1 Design (2026-09-07)

Status: Approved design (brainstorming complete, pending user spec review)
Stack: Bun + TypeScript (strict) + React on `@gpuix/react` (Zed's GPUI via React bindings)

---

## 1. Context

Sh_Images is a native, GPU-accelerated image viewer for Windows (first), written with
GPUIX (React + TypeScript over Zed's GPUI). Priorities, in order:

1. Minimal RAM and CPU consumption.
2. Modern aesthetics and fluid image viewing.
3. Core features: open folders, navigate images, crop and export (save new file or
   copy to clipboard), and be selectable as the default image viewer (Windows).

Working constraints come from `AGENTS.md` (project-local, not committed): no `any`,
strict TS, no unhandled rejections, typed errors, no new dependencies without
justification, performance budgets (open 1080p < 100 ms, idle RAM < 80 MB, min
30 FPS, etc.), test-coverage minimums, and mandatory integration flows.

## 2. Decisions (approved in brainstorm)

| # | Decision | Detail |
|---|---|---|
| D1 | Stack | TypeScript + React on `@gpuix/react`. No raw Rust, no egui/eframe/Electron. |
| D2 | Platform | Windows-first V1; code kept portable (macOS/Linux registrations later). |
| D3 | Formats V1 | PNG, JPEG, GIF (first frame), WebP, BMP, TIFF. |
| D4 | Formats Phase 2 | AVIF/HEIC (native Windows codecs), GIF animation, RAW. |
| D5 | Default theme | Cinema Dark "Lightbox" (image-first, auto-hide chrome). |
| D6 | Theming | Design-token JSON themes; user themes in app-data `themes/*.json`; validated schema. |
| D7 | Icons | Inline SVG, monochrome, tinted by theme tokens (`style.color` via GPUIX icon renderer). |
| D8 | Crop UX | `C` enters crop; drag selection + handles; dimmed outside + rule-of-thirds; aspect presets (Free, 1:1, 16:9, 4:3, 3:2); Enter confirm / Esc cancel; export → save PNG/JPEG or copy PNG. |
| D9 | Default viewer | Full Windows registration (ProgID, HKCU, no admin) at first run; user chooses final default in Windows Settings; app never overrides an existing default unilaterally. |
| D10 | Opening flows | Native dialog (`Ctrl+O`), drag & drop (folder or image), and system open (CLI arg → open image + derive containing folder). |
| D11 | Navigation | Validated shortcut table (see below); wheel navigates images; Ctrl+wheel zooms. |
| D12 | Keymap | Action registry + user `keymap.json` (validated, conflict detection); hand-edited JSON in V1; in-app editor is Phase 2. |
| D13 | Image pipeline | Option A: native `<img>` for display; worker-built downsampled thumbnails; decode-on-demand for crop/export. |
| D14 | Settings | `settings.json` with version migration; V1 fields: active theme, last folder, cache caps. |
| D15 | Phase 2 bundle | AVIF/HEIC, settings panel UI, icon shape overrides, GIF animation, RAW, multi-window, keybind editor UI, theme marketplace. |

## 3. Architecture

```
src/
├── main.tsx          # Entry; ends with render(). CLI arg parsed here or in app.tsx.
├── app.tsx           # Root: providers, global key handler, window-level events.
├── core/
│   ├── image-cache.ts    # Thumbnail LRU cache policy (pure logic, testable).
│   ├── navigation.ts     # Folder indexing, extension filter, sorting, circular nav.
│   ├── crop-math.ts      # Selection rect → source pixel region, aspect math, zoom-aware transform.
│   ├── theme/index.ts    # Token types, default themes, merge/resolve.
│   ├── theme/schema.ts   # JSON schema validation for themes (-> ShImagesError CONFIG_ERROR).
│   ├── actions.ts        # Action ID registry + default keybindings table.
│   ├── keymap.ts         # keymap.json parse/validate/merge/conflict detection.
│   └── formats.ts        # Supported extensions, MIME mapping, filename predicates (no React).
├── hooks/
│   ├── use-image-loader.ts   # Wraps native <img> loading state + fallback placeholder.
│   ├── use-thumbnails.ts     # Drives worker decode pool for grid thumbs.
│   ├── use-pan-zoom.ts       # Zoom/pan state machine -> crop-math + style props.
│   ├── use-keymap.ts         # Registers global keydown dispatch against actions.
│   └── use-theme.ts          # Reads active theme tokens from context.
├── components/
│   ├── image-view.tsx    # Single-image view (auto-hide chrome, floating toolbar, filmstrip).
│   ├── folder-grid.tsx   # Thumbnail grid view.
│   ├── crop-overlay.tsx  # Crop selection UI over image view.
│   ├── export-dialog.tsx # Save/Copy choice after crop confirm.
│   ├── titlebar.tsx      # Window chrome (custom or native) — Windows-first.
│   └── icons/            # Inline SVG icon components (monochrome, theme-tinted).
├── config/
│   ├── settings.ts       # settings.json load/save, defaults, version migration.
│   └── paths.ts          # App data dir resolution (Windows %APPDATA%), themes dir, cache dir.
├── workers/
│   ├── thumb-worker.ts   # Decode + downsample (<=256px) to cache files (@napi-rs/image).
│   └── crop-worker.ts    # Decode source region + encode PNG/JPEG, on demand.
└── utils/
    ├── errors.ts         # ShImagesError (DECODE_ERROR, IO_ERROR, UNSUPPORTED_FORMAT, CONFIG_ERROR, UNKNOWN).
    ├── logger.ts         # pino wrapper; min level `info` in production.
    ├── clipboard.ts      # OS clipboard write (image) via @napi-rs/clipboard.
    ├── file-dialog.ts    # Native open-folder/file dialog.
    └── registration.ts   # Windows ProgID + file-association registration (HKCU) + CLI arg handling.
```

### 3.1 Image pipeline (Option A — approved)

- **Single-image view**: native GPUIX `<img src={originalPath} objectFit>` — Rust-side
  decode by GPUI's asset system, its own LRU image cache, direct GPU upload. **No decoded
  pixels ever live in JS** for viewing.
- **Grid view**: worker pool (`thumb-worker.ts`) decodes each image once and writes a
  downsampled thumbnail (max edge 256 px) to an app-data cache dir; the grid renders
  `<img src={cacheFile}>`. `core/image-cache.ts` owns LRU eviction and disk-size caps;
  stale cache cleaned on exit. This prevents full-resolution decodes for hundreds of
  images.
- **Crop/export**: entering crop mode triggers `crop-worker.ts` — decode the source
  (region-aware where the decoder supports it; otherwise decode then crop), run
  `core/crop-math.ts` to map the on-screen selection to source pixel coordinates (with
  zoom/pan transform). On confirm: encode PNG (default, alpha-preserving) or JPEG, save
  via native save dialog, or copy to clipboard as image (`@napi-rs/clipboard` →
  CF_DIB/PNG on Windows). Buffers are released immediately; nothing is retained after export.

### 3.2 Theme system

- `core/theme/schema.ts` defines the token surface: surface colors, text, accent, borders,
  radii, spacing, opacity, blur, icon color/muted/stroke width, typography (family, sizes),
  plus a `meta` block (name, author, version).
- Bundled themes: **Lightbox** (default, Cinema Dark), **Terminal** (monospace, green/amber
  palette, square radii), **Studio Light**.
- User themes: JSON files in app-data `themes/`; validated at load; invalid files are
  rejected with `ShImagesError(CONFIG_ERROR)` and a clear message — never a crash.
- Components consume tokens via `useTheme()`; all colors/radii/fonts flow from tokens, no
  hardcoded colors in components.

### 3.3 Keymap system

- `core/actions.ts`: typed registry of action IDs (navigateNext, navigatePrev, zoomIn,
  zoomOut, zoomActual, zoomFit, toggleFullscreen, toggleChrome, crop, confirmCrop,
  cancelCrop, saveCrop, copyCrop, openFolder, openFile, goBack) with defaults.
- `core/keymap.ts`: parses `keymap.json`, validates against the schema, merges with
  defaults, and reports binding conflicts (user's explicit assignment wins with a warning).
- `use-keymap.ts` in `app.tsx`: global `keyDown` dispatch, chord normalization, action call.
- The validated shortcut table (defaults):

| Action | Default |
|---|---|
| Next / previous image | `ArrowRight` / `ArrowLeft` (and mouse wheel in single view) |
| Zoom in / out | `Ctrl`+wheel, `+` / `-` |
| Zoom 100% / fit | `0` / `F` |
| Fullscreen | `F11` or double-click image |
| Back to grid | `Esc` (or Back button) |
| Crop / confirm / cancel | `C` / `Enter` / `Esc` |
| Save crop / copy crop | `Ctrl+S` / `Ctrl+Shift+C` |
| Toggle chrome (immersive) | `H` |
| Open folder / open file | `Ctrl+O` / `Ctrl+Shift+O` |

### 3.4 Windows integration (default viewer + opening)

- First run: write ProgID + file associations (HKCU, no admin) so Sh_Images appears in
  "Open with…" and as an eligible default app in Windows Settings. Never writes the
  default handler choice; that is the user's decision in Settings.
- Launch: `sh_images.exe "C:\photo.jpg"` → open that image and load its containing folder
  for arrow navigation. No argument → start at the grid with folder dialog / recents.
- Drag & drop a folder or an image file onto the window loads it.
- All registry writes are isolated to HKCU and are the only OS-mutating surface.

### 3.5 Error handling and logging

- All failure paths throw/map to `ShImagesError` with codes:
  `DECODE_ERROR | IO_ERROR | UNSUPPORTED_FORMAT | CONFIG_ERROR | UNKNOWN`, plus `cause`.
- Top-level handlers catch and surface a visible message (no silent failures).
- `utils/logger.ts` (pino wrapper): error/warn/info/debug/trace; production min `info`.
- No `any` (justify with comment if ever unavoidable); no unhandled rejections in event handlers.

## 4. Config and persistence

- `config/settings.json` in app data: `{ version, activeTheme, lastFolder, cache: { thumbMaxEdgePx, maxCacheBytes } }` (an explicit decoded-image count cap may be added later if profiling shows the native LRU needs it).
- Version field enables migration on load; unknown/invalid settings fall back to defaults
  with a logged warning, never a crash.
- Paths resolved through `config/paths.ts` (Windows `%APPDATA%\Sh_Images`; themes/
  and cache/ subdirs).

## 5. Testing and QA

Per `AGENTS.md`:

- Unit coverage: `core/` ≥ 90%, `utils/` ≥ 85%, `hooks/` ≥ 70%, total ≥ 75%.
- Mandatory tested units: `image-cache` (LRU eviction, memory limits, hit/miss),
  `navigation` (circular nav, extension filter, sorting), `crop-math` (coordinate
  transform, aspect limits), theme schema, keymap parse/merge/conflict, `settings`
  (defaults, serialization, version migration), formats predicates.
- Integration flows: open flow, navigation flow, zoom/pan flow, error flow (corrupt file
  → visible error, no crash), config flow (persist across restart).
- Fixtures in `tests/fixtures/`: valid PNG/JPEG/GIF, corrupt `.png` (random bytes), empty file.
- Benchmarks: open 1080p < 100 ms, 4K < 200 ms, 8K < 500 ms; navigation < 50 ms;
  idle RAM < 80 MB; 4K open RAM < 200 MB; pan/zoom ≥ 30 FPS.

## 6. Spikes / risks to verify at implementation start

1. `@napi-rs/image` and `@napi-rs/clipboard` load and work under Bun on Windows
   (N-API modules; clipboard image write as CF_DIB/PNG).
2. GPUIX `<img>` behavior: arbitrary native filesystem paths for `src`, cache eviction
   semantics, objectFit behavior for all V1 formats; `keyDown`/`keyUp` event shape and
   focus requirements for global shortcuts.
3. Thumbnails as cached files vs `data:` URLs for `<img>` (prefer files with cleanup;
   revisit if GPUIX accepts PNG data URLs).
4. Native save dialog availability from JS (GPUIX/GPUI picker) — alternative: a simple
   in-app filename input fallback.
5. GIF first-frame handling (GPUI decodes GIF; confirm only frame 0 is shown in V1).

## 7. Non-goals (Phase 2)

AVIF/HEIC, GIF animation, RAW, settings panel UI, in-app keybind editor, icon shape
override, multi-window, theme marketplace, macOS/Linux default-viewer registration,
web/Wasm build tuning.

## 8. Documentation mapping

- This design doc lives in `docs/superpowers/specs/` (working design artifact).
- Per `AGENTS.md`, architecturally significant decisions also get ADR entries in
  `docs/ARCHITECTURE.md` as they are implemented (framework choice, image pipeline,
  caching strategy, theming, Windows registration).
# Sh_Images

Native, GPU-accelerated image viewer for Windows, built with Rust +
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui). No
Electron, no web views, no runtime GC — the UI renders directly through the
GPU via DirectX. Built as a fast, minimal replacement for Windows Photos
for single-image viewing.

## Build

```sh
cargo build --release -p sh-app
```

Binary: `target/release/sh-app.exe`.

## Usage

```sh
sh-app.exe                        # Welcome screen (Continue / Open folder)
sh-app.exe "C:\path\to\folder"    # Folder grid
sh-app.exe "C:\path\to\image.png" # Viewer, with its sibling folder
```

Flow: **Welcome → Grid → Viewer**. The welcome screen offers to continue in
the last folder or pick a new one; the grid shows folder thumbnails (click to
view, arrows + `Enter` work too); the viewer shows one image with a top bar
and bottom controls. The top bar dissolves when the bottom controls are
active. You can also:

- **Drag & drop** an image or a folder onto the window.
- Press **Ctrl+O** for a native file dialog, **Ctrl+Shift+O** for a folder
  dialog (any view).

### Controls

| Input                        | Action                                    |
| ---------------------------- | ----------------------------------------- |
| Mouse wheel (viewer)         | Zoom, anchored at the cursor              |
| Mouse wheel (grid)           | Scroll thumbnails                         |
| Drag (viewer)                | Pan (only when zoomed)                    |
| Drag (viewer, crop mode)     | Select crop region                        |
| Double-click                 | Toggle 100% / fit-to-window               |
| ← / →                        | Previous / next image, or grid selection  |
| Enter (grid)                 | Open selected image                       |
| Enter (crop bar visible)     | Copy the selection                        |
| C (viewer)                   | Toggle crop mode                          |
| Space (viewer)               | Start or stop slideshow                   |
| Esc (viewer)                 | Back to grid                              |
| Esc (crop mode)              | Leave crop mode, discard selection        |
| Tab (viewer)                 | Show/hide the bottom bar (zoom + arrows)  |
| F11                          | Toggle fullscreen                         |
| Ctrl+,                       | Open Settings                              |
| Esc (Settings)               | Return to the view that opened Settings   |
| Ctrl+Shift+O                 | Open folder dialog                        |

### Viewer settings

Open **Settings → Appearance** with `Ctrl+,`. `Esc` returns to the view that
opened it. In the Appearance section, `Tab` and `Shift+Tab` move through five
keyboard stops in order (four rows; the interval row has separate decrement
and increment stops); unmodified `Enter` or `Space` activates the focused
control. Changes are sent through the shared atomic settings writer.

| Control | Current behavior |
| --- | --- |
| Filmstrip | Defaults on. The Viewer mounts a fixed 104 px bottom strip, builds a window around the current image (up to ±24 cells), keeps the active cell centered, and navigates on click. Missing thumbnails use placeholders. |
| Checkerboard | Defaults on. A fixed-gray transparency underlay appears only after a cached alpha probe confirms transparency; opaque and verdict-pending images keep their normal background. The current GPUI 0.2.2 implementation is one underlay, not a tiled pixel-golden surface. |
| Slideshow interval | Defaults to 3 seconds and accepts 1–60 seconds. Changing it while playback is active re-arms the delay; folder changes, crop, leaving Viewer, or stopping cancel the pending task. |
| Reduced motion | Defaults on because the platform has no OS reduced-motion query. On means instant hover changes; off enables only the bounded 150 ms background-hover transition. Layout, grid, and window positions do not animate. |

### Crop

Press **C** in the viewer (or the ✂ top-bar button) to enter crop mode, then
drag to select a region. A confirm bar appears on release:

- **Copy** (or `Enter`) — copies the region to the clipboard; paste anywhere
  (Paint, Photoshop, chat apps).
- **Save…** — opens a save dialog filtered to PNG, defaulting to
  `<original>_crop.png` in the image's folder. Lossless.
- **Cancel** (or `Esc`) — discards the selection and leaves crop mode.

The crop always cuts full-resolution pixels (never the fitted/zoomed view),
runs in the background (no UI freeze on large images), and clamps to image
bounds. Tiny accidental selections (< 4 px²) are discarded silently.

Overlays (the bottom bar: zoom, prev/next) auto-hide after ~1.5 s of
inactivity. Filename and position remain in the top bar.

## Themes

Themes are JSON files in `%APPDATA%\sh_images\themes\` and **hot-reload**
about one second after you save a change. Invalid edits are warned once and
skipped — the previous theme stays applied.

Built-in themes (written to the config dir on first launch): **Noir
Gallery** (default), **Deep Neutral**, **Dark Clinical**, **Light Clean**.
Switch by editing `"theme"` in settings.json — a missing file is bootstrapped
from the built-in on next launch.

## Settings and release evidence

`%APPDATA%\sh_images\settings.json` is written atomically and currently uses
schema v10. It persists the theme, recent/last folder, sort and grid choices,
language, keymap, filmstrip and checkerboard visibility, slideshow interval,
and reduced motion. Older valid files receive safe defaults without losing
their other preferences.

The automated test harness checks structural selectors, bounds, and render
state; GPUI 0.2.2 does not capture real pixels. Release screenshots, timing,
frame-time, memory observations, and launch checks therefore remain manual.
See [`docs/RELEASE_QA.md`](docs/RELEASE_QA.md) for the evidence boundary and
checklist.

## Development

```sh
# Full quality gate
cargo check --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --release -p sh-app

# Decode benchmark (1080p PNG fixture)
cargo bench -p sh-core --bench decode_bench
```

Workspace layout:

- `crates/sh-core` — pure logic: navigation, decode, cache, transform,
  theme, settings. No GPUI, `#![forbid(unsafe_code)]`.
- `crates/sh-app` — GPUI application: viewer, overlays, session state,
  platform glue.

Architecture decisions: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

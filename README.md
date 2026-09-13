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

Binary: `target/release/sh-app.exe` (~9 MB).

## Usage

```sh
sh-app.exe                        # Welcome screen (Continue / Open folder)
sh-app.exe "C:\path\to\folder"    # Folder grid
sh-app.exe "C:\path\to\image.png" # Viewer, with its sibling folder
```

Flow: **Welcome → Grid → Viewer**. The welcome screen offers to continue in
the last folder or pick a new one; the grid shows folder thumbnails (click to
view, arrows + `Enter` work too); the viewer shows one image with a persistent
top bar (`← Grid`, image name, open-folder button). You can also:

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
| Esc (viewer)                 | Back to grid                              |
| Esc (crop mode)              | Leave crop mode, discard selection        |
| Tab (viewer)                 | Show/hide overlays                        |
| F11                          | Toggle fullscreen                         |
| Ctrl+Shift+O                 | Open folder dialog                        |

### Crop

Press **C** in the viewer (or the ✂ top-bar button) to enter crop mode, then
drag to select a region. A confirm bar appears on release:

- **Copiar** (or `Enter`) — copies the region to the clipboard; paste
  anywhere (Paint, Photoshop, chat apps).
- **Guardar…** — opens a save dialog filtered to PNG, defaulting to
  `<original>_crop.png` in the image's folder. Lossless.
- **Cancelar** (or `Esc`) — discards the selection and leaves crop mode.

The crop always cuts full-resolution pixels (never the fitted/zoomed view),
runs in the background (no UI freeze on large images), and clamps to image
bounds. Tiny accidental selections (< 4 px²) are discarded silently.

Overlays (filename, position, zoom, prev/next) auto-hide after ~1.5 s of
inactivity.

## Themes

Themes are JSON files in `%APPDATA%\sh_images\themes\` and **hot-reload**
about one second after you save a change. Invalid edits are warned once and
skipped — the previous theme stays applied.

Built-in themes (written to the config dir on first launch): **Noir
Gallery** (default), **Deep Neutral**, **Dark Clinical**, **Light Clean**.
Switch by editing `"theme"` in settings.json — a missing file is bootstrapped
from the built-in on next launch.

## Settings

`%APPDATA%\sh_images\settings.json` persists the active theme and the last
opened directory (saved atomically; user-edited fields survive saves).

## Development

```sh
# Full quality gate (AGENTS.md §5.1)
cargo check --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check && cargo test --workspace

# Decode benchmark (1080p PNG fixture)
cargo bench -p sh-core --bench decode_bench
```

Workspace layout:

- `crates/sh-core` — pure logic: navigation, decode, cache, transform,
  theme, settings. No GPUI, `#![forbid(unsafe_code)]`.
- `crates/sh-app` — GPUI application: viewer, overlays, session state,
  platform glue.

Architecture decisions: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

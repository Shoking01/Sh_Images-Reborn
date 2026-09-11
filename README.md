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
sh-app.exe "C:\path\to\image.png"
```

Opens the image and its sibling folder. You can also:

- **Drag & drop** an image onto the window.
- Press **Ctrl+O** for a native file dialog.

### Controls

| Input                        | Action                                    |
| ---------------------------- | ----------------------------------------- |
| Mouse wheel                  | Zoom, anchored at the cursor              |
| Drag                         | Pan                                       |
| Double-click                 | Toggle 100% / fit-to-window               |
| ← / →                        | Previous / next image (circular)          |
| Tab                          | Show/hide overlays                        |
| F11                          | Toggle fullscreen                         |

Overlays (filename, position, zoom, prev/next) auto-hide after ~1.5 s of
inactivity.

## Themes

Themes are JSON files in `%APPDATA%\sh_images\themes\` and **hot-reload**
about one second after you save a change. Invalid edits are warned once and
skipped — the previous theme stays applied.

Built-in themes (written to the config dir on first launch): **Deep
Neutral**, **Dark Clinical**, **Light Clean**.

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

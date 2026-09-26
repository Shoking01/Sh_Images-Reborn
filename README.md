# Sh_Images

![Sh Images](assets/branding/sh-images-icon.svg)

Native, GPU-accelerated image viewer for Windows, built with Rust +
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui). No
Electron, no web views, no runtime GC — the UI renders directly through the
GPU via DirectX. Built as a fast, minimal replacement for Windows Photos
for single-image viewing.

[![CI](https://img.shields.io/github/actions/workflow/status/Shoking01/Sh_Images-Reborn/Windows%20Quality%20Gates?label=CI)](https://github.com/Shoking01/Sh_Images-Reborn/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/github/license/Shoking01/Sh_Images-Reborn?label=license)](LICENSE)

**Windows only.** The binary targets `x86_64-pc-windows-msvc` and the
installer is built for `x64compatible` (x64, plus ARM64 Windows that can run
it). There is no macOS or Linux build.

[Changelog](CHANGELOG.md) ·
[License](LICENSE) · [Notice](NOTICE) ·
[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) ·
[Code of Conduct](CODE_OF_CONDUCT.md) ·
[Architecture decisions](docs/ARCHITECTURE.md) ·
[Release QA](docs/RELEASE_QA.md) ·
[Issues](https://github.com/Shoking01/Sh_Images-Reborn/issues)

## Download and install

**No release is published yet.** The links below point at this repository's
releases page, which becomes usable as soon as the first release is published;
until then the page is empty. Watch the repository or the
[CHANGELOG](CHANGELOG.md) for the announcement.

Two artifacts are produced per release:

| Artifact | What it is | Use it when |
| --- | --- | --- |
| **Installer** — `ShImages-Setup-<version>-win-x64.exe` | Inno Setup 6 package, installs per user | Recommended. Creates Start Menu and Add/Remove Programs entries and an optional desktop shortcut. |
| **Portable ZIP** — `ShImages-<version>-win-x64.zip` | The `sh-app.exe` executable plus the `LICENSE`, no installation | You want to run it from a USB stick or a managed environment with no installer. |

Releases page:
<https://github.com/Shoking01/Sh_Images-Reborn/releases>

**The installer requires no administrator rights and shows no UAC prompt.** It
installs per user into `%LOCALAPPDATA%\Programs\Sh Images`, so nothing is
written outside your own profile. It **does not register file associations**:
double-clicking a `.png` in Explorer will not open Sh Images, and that is
deliberate, not a bug. Launch it from the shortcut, or pass a path on the
command line (see [Usage](#usage)).

### SmartScreen: the installer is not code-signed

The installer is **unsigned**. Windows will very likely show *"Windows
protected your PC"* with an **Unknown publisher** warning, because there is no
code-signing certificate behind the file. This is expected, and it is not by
itself evidence of a problem.

To proceed: click **More info**, then **Run anyway**.

Do not disable SmartScreen, and do not blanket-allow this file. If you want
independent confirmation that what you downloaded is what was published,
verify the download against the published checksums before running it.

### Verifying downloads

Every release publishes a `SHA256SUMS.txt` next to the two artifacts above.
The three files must be in the **same directory** for the checksum file to
verify — the file references the artifacts by bare file name.

```sh
# Linux, and Git Bash on Windows
sha256sum -c SHA256SUMS.txt

# macOS: the BSD tool is `shasum`, not `sha256sum`
shasum -a 256 -c SHA256SUMS.txt
```

```powershell
# PowerShell: no -c equivalent, so hash each file and compare with
# SHA256SUMS.txt by eye.
Get-FileHash -Algorithm SHA256 .\ShImages-<version>-win-x64.zip
Get-FileHash -Algorithm SHA256 .\ShImages-Setup-<version>-win-x64.exe
```

Each line of `SHA256SUMS.txt` is `"<hash>  <file name>"` with two spaces. Both
commands print `OK` per file when the hashes match; anything else means the
download is incomplete or has been altered — do not run it.

## Upgrading

**There is no in-app updater.** Nothing in this project's own code opens a
network connection: there is no update check, no telemetry, and no remote
service. A new version only reaches your machine because you downloaded it.

To upgrade, download the new installer and **run it over the existing
installation**. The installer recognizes the previous install by its fixed
`AppId`, runs the old uninstaller, and replaces the program files in place. You
end up with one installation and one Add/Remove Programs entry, not two. If a
newer version is already installed, the older installer exits without changing
anything rather than downgrading it.

Your data lives in `%APPDATA%\sh_images` (Roaming), **outside** the install
directory (`%LOCALAPPDATA%`, Local). So settings, custom themes, and recent
folders are not touched by an install, an upgrade, or an uninstall. That is the
reason the update model is "run the new installer again" rather than something
smarter.

Portable ZIP users replace the files in their folder. Note that the ZIP carries
no uninstaller and no Add/Remove Programs entry, which is the main reason to
prefer the installer.

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

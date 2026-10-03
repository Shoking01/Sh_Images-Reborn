# Visual baselines

Reference frames for `tools/visual/capture.ps1`, one per scenario. They exist so a
human can look at what the app actually painted and decide whether it is right.

**They are not a test fixture.** No gate reads them, CI never fails on them, and
nothing regenerates them automatically. See `docs/ARCHITECTURE.md` ADR-022 for why
pixel comparison stays advisory while geometry invariants stay required.

## What each image is

| File | Scenario | What it shows |
| --- | --- | --- |
| `grid-mixed-aspect.png` | `grid-mixed-aspect` | Grid view over six generated fixtures that cycle wide (800x200), square (400x400) and tall (200x800). This is the frame the grid-cell-overflow defect was found in, and the reason the fixture set mixes aspect ratios: a tall image has to sit next to shorter ones in the same row for the defect to be visible at all. |
| `viewer-single.png` | `viewer-single` | Viewer on one square fixture. The square is deliberate — it is the only shape that exercises all four letterbox edges at once. The filmstrip along the bottom shows all six fixtures, so this frame also records how each aspect ratio is scaled in the strip. |
| `welcome.png` | `welcome` | Welcome view, launched with no argument. Covers the dropzone, the `Open folder` button and the app icon, which is where ADR-020's asset-source defect hid. |
| `empty-grid.png` | `empty-grid` | Grid view over an empty directory: the `No images in this folder` empty state. Cheap to capture and it is the one state where an empty session must not look like a failed load. |

All four are 1016x759 — the app's 1000x720 client area plus the native window
frame. That size is a property of this machine's DPI and border style, not of the
app, and it is the first thing to differ on another one.

## Regenerating them is a deliberate human action

There is no `--update-baselines` flag, on purpose. A tool that quietly refreshes
its own reference makes the reference worthless: the diff you were trying to read
disappears at the moment you run the comparison.

To re-baseline:

1. Build release: `cargo build --release`.
2. Capture: `powershell -File tools/visual/capture.ps1`.
3. **Look at the new frames in `artifacts/visual/current/` and decide whether the
   change was intended.** A frame that looks wrong is a finding, not a baseline.
4. Only then copy across:
   `Copy-Item artifacts/visual/current/*.png artifacts/visual/baseline/ -Force`
5. Commit the PNGs **in the same commit as the change that caused them**, so the
   diff shows the code change and the visual consequence together.

Never run this from CI, never wire it to a bot, and never re-baseline to make a
delta go away. Re-baselining to silence a diff is the exact failure mode that ADR-022
rejected golden images as a gate over.

## Before you re-baseline, check you are not chasing noise

The harness pins the two things that otherwise make frames differ for reasons
unrelated to the app — the mouse pointer is parked outside the window, and the
OS-drawn band above the client area is excluded from every delta. If a delta
appears anyway, read the diff PNG in `artifacts/visual/current/compare/`: the blue
band across the top is the excluded native chrome, red is "differs", and magenta
means the two frames are different sizes.

A delta concentrated in one cell with a rounded-rectangle shape is usually a hover
card caught mid-transition, not a layout change.
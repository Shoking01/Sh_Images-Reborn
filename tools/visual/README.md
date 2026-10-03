# Visual baseline harness (Windows)

Captures what Sh_Images actually painted, so a human can look at it.

**This is not a gate.** It never fails a build on a pixel difference, it is not a
required status check, and nothing regenerates the baselines automatically. If you
are looking for the mechanism that decides whether a change is acceptable, that is
the geometry invariants in `crates/sh-app/src/app.rs`
(`grid_rows_are_uniform_and_never_overlap`, `viewer_chrome_stays_inside_its_band`),
which run in `ci.yml` and need no GPU. The reasoning is in
`docs/ARCHITECTURE.md` ADR-022.

## Why it exists

The bounds-based guards cannot see a missing `overflow_hidden()`. That call clips
PAINTING without changing BOUNDS, so paint can escape a cell while every geometric
assertion stays green — which is exactly how a grid cell grew its own flex line and
painted over the row above. Only reading pixels closes that gap.

## Requirements

Windows, PowerShell 5.1, and a release build of the app:

```powershell
cargo build --release
```

No extra tooling. The script compiles its own interop helpers with `Add-Type` on
first run.

## Run it

Capture into the default `artifacts/visual/current`:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/visual/capture.ps1
```

Capture and report a delta against the committed baselines:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/visual/capture.ps1 -Compare
```

Useful switches:

| Switch | Effect |
| --- | --- |
| `-OutputDir <path>` | Where PNGs, `compare/` and `summary.json` go. Default `artifacts/visual/current`. |
| `-ExePath <path>` | The binary to launch. Default `target/release/sh-app.exe`. |
| `-Compare` | Report pixel deltas against `artifacts/visual/baseline`. Never affects the exit code. |
| `-BaselineDir <path>` | Baselines to compare against. Default `artifacts/visual/baseline`. |
| `-KeepRunRoot` | Keep the temp fixtures and hermetic profile instead of deleting them. The path is printed on failure so a bad run can be reproduced. |

Exit codes: `0` when at least one scenario produced a valid frame, `1` when nothing
valid was captured or the harness itself failed. **A pixel delta never produces
either.**

## What it does

Per scenario it launches the real binary, waits for a top-level window, then
captures with `PrintWindow(hwnd, hwnd, 2)` — `PW_RENDERFULLCONTENT`. Flag `0` and
`PW_CLIENTONLY` both return an empty frame for this composited window, and
`CopyFromScreen` was tried and rejected: it loses z-order fights and captured the
wrong window.

Four scenarios: `grid-mixed-aspect`, `viewer-single`, `welcome`, `empty-grid`.

Three guards, because a wrong frame is worse than no frame:

- **Stability gate** — a frame is accepted only after two consecutive captures are
  pixel-identical, so a half-painted frame is never written.
- **Blank detection** — an essentially uniform frame is a capture *failure* and is
  never written. If every attempt is blank the run reports that the window was
  never composited, which is what a session with no interactive desktop looks like.
- **Process hygiene** — the child is killed in a `finally`, on success and on every
  error path.

Two inputs are pinned so a diff means something:

- The **mouse pointer is parked** outside the window during capture and restored
  afterwards. A hovered grid cell paints an elevated card 180px wide against the
  160px of every other cell, which reads as a layout regression and is not one.
- The **OS-drawn band above the client area is excluded** from every delta, measured
  with `GetClientRect`/`ClientToScreen`. The native title bar is drawn by Windows
  and paints differently focused versus unfocused.

The profile is hermetic. `config_dir()` reads `APPDATA` directly, so the child is
launched with `APPDATA` pointing at a fresh temp directory: it bootstraps its own
defaults and theme and never touches your real `%APPDATA%\sh_images`. Do not
hand-author a `settings.json` for it — the app writes current defaults into a
missing profile, and a hand-written file drifts the day the schema version moves.

Fixtures are generated every run and are byte-stable across runs (verified by
SHA-256). They cycle wide (800x200), square (400x400) and tall (200x800) in solid
distinct colours so any consecutive subset mixes them; a tall image has to sit next
to shorter ones for the original defect to be visible.

## Reading the compare output

`-Compare` writes a `diff-<scenario>.png` per image plus numbers in `summary.json`
under `compareResults`:

- **red / orange** — pixels that differ, brighter with magnitude.
- **blue band across the top** — the excluded native window chrome. Not compared,
  and marked so it cannot be mistaken for "identical".
- **magenta** — the two frames are different sizes; the magenta area is the
  non-overlapping part. Common across machines, where borders and DPI change the
  captured rect.
- **light grey** — identical.

`deltaPercent` is over *compared* pixels, at a per-channel tolerance of 8, which
absorbs text antialiasing shifting by a unit or two. Anything a human would point
at is far above that.

On one machine, with the pointer pinned, repeated runs are byte-identical and the
committed baselines report 0.00%. **Across machines they will not be.** Output
varies with GPU driver, Windows build, DPI scale and font fallback. That
unreliability is the whole reason this is advisory — read the diff, do not gate on
the number.

## Regenerating the baselines

A deliberate human action, never automated. There is no `--update-baselines` flag
on purpose: a tool that refreshes its own reference removes the diff you were
trying to read. See `artifacts/visual/baseline/README.md` for the procedure. The
short version: capture, **look at the new frames and decide whether the change was
intended**, then copy them across in the same commit as the change.

## Known limitations

Read these before trusting a delta.

- **No coverage for views without a scenario.** Settings, crop mode, slideshow,
  drag-and-drop, and the top bar at narrow widths are not captured.
- **No interaction states.** Everything is reached by CLI argument, so anything
  that needs a real click, key or drag is out of reach.
- **No theme coverage.** The hermetic profile bootstraps the default theme, so a
  theme change is invisible here by design.
- **Nothing in motion.** The stability gate rejects frames that are still changing,
  so anything animated is either captured settled or not captured at all.
- **Activation-dependent chrome is excluded, not fixed.** Forcing the window to the
  foreground was tried and rejected; Windows' foreground lock refuses a background
  process. The band is excluded instead.
- **Blank frames happen on headless runners.** If `summary.json` reports every
  scenario blank, nothing is uploaded, because a black PNG is worse than no PNG.
- **A human is the gate.** The tool reports that pixels differ; it cannot say
  whether that is a bug.

## CI

`.github/workflows/visual-baseline.yml` builds release, runs this script on
`windows-latest` and uploads the frames as a workflow artifact. It does **not** run
`-Compare`: a runner shares no GPU, driver, DPI or font set with the machine that
produced the baselines, so the comparison would report a large delta on every run
and teach everyone to ignore it. It is not a required status check and must never
be added to the required list for `main`.
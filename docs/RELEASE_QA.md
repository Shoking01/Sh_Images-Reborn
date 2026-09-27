# Release QA

This document defines the manual release evidence for the Sh_Images UI. Use it
with the deterministic structural baseline in `crates/sh-app/src/app.rs`.

## Evidence boundary

GPUI 0.2.2's test window exposes element bounds, selectors, and render state,
but it does not capture actual rendered pixels. Therefore:

- The `layout_baseline` tests are deterministic structural/render-state checks.
- Screenshots in this checklist are **manual evidence**, not automated pixel
  goldens.
- Do not claim pixel-perfect coverage from the test harness. Record visual
  observations, platform details, and capture names instead.
- Screenshots are optional when the operator cannot provide them. In that
  case, record an explicit `EXCEPTION — screenshots unavailable` entry for
  the affected IDs and preserve the available data-only observations. This is
  not equivalent to claiming visual evidence.

Themes control colors, surfaces, and styling; they do not enable animation.
The `reduce_motion=true` default bypasses animation, while `false` enables the
selected 150ms hover transitions. Not perceiving an animation is therefore
compatible with a working application and is recorded as `NOT OBSERVED`, not
as a functional failure. Slideshow navigation is timer-driven and does not
depend on the theme or the motion helper.

## Release evidence record

Use one record per release candidate. `PENDING` is intentional until the
corresponding check or manual capture is actually run; automated structural
results must not be used to fill manual screenshot rows.

| Field | Value |
| --- | --- |
| Candidate | `perf/grid-virtualization` / `ee103cf` |
| Date / operator | `2026-09-23` / operator report for `a77e5e3`; latest data-only acceptance for `ee103cf` |
| Platform | `PENDING` — Windows reported; GPU/renderer, display scale, and window size not recorded |
| Settings / fixtures | `PENDING` — general behavior reported; theme, language, values, and fixture folder not recorded |

### Automated evidence

| Check | Command | Result | Evidence / date |
| --- | --- | --- | --- |
| Workspace tests | `cargo test --workspace` | `PASS` — 283 `sh-app`, 184 `sh-core`, 3 integration; 0 failed | `2026-09-23` local run |
| Format | `cargo fmt --all --check` | `PASS` — exit 0, no output | `2026-09-23` local run |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `PASS` — exit 0; Cargo future-incompat warning for `proc-macro-error2 v2.0.1` | `2026-09-23` local run |
| Release build owner | `cargo build --release -p sh-app` | `PASS` — exit 0, latest cached run 0.91s; `target/release/sh-app.exe` (10,412,032 bytes) | `2026-09-23` local run |
| CI and documentation review | YAML syntax plus Markdown links/claims | `PASS` — manual review; local `actionlint`/YAML parser unavailable | `2026-09-23` local run |

> The release build completed successfully, but the cold local run took
> 5m21s, slightly above the AGENTS.md five-minute release-build target. Record
> the hosted Windows CI timing before final release sign-off.

### Manual evidence

| Evidence | Scope | Result |
| --- | --- | --- |
| Screenshots | IDs W-01 through M-01 below, or an explicit exception per ID | `EXCEPTION` — operator cannot provide screenshots; applies to W-01 through M-01; no screenshot is claimed |
| Interaction matrix | Filmstrip, checkerboard, interval, reduced motion, keyboard Settings, slideshow transitions | `OPERATOR ACCEPTANCE` — latest report confirms the current persistent Back, lower-overlay coexistence, and repeated arrow behavior work perfectly; detailed row-by-row visual evidence remains unavailable |
| Performance and memory | Frame-time and memory observations before/after motion activation | `OPERATOR REPORT` — normal use stayed at or below 90 MB; rapid movement of many images may raise usage toward 250–300 MB; slideshow did not increase memory; CPU use was negligible |
| Release launch | Launch the produced Windows binary and record the result | `OPERATOR REPORT` — application functioning confirmed in the Windows environment; no separate launch log attached |

### Operator data-only record

The following is the release evidence supplied by the operator. It is kept
separate from automated structural results and is not presented as screenshot
or pixel-golden evidence.

| Observation | Recorded result |
| --- | --- |
| General behavior | Application reported functioning correctly at the overall level |
| Latest Viewer interaction acceptance | Operator confirmed that persistent Back, lower-overlay coexistence, and repeated arrow clicks work perfectly on `ee103cf` |
| Memory | At or below 90 MB during normal use; rapid movement of many images may raise usage toward 250–300 MB |
| Slideshow | No observable increase in memory consumption during slideshow |
| CPU | Reported as negligible |
| Themes | Tested themes reported working; no theme-specific failure reported |
| Motion | No animation perceived on the baseline `a77e5e3`; compatible with the default `reduce_motion=true` |
| Screenshots | Unavailable; explicit data-only exception applies to the affected IDs |

If a release gate requires visual proof of animation, rerun only that check
with `reduce_motion=false` and record whether a transition is perceived. The
absence of a visible transition is not, by itself, a defect.

### Post-QA code changes

The operator data-only report was collected against `a77e5e3`. Three
subsequent commits change Viewer behavior:

- `be17fcf` added the initial bottom-chrome Back control.
- `16de7ce` expanded stronger reduced-motion-aware hover treatment across
  interactive Viewer controls while keeping the 150 ms duration.
- `ee103cf` keeps Back persistent in the viewer main area and refreshes the
  idle clock on direct Previous/Next and filmstrip navigation.

Automated tests and the release build pass for `ee103cf`. The operator has
now confirmed the newly changed persistent Back target, lower-overlay
coexistence, and repeated arrow-click behavior work perfectly. This is accepted
as data-only evidence for the current candidate; it does not replace the
explicit screenshot exception or provide pixel-level proof.

## Installer and packaging verification

> **The rule: assert the installed artifact, never the process exit code
> alone.** A successful install does not imply a correct install. The installer
> script `installer/sh-images.iss` compiled cleanly, installed cleanly, and
> reported success while every shortcut it created pointed at an icon path
> that does not exist on the target machine. Only reading the installed
> artifact back revealed it.

### Reading a created shortcut's real `IconLocation`

`[Icons] IconFilename` is a **runtime** path. Inno Setup writes it verbatim into
the `.lnk` and never validates it — not at compile time, and not at install
time. A repository-relative value such as `..\assets\branding\sh-images.ico`
is accepted without complaint and resolves to nothing once the setup is running
from a temp extraction directory. Always confirm the icon file exists at the
path the shortcut actually stores, and that it is the intended asset.

```powershell
# Install silently into a throwaway directory, then read the shortcut back.
$setup = (Resolve-Path 'target\installer\ShImages-Setup-0.1.0-win-x64.exe').Path
$dir   = Join-Path $env:TEMP 'sh-images-installer-test'
# The quotes inside "/DIR=..." are required: Start-Process -ArgumentList joins
# the array with spaces and adds none, so an unquoted path containing a space
# arrives at the installer as several separate tokens and it creates the wrong
# directory. %TEMP% can contain a space when the username does.
$p = Start-Process -FilePath $setup `
  -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=`"$dir`"" `
  -Wait -PassThru
"install exit code: $($p.ExitCode)"

$sh   = New-Object -ComObject WScript.Shell
$lnk  = $sh.CreateShortcut(
  (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Sh Images\Sh Images.lnk'))
"TargetPath   : $($lnk.TargetPath)"
"IconLocation : $($lnk.IconLocation)"

# Strip the trailing ",<index>" that IconLocation carries, then assert the file
# is real and is the branding asset rather than some unrelated icon.
$icon = ($lnk.IconLocation -replace ',\s*-?\d+$', '').Trim()
"is absolute?    : $([IO.Path]::IsPathRooted($icon))"
"icon exists?    : $(Test-Path -LiteralPath $icon)"
"branding match? : $((Get-FileHash -LiteralPath $icon -Algorithm SHA256).Hash -eq
                     (Get-FileHash 'assets\branding\sh-images.ico' -Algorithm SHA256).Hash)"
```

The final line is the assertion that matters. A shortcut can store a path that
is perfectly well-formed, absolute, and still not be the icon you intended;
comparing the hash against `assets/branding/sh-images.ico` closes that gap.

> **Do not trust `System.Drawing.Icon.ExtractAssociatedIcon` or
> `SHGetFileInfo(SHGFI_ICON)` to report which icon a `.lnk` shows.** Both were
> measured against a known-good control and neither resolved a shortcut's
> `IconLocation`. Calibrate any icon probe against a shortcut pointing at a
> known file before believing its output; otherwise assert on the stored path,
> not on rendered pixels.

### Inno Setup gotchas

| Gotcha | Observable symptom | Correct fix |
| --- | --- | --- |
| `[Icons] IconFilename` is a runtime path and is never validated | Script compiles, install exits 0, Setup log says "Successfully created the icon" with no warning, but the `.lnk` stores a path that does not exist | Install the icon with `[Files]` and reference `{app}\<name>.ico`; verify the stored `IconLocation` is absolute and exists |
| `[Run]` executes programs; it cannot create shortcuts | A `[Run]` entry labelled "Create a desktop shortcut" creates nothing and instead launches the app a second time under a misleading label | Create the shortcut only in `[Icons]`, gated by its task and `Check: not WizardSilent` |
| `unins000.exe` re-launches itself from a temp copy and can return first | Uninstall exits 0 but the install directory is still present on the next line | Poll for removal with a deadline instead of checking once |
| A `[Files] Source:` payload that does not exist is a hard compile error | `ISCC` exits 2 with `Source file ... does not exist` | For script-only validation, create an explicit stub at the expected path; a local compile with no release build fails this way, which is expected and not a defect |
| A silent uninstall leaves a temporary directory behind in the system temp directory | After `/VERYSILENT` uninstall, `%TEMP%` still contains `is-<random>-uninstall.tmp\` holding a copy of the uninstaller (about 4.3 MB) plus `_unins-done.tmp`, and it does not self-clean | Expected Inno Setup engine behavior that no `.iss` directive controls; the release smoke test measures, logs and removes it rather than asserting on it, and it is not user-visible on a normal interactive uninstall |

### Installer verification checklist

- [ ] Payload present: the executable and `LICENSE` exist in the install
      directory.
- [ ] Uninstaller present: `unins000.exe` exists, so the app is removable.
- [ ] Shortcut `IconLocation` is absolute and the file it names exists.
- [ ] Branding identity: the installed icon's SHA-256 matches
      `assets/branding/sh-images.ico`.
- [ ] User data unchanged: hash every file under `%APPDATA%\sh_images` before
      and after, and require zero differences.
- [ ] Reinstall-over works: running the installer again over the same directory
      yields one install directory and one Add/Remove Programs entry.
- [ ] No leftover install directory after uninstall (polled, not checked once).
- [ ] No leftover Add/Remove Programs entry for the `AppId`.
- [ ] No leftover Start Menu or desktop shortcut.
- [ ] Uninstaller temp residue under `%TEMP%`: any `is-*-uninstall.tmp` left by the
      silent uninstall is reported and removed by the smoke test, not asserted on.

Concretely, the user-data check:

```powershell
function Get-UserDataFingerprint {
  Get-ChildItem -Recurse -Force -File -LiteralPath (Join-Path $env:APPDATA 'sh_images') |
    Sort-Object FullName |
    ForEach-Object { "{0}  {1}" -f $_.FullName, (Get-FileHash $_.FullName -Algorithm SHA256).Hash }
}
$before = Get-UserDataFingerprint
# ... install, reinstall over the same directory, then uninstall ...
$after = Get-UserDataFingerprint
$differences = Compare-Object $before $after
if ($differences) { throw "Installer modified user data: $($differences | Out-String)" }
"user data untouched"
```

### Provenance of the results above

The behaviors in this section were observed while building and fixing
`installer/sh-images.iss` on branch `installer/inno-setup`. Read them with
these limits:

- The local compile used a **stub payload** (`target\release\sh-app.exe`,
  a 44-byte placeholder), because `cargo build --release` was deliberately not
  run locally. Script directives, install, shortcut, icon, and uninstall
  behavior were verified; the **real** executable was not.
- The first end-to-end proof with a genuine release binary is the
  silent-install smoke test in `.github/workflows/release.yml`. That workflow no
  longer runs on every merge; it is `workflow_dispatch` only, so the proof has
  to be asked for. The permanent release path is a tag-triggered publish
  workflow that runs the same build, packaging and smoke test on a version
  tag. Until such a run passes, this section documents verified *installer
  mechanics*, not a verified shipped artifact.
- The packaging build is no longer a per-merge canary, so a broken installer
  surfaces at release time rather than at merge time.
- The `IconLocation` values were read from shortcuts produced by local silent
  installs. **No interactive install was performed and no screenshot evidence
  was captured.** Whether the icon renders as expected in Explorer has not been
  confirmed visually; only the stored path and file identity were asserted.
- The desktop shortcut path is behind a `Check: not WizardSilent` guard, so it
  cannot be produced by a silent install. Verifying it requires either an
  interactive install or a throwaway script compiled with a distinct `AppId`
  and the guard relaxed.

## Capture setup

Record these details with every evidence set:

- Commit or release identifier.
- OS, GPU/renderer, display scale, and window size.
- Theme, language, and relevant Settings values.
- Window state: Welcome, Grid, Viewer, or Settings; selected section when
  applicable.
- Screenshot naming pattern:
  `YYYYMMDD-<surface>-<state>-<theme>-<motion>.png`.

Use a folder containing both an opaque image and a transparent image, plus an
empty folder. Keep the same fixture set for comparisons where possible.

## Screenshot checklist

| ID | Surface and state | Capture / check | Expected evidence |
| --- | --- | --- | --- |
| W-01 | Welcome, no recent folders | Capture the hero, drop zone, and `Open folder…` action. | No Continue action; drop zone is visible and balanced. |
| W-02 | Welcome, recent folders | Capture with two or more recent folders. | Continue and recent-folder chips are visible; labels are readable. |
| G-01 | Grid, empty folder | Capture the empty-state message and top bar. | No thumbnail cells; empty message is centered and unobscured. |
| G-02 | Grid, populated | Capture S, M, and L density presets with a mixed folder. | Cells, labels, top bar, and selection/cursor markers are aligned. |
| G-03 | Grid, transparency | Capture an opaque and a transparent thumbnail. | The transparency board is present only for the transparent thumbnail when enabled. |
| V-01 | Viewer, opaque image | Capture fit mode and one zoomed state. | Image framing, top bar, filename, and position are readable. |
| V-02 | Viewer, transparent image | Capture transparency board enabled and disabled. | Board follows the setting; disabling it does not change image framing. |
| V-03 | Viewer filmstrip | Capture the strip enabled and disabled, including narrow and long folders. | The strip docks below the image, keeps its fixed height, and the current cell is centered. |
| V-04 | Viewer overlays | Capture active bottom chrome, idle-hidden chrome, info popover, crop selection, and crop confirm bar. | Controls are reachable in their supported states; hidden chrome is not painted or interactive. |
| V-05 | Slideshow | Capture active and paused Viewer states; record interval observations separately. | Play/pause state is clear and navigation remains stable. Timing is a manual observation. |
| S-01 | Settings, all sections | Capture General, Appearance, and Shortcuts at the minimum supported window (480×320). | Header, sidebar, content, and all section controls remain reachable. |
| S-02 | Settings scroll | Capture the top and bottom of each section at minimum size. | No section is stranded; the last row and reset/control rows can be reached. |
| S-03 | Settings controls | Capture filmstrip, checkerboard, slideshow interval, and reduced-motion rows. | Checkmarks/value controls reflect persisted state; changing a row does not resize the surface unexpectedly. |
| T-01 | Themes | Capture Welcome, Grid, Viewer, and Settings with each built-in theme; include a light theme. | Text, surfaces, accents, borders, and focus/hover states remain legible. Invalid theme edits retain the previous valid theme. |
| M-01 | Reduced motion | Capture the same hover/control state with reduced motion on and off. | Layout is identical; reduced motion is instant, while motion-enabled hover is restrained. Timing requires observation, not screenshot comparison. |

## Manual sign-off

For each capture or explicit data-only exception, record:

- [x] Screenshot name and surface/state match this checklist, or the affected
      IDs are covered by the screenshot-unavailable exception.
- [ ] Theme, language, Settings values, viewport, and platform are recorded.
- [ ] The expected control is visible, hidden, or scroll-reachable as described.
- [ ] No clipping, overlap, unreadable text, or unintended layout movement is
      observed.
- [x] Timing, GPU, font, platform, and motion differences are recorded as
      operator observations rather than silently normalized.

A release evidence set is complete when the required surfaces and states above
have captures or an explicit documented exception. Automated structural test
results are recorded separately from these manual observations.

## Candidate sign-off

- [x] Automated commands in the release evidence record have exact results.
- [x] The final `cargo build --release -p sh-app` result and binary path are recorded.
- [x] Manual screenshots or explicit per-ID exceptions are attached.
- [x] Frame-time, memory, and launch observations are recorded or explicitly deferred.
- [x] Theme Editor and pixel-golden coverage are not reported as shipped evidence.

**Sign-off status:** `CONDITIONAL DATA-ONLY` — automated gates and the latest
operator acceptance pass for `ee103cf`; screenshots, detailed platform values,
and hosted Windows CI timing remain explicitly pending.

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
| Date / operator | `2026-09-23` / operator report for `a77e5e3`; automated verification for `ee103cf` |
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
| Interaction matrix | Filmstrip, checkerboard, interval, reduced motion, keyboard Settings, slideshow transitions | `BASELINE REPORT` — operator reported general behavior as working on `a77e5e3`; post-QA Back/motion changes need a targeted recheck |
| Performance and memory | Frame-time and memory observations before/after motion activation | `OPERATOR REPORT` — normal use stayed at or below 90 MB; rapid movement of many images may raise usage toward 250–300 MB; slideshow did not increase memory; CPU use was negligible |
| Release launch | Launch the produced Windows binary and record the result | `OPERATOR REPORT` — application functioning confirmed in the Windows environment; no separate launch log attached |

### Operator data-only record

The following is the release evidence supplied by the operator. It is kept
separate from automated structural results and is not presented as screenshot
or pixel-golden evidence.

| Observation | Recorded result |
| --- | --- |
| General behavior | Application reported functioning correctly at the overall level |
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

Automated tests and the release build pass for `ee103cf`. The prior operator
report remains valid as a general baseline, but it does not prove the newly
changed persistent Back target, arrow-click visibility, or pointer-driven
hover feedback. A targeted data-only recheck of those behaviors remains
pending.

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

**Sign-off status:** `CONDITIONAL DATA-ONLY` — automated gates pass for
`ee103cf`; the operator report is a baseline for `a77e5e3`; screenshots,
detailed platform values, hosted Windows CI timing, and the targeted Back/motion
recheck remain explicitly pending.

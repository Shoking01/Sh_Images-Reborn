# Fix plan (PROPOSED → IMPLEMENTED 2026-09-20): viewer round 2

Status: IMPLEMENTED on branch `fix/viewer-round2` (from `fix/viewer-trio`), one work-unit commit per fix,
RED/GREEN + gates per unit. User instruction 2026-09-20: "Abre una rama nueva desde fix/viewer-trio y vamos
a implementar los fixes de este archivo".
Commits: R1 `faaaceb`, R2 `8e82edb`, R3 `fddb6b5`.
Gates: `cargo test -p sh-core` 156 passed; `cargo test -p sh-app` 203 passed; `cargo clippy --all-targets
-- -D warnings` clean; `cargo fmt --check` clean; `cargo build --release` ok (3m34s).
Implementation deviations from the plan (all conservative): see the per-fix "IMPLEMENTED" notes below.

## R1 — Remove the 50% chip — IMPLEMENTED (faaaceb)

IMPLEMENTED as planned. Anti-drift count: 64 → 63 (`ALL_KEYS.len()`), changelog key counts updated
(63 / 58). `zoom_preset_segments` now returns exactly `[Fit, Scale100, Scale200]` (asserted);
sub-floor-snap test re-pointed at 100% with a 2160×2160 image in a 2160×2160 viewport.

Root cause: the 4-chip row is hardcoded (`app.rs:2006-2011`, `zoom_preset_segments` Fit/50/100/200) fed by
`ZoomPreset::Scale50` (`state/session.rs:38`, `scale()` → `Some(0.5)` at `:50`) with label
`StrKey::ZoomPreset50` (`sh-core/src/i18n.rs:301,378,447,522`). Dependents: active-match rule
(`app.rs:2016`), sub-floor snap test (`session.rs:883-899`).

Fix steps:
1. `state/session.rs:34-55` — delete `Scale50` variant + its `scale()` arm.
2. `app.rs:1978-2023` — remove `Scale50` from `zoom_preset_label` and `zoom_preset_segments` + active-match arm; fix doc comment to Fit/100/200.
3. `sh-core/src/i18n.rs` — delete `ZoomPreset50` from `StrKey`, `en()`/`es()` tables, and the 3 test lists (`:649-720`).
4. `CHANGELOG.md:47,50,59` — rewrite as three chips; fix the Unreleased disabled-chip example (`:11-12`, "e.g. 50%" becomes invalid); fix key counts.
5. `odd/tasks/bugfix-viewer-trio.md` — tracking only, update or leave.

Tests: `zoom_preset_scale_yields_the_exact_four_variants` (`session.rs:798` → three variants);
`set_zoom_preset_lands_above_floor_scales_exactly` (`:817`, drop 0.5 case); `zoom_preset_label_resolves…`,
`zoom_preset_segments_mark_only_the_active_chip`, `zoom_preset_disabled_*` (`app.rs:5866-5998`, drop Scale50
asserts); delete or re-point `set_zoom_preset_sub_floor_snaps_back_to_fit` at 100% with a tiny image; add
assertion that segments return exactly `[Fit, Scale100, Scale200]`.
Risks: anti-drift key-count tests (59/64) fail until tables + counts update together. Size: ~30 lines, 4 files.

## R2 — Info button buried by options/crop buttons on Tab — IMPLEMENTED (8e82edb)

IMPLEMENTED with one deviation: the headless harness cannot assert on the element tree, so the
row-membership contract is pinned by a pure predicate `info_button_in_chips_row(topbar_dissolved)`
(same B3 `info_button_visible` pattern) instead of scraping `#viewer-chips` children. The standalone
float wrap stays in the tree (Tab OFF); Tab ON parents the button as the third chips-row slot via
`Option::take()` — exactly one container per frame, compiler-checked.

Root cause: two absolute top-right layers share one corner. B3 floats the info button at
`app.rs:2348-2358` (`#info-btn-float`: `absolute, top(12), right(12)`). Tab-ON mounts `#viewer-chips`
(`app.rs:3489-3508`, `absolute, top(10), left(12), right(12)`) whose right cluster (`chip-settings` +
`chip-crop`, `app.rs:3440-3487`) lands on the same coordinates. Paint order in `#viewer-area`
(`app.rs:3601-3604`) puts chips after the info button, so gear/crop cover it. The popover itself
(`overlay.rs:195-205`) is fine — only the button is buried.

Fix steps (reserved corner slots, single row):
1. `app.rs:3489-3508` — make the right cluster a 3-slot row: gear + crop + info button (one absolute root).
2. `app.rs:2348-2361` — remove the standalone float wrap; pass `info_btn` as third child of the right cluster
   when dissolved, keep float position when Tab-OFF.
3. `app.rs:3601-3602` — delete the duplicate `.children(floating_info_el)` line.

Tests: headless test Tab-ON + image → single `#viewer-chips` row holds settings + crop + info-btn, no
`#info-btn-float`; Tab-OFF float still present (`info_button_visible`, `app.rs:4227` stays green). Manual: Tab
toggle with popover open, both languages. Rejected alternative: static down-offset (re-collides on narrow
windows). Risks: `info-btn` id moves in tree — update id-scraping tests. Size: ~25 lines, `app.rs` only.

## R3 — Image jolts in windowed mode on Tab toggle — IMPLEMENTED (fddb6b5)

IMPLEMENTED with two deviations beyond the plan's step list (both required to actually kill the jolt):
1. Removing only the refit was NOT enough — the topbar is an in-flow flex child, so hiding it still
   changes viewer-area's layout height by 40px per toggle. The topbar now FLOATS in the Viewer
   (`#topbar-float`, absolute, attached after viewer-area at the root; Grid keeps the in-flow bar), so
   viewer-area owns the full window height in every Tab state and paint order is unchanged relative to
   the chips/popover.
2. `viewer_fit_height` + `topbar_was_hidden` are deleted; `viewer_viewport()` IS
   `stable_open_viewport` now (chrome-independent contract documented on the method). Crop drag coords
   dropped their manual `- TOPBAR_H_PX` y-shift (no layout offset exists anymore). The Tab-OFF
   info-button float parks at `TOPBAR_H_PX + 12` so the solid floating bar cannot bury it (new R2×R3
   collision the plan could not foresee).
Tests: `viewer_viewport_subtracts_topbar` rewritten as
`viewer_viewport_is_full_window_in_every_chrome_state`; new `viewer_viewport_is_tab_independent_headless`
(dims on the current image so a regression to refit would fail on zoom/offset, not vacuously).
Manual QA still pending (windowed 800×600 Fit, Tab ×5 → pixel-static; Tab toggle with popover open,
both languages) — needs a human at the GPU window.

Root cause: fit viewport is chrome-dependent. `viewer_viewport()` (`app.rs:493-501`) returns full height when
dissolved, minus 40px otherwise (`viewer_fit_height`, `app.rs:1853-1860`). Tab flips `topbar_dissolved`
(`app.rs:2152-2156`) + immediate `refit_for_viewport` (`session.rs:194-202`) with centering fit
(`transform.rs:41-49`). Every Tab press changes fit height by 40px and re-centers: visible jolt. Pinned by
`viewer_fit_height_tracks_bar_visibility` (`app.rs:4357`) and `viewer_viewport_subtracts_topbar` (`app.rs:5565`).

Fix steps (float chrome, stable viewport — no refit):
1. `app.rs:493-501` — `viewer_viewport()` always returns full window size (keep `.max(1.0)` floor).
2. `app.rs:2152-2156` — delete the dissolve-flip `refit_for_viewport` block (keep resize refit `:2121-2125`).
3. `app.rs:1849-1860` — drop `viewer_fit_height` if no other caller needs it; unify `stable_open_viewport`
   (`app.rs:2062`) with the new `viewer_viewport` or document.

Tests: update the two pinning tests to Tab-independent viewport; add `refit_not_called_on_tab_toggle`
(zoom.scale/offset bit-identical across toggle); re-run `refit_for_viewport_*` (`session.rs:589+`),
preset-centering, `toggle_fit_100` (explicit viewports — unaffected). Manual: windowed 800×600 Fit, Tab ×5 →
pixel-static. Tradeoff (deliberate UX change): chrome overlays image edges instead of reserving space.
Rejected alternative: anchor-preserving refit (keeps 40px scale jump, more code). Size: ~15 lines `app.rs` +
2 tests updated, 1 added.

## Record
- Analysis: read-only general subagent, 2026-09-20. No code, commits, or test runs for this plan.
- Implementation: Buffy writer, 2026-09-20, branch `fix/viewer-round2` — R1 `faaaceb`, R2 `8e82edb`,
  R3 `fddb6b5`. Gates green per commit (see status header). No PR pushed (per instructions).
- Engram mirror: PENDING (runtime session ambiguity blocks mem_save; record lives here).
- Remaining: manual QA (Tab toggle pixel-static + popover both languages) and user review of the R3
  UX tradeoff (solid topbar covers the image's top 40px with Tab OFF).

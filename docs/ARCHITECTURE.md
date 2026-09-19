# Sh_Images — Architecture Decision Records

> Decisions are recorded per AGENTS.md §9.2 (Context / Decision /
> Consequences / Alternatives considered). Each ADR reflects the architecture
> **as built** on branch `feat/v1-core`; references to specific behaviors
> point at the code that implements them.

---

## ADR-001: Cargo workspace with sh-core / sh-app separation

- **Status:** Accepted

- **Context:** AGENTS.md requires core business logic to never import GPUI.
  Convention alone drifts over time; a module boundary inside one crate
  enforces nothing.

- **Decision:** Two crates in one workspace. `sh-core` (pure logic:
  navigation, decode, cache, theme, transform, settings) declares **no
  `gpui` dependency at all** and is `#![forbid(unsafe_code)]` — the compiler,
  not review, enforces the layering. `sh-app` holds all GPUI code (views,
  session state wiring, platform glue).

- **Consequences:** Slightly slower first full builds (two crates); a hard
  guarantee that core logic is headless-testable and portable; core tests
  run with zero GPU/window infrastructure. Every capability in `sh-core`
  remains usable by future frontends (e.g., a CLI batch converter).

- **Alternatives considered:** single binary with `core/` and `ui/` module
  directories (convention only — nothing prevents a stray `use gpui`); a GPUI
  fork as a subcrate (heavy, unbounded maintenance).

---

## ADR-002: V1 scope — solid single-image core first

- **Status:** Accepted

- **Context:** The project restarted from zero in Rust after the previous
  GPUIX/React/Bun implementation was discarded. Delivering the full AGENTS.md
  surface (grid, crop, export) as a first milestone carried large risk on an
  unfamiliar, pre-1.0 UI framework.

- **Decision:** V1 ships a single-image viewer with circular folder
  navigation, zoom/pan/fit, JSON themes with hot reload, ephemeral overlays,
  fullscreen, drag&drop / Ctrl+O / CLI-arg open, and settings persistence.
  Grid, crop, and export are deferred.

- **Consequences:** Smaller first milestone; the riskiest integrations
  (GPUI 0.2.2 rendering semantics, input dispatch, entity leases, Windows
  platform quirks) were proven on a minimal surface that all later features
  build on.

- **Alternatives considered:** full AGENTS.md feature set in V1 (high risk of
  a half-finished everything); grid-first (doubles the decode/render surface
  before the single-image path was trustworthy).

---

## ADR-003: GPUI 0.2.2 pinned from crates.io

- **Status:** Accepted

- **Context:** GPUI is pre-1.0 with breaking API changes between versions;
  Zed's `main` examples routinely use APIs that do not exist in released
  crates.

- **Decision:** Pin `gpui = "0.2.2"` from crates.io. Every non-trivial API
  assumption (mouse dispatch semantics, `Display::None` behavior, entity
  lease rules, `img()` caching) was verified against the crate source, not
  against Zed main.

- **Consequences:** Stable, reproducible builds; API deltas are handled
  deliberately at upgrade time, not discovered mid-task. Bugs found during
  V1 (e.g., opacity being paint-only) are documented in-code with file/line
  references to the 0.2.2 source.

- **Alternatives considered:** git dependency on Zed main (unstable, massive
  compile times, no version discipline); vendoring GPUI as a fork (permanent
  maintenance burden).

---

## ADR-004: Zoom/pan via explicit element sizing, not transforms

- **Status:** Accepted

- **Context:** GPUI 0.2.2 exposes no `div().transform()` — transformation is
  only implemented for SVG/paths. `object_fit` was also unsuitable: it maps a
  fixed-size source into a box, while zoom needs the element to *be* the
  zoom math's output rectangle.

- **Decision:** The viewer maps `sh_core::transform` state directly onto
  layout: the `img()` element is positioned absolutely at the pan offset
  (`left`/`top`) and sized to `dimensions × scale` (`w`/`h`). The element's
  aspect ratio always equals the image's, so no fit logic is needed inside
  the element. The GPU rescales the texture at paint time.

- **Consequences:** Zoom/pan is fully deterministic from session state; no
  hidden layout machinery. Deep zoom on huge images costs a per-frame
  texture rescale, which is acceptable for V1's single-image scope. The
  one-frame-before-probe case falls back to 1×1 px (a header probe is
  near-instant).

- **Alternatives considered:** a canvas element with manual texture draws
  (more complex, premature for V1); `object_fit`-based fitting (cannot
  express zoom at all).

---

## ADR-005: Rendering via GPUI `img()` + header-only dimension probes

- **Status:** Accepted (supersedes the original plan's DecodeImageAsset /
  Asset-pipeline design)

- **Context:** The plan proposed a custom `Asset` implementation with
  background-executor loads. Two problems emerged during implementation:
  GPUI's built-in `img()` element already handles file loading, decoding,
  and BGRA conversion internally with a path-keyed cache, and an earlier
  design that kept decoded RGBA in session state caused unbounded RAM
  growth (one full decode per visited image).

- **Decision:** Split responsibilities. **Rendering** uses GPUI's built-in
  `img(PathBuf)` — content-based decode plus its internal path-keyed cache,
  zero custom decode code on the UI side. **Session state** stores
  dimensions only: `sh_core::decode::probe_dimensions` reads the header
  (no pixel decode) on a background executor to drive fit/zoom math. The
  session never holds pixel data.

- **Consequences:** RAM stays flat regardless of how many images are
  visited (headers are bytes, decodes are GPUI-internal and path-keyed).
  `sh_core::decode::load` and `DecodeCache` (LRU with byte budget) remain
  library capabilities — exercised by tests and benchmarks, not wired into
  the app's render path.

- **Alternatives considered:** the plan's Asset/`use_asset` pipeline (custom
  decode duplicated GPUI's `img()` behavior and kept full RGBA in app
  state); storing `DecodedImage` in the session (the RAM bug this ADR
  fixed).

---

## ADR-006: Navigation stale-completion safety via a monotonic sequence guard

- **Status:** Accepted

- **Context:** `navigate()` spawns async dimension probes (current image)
  and prefetches (next image) that complete at arbitrary times.
  `open_path()` swaps the entire image list, so a completion's captured
  index can point past the new list (out-of-bounds panic) or poison a slot
  with another image's dimensions.

- **Decision:** A monotonic `navigation_seq` counter. Every `navigate()` and
  every list swap bump it; each spawned completion captures the seq at spawn
  time and commits **nothing** — no global state, no `dimensions` slot
  write — unless it still matches the live counter. Stale completions are
  dropped entirely; re-navigation re-probes (a header read is
  near-instant).

- **Consequences:** All session mutations from async completions are safe
  against rapid navigation and open-path swaps by construction. The cost is
  a dropped prefetch when the user outruns it — never a wrong result.

- **Alternatives considered:** per-task cancellation handles (gpui 0.2.2
  task abort semantics were less direct than a compare-and-skip guard);
  bounds-checking stale writes (prevents the panic but still poisons slots
  with wrong data — rejected for correctness, not just safety).

---

## ADR-007: File dialogs must be async (rfd on a dedicated thread)

- **Status:** Accepted

- **Context:** `Ctrl+O` opens a native file dialog. A synchronous modal
  dialog inside a `cx.listener` pumps the Win32 message loop while the App
  entity is still leased — gpui's redraw ticks then re-lease the entity and
  hit `double_lease_panic`. This was proven during Task 10, not theorized.

- **Decision:** Use `rfd::AsyncFileDialog` driven through `cx.spawn`:
  the entity lease is released before `pick_file()` blocks, and rfd runs the
  native dialog on its own dedicated thread.

- **Consequences:** The UI keeps rendering while the dialog is open (the
  window does not freeze); no lease conflicts. The open action lands
  through a normal async entity update when a file is chosen.

- **Alternatives considered:** a synchronous `rfd::FileDialog` (provably
  panics — see context); implementing a custom in-app file picker (out of
  V1 scope, worse native fidelity).

---

## ADR-008: Overlay visibility via `Display::None`, not opacity

- **Status:** Accepted

- **Context:** Ephemeral overlays (top: filename/position; bottom:
  zoom/arrows) must hide after ~1.5 s of idle and on Tab. GPUI 0.2.2's
  `.opacity(0.0)` is paint-only: an invisible overlay keeps its hitboxes
  and stays clickable.

- **Decision:** Gate visibility with `Display::None` (`.hidden()`), which
  skips child prepaint/paint entirely — hidden overlays get no hitboxes.
  The element stays in the tree, keeping IDs stable across toggles. Idle
  auto-hide runs from a background timer that flips a latch and notifies
  exactly once per visible→hidden transition (any interaction resets it),
  avoiding a perpetual notify loop.

- **Consequences:** Hidden overlays are genuinely non-interactive;
  keyboard-only users never lose the chrome because every interaction
  surface refreshes the idle clock. A future fade animation can layer
  `.opacity()` *on top of* the gate — opacity alone would regress
  interactivity.

- **Alternatives considered:** opacity-only hiding (breaks hit-testing);
  removing overlays from the tree when hidden (unstable element IDs, more
  churn); a render-side `cx.notify()` timer loop (wasteful repaints).

## ADR-009: Embedded SVG icon system via AssetSource

- **Status:** Accepted

- **Context:** GPUI 0.2.2's `svg().path()` resolves asset path strings
  through the `AssetSource` trait, not raw SVG path data. The app needed
  line icons that inherit theme color without an icon-font dependency.

- **Decision:** Lucide-derived SVG files under `crates/sh-app/assets/icons/`,
  embedded via `include_bytes!` in a custom `AppAssets` AssetSource
  registered at boot (`Application::new().with_assets`). Icons recolor via
  `.text_color()` because GPUI paints svg strokes with the element's text
  color. The 10-icon registry is enum-locked and unit-tested (enum ↔ asset
  key 1:1, paths unique, every key loads bytes).

- **Consequences:** Adding an icon = SVG file + enum variant + match arm +
  registry entry. Per-theme colors are free (same asset, different
  `text_color`). Compile-time embedding — no disk access, no missing-file
  risk at runtime.

- **Alternatives considered:** icon font (new dependency + license overhead +
  glyph metrics fiddliness), rasterized PNGs (blurry at fractional DPI, one
  file per color per theme).

## ADR-010: Hybrid dissolving topbar (Viewer only)

- **Status:** Accepted

- **Context:** The solid topbar takes 40px from the viewport; the
  Gallery-pro direction wants the image edge-to-edge when the user is idle.

- **Decision:** Viewer-only dissolve keyed off the existing `OVERLAY_IDLE`
  clock (no new timers). The bar hides via `.hidden()` (Display::None — no
  hitboxes, stable IDs); info/actions survive as translucent corner chips.
  Tab (overlays off) pins the solid bar: a user who hid the overlays keeps
  the chrome. Grid never dissolves. Fit math is dissolve-aware via
  `viewer_fit_height()`; bar-state changes trigger `refit_for_viewport()`
  for Fit-mode images only (Percent100 user zoom is intentionally left
  alone).

- **Consequences:** The dissolve is an instant hide/show, not an animated
  fade — animated opacity fades are deferred to the V3 motion layer.

- **Alternatives considered:** always-floating chips only (rejected: actions
  need affordance clarity when active); opacity-only animation now
  (rejected: V3 scope).

## ADR-011: V3 sort engine — session-owned order with metadata-carrying entries

- **Status:** Accepted

- **Context:** The gallery historically scanned with a name-only natural
  sort baked into `scan_dir`. V3 requires sorting by name / created /
  modified / size / type with asc/desc, persisted globally in settings,
  changed from a topbar dropdown — and the selection must stay on the
  same image when the order changes.

- **Decision:** The session is the single source of truth for order.
  `sh_core::navigation` gains `ImageEntry { path, size, modified,
  created }`, `scan_entries` (one enumeration pass; on Windows metadata
  rides the directory listing, no per-file stat), and a pure comparator
  `compare_meta` over borrowed `MetaView`s (ties break by natural name,
  always ascending — deterministic output for every criterion; `created`
  falls back to `modified` where the OS can't report it).
  `Session::resort()` sorts `ImageItem`s in place through those views —
  probed dimensions survive per path — and re-anchors `current` by
  path, so the grid never jumps images. Settings v2 persists
  `sort_by`/`sort_dir` with per-field `#[serde(default)]` so a v1 file
  migrates by loading `Name/Asc` defaults without dropping existing
  keys. `scan_dir` delegates to `scan_entries` (one scan/sort path).

- **Consequences:** Changing the criterion with a folder open is pure
  in-memory work — zero re-scan. `App::set_sort` mirrors the change into
  the settings copy and persists. Cost: `ImageItem` carries 3 metadata
  fields (~40 bytes/image); `open_folder` → `open_path` re-scans the
  same folder once (OS-warm readdir — accepted).

- **Alternatives considered:** lazy per-file `stat()` at sort time
  (O(n) syscalls on every criterion change — rejected); round-tripping
  items through `ImageEntry` on resort (drops async-probed dimensions
  that feed fit/zoom math — rejected, the trap the re-anchor tests pin).

## ADR-012: Embedded compile-time i18n table (English + neutral Spanish)

- **Status:** Accepted

- **Context:** Every user-facing string lived as an inline English literal at
  its render call site (`sh-app`), with sentence fragments concatenated
  positionally (`format_report("Moved", …)`, `batch_bar_message`). Adding a
  second language on that shape would fork word order bugs (Spanish puts
  counts and verbs in different positions) and leave no mechanism to detect
  a string added in one language but not the other.

- **Decision:** A compile-time table in `sh-core::i18n` (no `gpui`/`tokio`
  imports, no new dependencies): `Language { En, Es }` (serde lowercase,
  default `En`, no OS-locale seeding), one `StrKey` variant per inventoried
  string, and `Language::get` over two exhaustive `match` arms returning
  `&'static str` (O(1), zero allocation). A missing arm is a *compile
  error*; an `ALL_KEYS` anti-drift test asserts every key renders non-empty
  in both languages; an empty Spanish arm falls back to English so the UI
  never blanks. Interpolated sentences are per-language named-argument
  template functions (`batch_report`, `batch_bar_delete`, `batch_bar_move`,
  `recents_header`, `conflict_text`, `no_images_in`, `selected_suffix`);
  plurals go through `plural(n)` (`One` iff `n == 1`, proptest-pinned).
  `ShImagesError` Display prefixes and proper nouns (theme/folder names, key
  chips, brand, glyphs) stay outside the table by binding decision.
  `Settings.language` (schema 4 → 5, per-field `#[serde(default)]`) follows
  the proven sort/keymap migration pattern, so v4 files load with `En` and
  prefs intact.

- **Consequences:** Slices 2–5 become pure literal→lookup swaps threaded
  from `Settings.language`, each shippable independently (un-swapped
  surfaces keep their English literals). Cost: one new `sh-core` module
  (~600 lines with tests/docs); lookup adds a single `match` on data
  already in memory — no frame-loop, bench, or binary-size impact of note.
  Spanish wording needs a native-speaker review pass before the slice
  merges; the anti-drift test pins whatever wording lands.

- **Alternatives considered:** `HashMap<(Language, StrKey), &str>` (runtime
  construction + allocation, no exhaustiveness checking — rejected);
  per-key methods (~65 methods — rejected); format-string storage with
  call-site formatting (splits each sentence across two files, invites
  positional misuse — rejected); keeping `format_report`'s `verb_past:
  &str` (passes an English word into a Spanish sentence — rejected, fixed
  by the typed `BatchVerb` enum).

---

## ADR-013: Zoomable grid — persisted preset enum in core, pixel table in UI

- **Status:** Accepted

- **Context:** The gallery grid rendered every cell at one fixed geometry
  (180/170/160×120). S/M/L density presets needed a persisted choice plus
  per-preset pixel constants. `sh-core` cannot own UI geometry (AGENTS.md
  §3.2, dependency direction `sh-app → sh-core`), and an inherent
  `impl GridSize` outside `sh-core` violates the orphan rule — the same
  layering question ADR-011 (sort) and ADR-012 (i18n) already answered.

- **Decision:** `GridSize` (S / M / L, serde lowercase, default M) lives in
  `sh-core::settings` as schema v6 with per-field `#[serde(default)]`, so
  v5 files load as M with all prefs intact. The integer geometry table
  lives in `sh-app::ui::grid` as `GridGeometry` plus a `GridSizeGeometry`
  extension trait exposing `size.geometry()`; `grid_columns` /
  `grid_max_scroll` take `&GridGeometry`, and `App::set_grid_size` mirrors
  the proven `set_sort` contract (settings mirror + atomic persist +
  notify) with scroll re-clamp and cursor-into-view on top.
  `THUMB_MAX_DIM = 256` is unchanged — a size change is layout-only, zero
  re-decodes.

- **Consequences:** No new architectural pattern — the same
  persisted-enum-in-core / presentation-in-ui split as ADR-011/ADR-012.
  M renders the previous pixels verbatim, so the change is
  behavior-preserving until the user touches the chip. Cost: grid call
  sites pass one extra geometry param (mechanical, compiler-checked).

- **Alternatives considered:** pixel table in `sh-core` (rejected:
  presentation concern inside the pure-logic crate); float scale factors
  off M (rejected: rounding drift in scroll math; spec mandates
  integers); a redundant `session.grid_size` (rejected: no reader —
  density is view/persistence state, not session truth).

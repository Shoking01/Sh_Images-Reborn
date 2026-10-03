# Sh_Images — Architecture Decision Records

> Decisions are recorded per AGENTS.md §9.2 (Context / Decision /
> Consequences / Alternatives considered). Each ADR reflects the architecture
> **as built** on `main`; historical scope decisions are labeled where later
> work superseded them, and an ADR that a later amendment corrects says so in
> place. References to specific behaviors point at the code that implements
> them.
>
> **On version references.** ADRs written before ADR-020 cite `gpui 0.2.2` in
> their Context and Consequences sections. Those citations are historical: they
> record what was true when the decision was taken, and are left unedited so the
> reasoning stays readable. ADR-020 moved the framework to `gpui-pre` 0.3.7 and
> supersedes ADR-003; where an earlier ADR states a 0.2.2 behavior as a *current*
> constraint, read it against ADR-020's list of what changed.

---

## ADR-001: Cargo workspace with sh-core / sh-app separation

- **Status:** Accepted

- **Context:** AGENTS.md requires core business logic to never import GPUI.
  Convention alone drifts over time; a module boundary inside one crate
  enforces nothing.

- **Decision:** Two crates in one workspace. `sh-core` (pure logic:
  navigation, decode, theme, transform, settings) declares **no
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

## ADR-002: Initial V1 scope — solid single-image core first

- **Status:** Accepted for the initial V1; superseded in part by later
  Gallery, crop, and viewer work

- **Context:** The project restarted from zero in Rust after the previous
  GPUIX/React/Bun implementation was discarded. Delivering the full AGENTS.md
  surface (grid, crop, export) as a first milestone carried large risk on an
  unfamiliar, pre-1.0 UI framework.

- **Decision:** The initial V1 milestone shipped a single-image viewer with
  circular folder navigation, zoom/pan/fit, JSON themes with hot reload,
  ephemeral overlays, fullscreen, drag&drop / Ctrl+O / CLI-arg open, and
  settings persistence. Gallery, crop, and export were deferred at that
  point; later work records those additions separately.

- **Consequences:** The first milestone isolated the riskiest integrations
  (GPUI 0.2.2 rendering semantics, input dispatch, entity leases, and
  Windows platform quirks) behind a smaller surface. The current application
  now includes later Gallery and crop work; this ADR is historical scope
  context, not a current feature inventory.

- **Alternatives considered:** full AGENTS.md feature set in V1 (high risk of
  a half-finished everything); grid-first (doubles the decode/render surface
  before the single-image path was trustworthy).

---

## ADR-003: GPUI 0.2.2 pinned from crates.io

- **Status:** Superseded by ADR-020 (`gpui-pre` 0.3.7, pinned exact). The
  *reasoning* below still stands — GPUI is pre-1.0 and its API moves between
  releases — and ADR-020 keeps the discipline by pinning `=0.3.7` exactly
  rather than by staying on 0.2.2. The specific version below is historical.

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
  `sh_core::decode::load` and its `load_with_limit` sibling remain library
  capabilities — used by crop and thumbnail decode, not by the app's render
  path, which is entirely GPUI's.

- **Amendment (dead-code removal):** this ADR originally also listed
  `DecodeCache` (LRU with byte budget) among the unwired capabilities. That
  type has since been **deleted**, so the claim above no longer holds and
  the capability list is not part of this decision. Rationale: `DecodeCache`
  cached *decoded RGBA8* keyed by path, which is what GPUI's `img()` cache
  already holds in a form the renderer can consume directly. Keeping it
  meant a second cache over the same keys that nothing read — reachable
  only from its own tests, whose passing therefore proved nothing about the
  app. Deleting it removed a false signal, not a capability;
  `Settings::cache_memory_limit_mb`, the only knob that would have sized
  it, went with it (a persisted setting that controlled nothing observable).

  - **Consequences:** no memory-backed decode cache is available to a
    future non-GPUI consumer of `sh-core`; re-adding one means writing it
    against a real consumer rather than in advance of one. The settings
    schema did not need a version bump for the removal: `Settings` derives
    `Deserialize` without `deny_unknown_fields`, so a pre-removal file's
    orphaned key is ignored on read and every other preference survives —
    pinned by `file_carrying_removed_cache_field_loads_with_prefs_intact`.
    Adding `deny_unknown_fields` later would silently turn any future field
    removal into a total settings wipe, because `load` falls back to
    `unwrap_or_default()` on a parse failure.

  - **Alternatives considered:** keeping `DecodeCache` as a documented
    "library capability" (rejected — unexercised dead weight whose tests
    gave false assurance); replacing it with a new app-level cache
    (rejected — re-duplicates the GPUI cache this ADR resolved).

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
  surface refreshes the idle clock. A future fade would need to layer
  `.opacity()` *on top of* the gate; the current motion layer intentionally
  does not animate overlay visibility, and opacity alone would regress
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

## ADR-010: Hybrid topbar and in-flow bottom chrome (Viewer only)

- **Status:** Accepted

- **Context:** The Viewer should maximize the image while keeping navigation
  and actions reachable. A solid topbar plus a solid bottom row would stack
  chrome, and idle visibility must not make the image jump.

- **Decision:** The Viewer topbar is an overlay with zero layout space. It
  dissolves whenever the bottom chrome is armed (`show_overlay_bottom`); idle
  visibility changes do not alter that relationship. The bottom chrome is
  in-flow below the image, while the filmstrip is a separate fixed-height
  carve. `viewer_viewport()` is the shared geometry contract, and toggling
  the bottom chrome triggers one Fit-mode refit; Percent100 zoom is left
  untouched. Grid keeps its in-flow topbar. Hidden transient surfaces use
  `Display::None` so they have no hitboxes.

- **Consequences:** The image receives the full width and the intended
  vertical carve without two stacked solid bars. Tab is a deliberate layout
  change for Fit mode, while idle hiding is geometry-neutral. The dissolve
  remains an instant hide/show; ADR-017 adds only bounded background-hover
  motion, not overlay fades.

- **Alternatives considered:** make the topbar consume layout space (wastes
  image area); animate topbar/overlay visibility with opacity alone (does not
  remove hitboxes); remove transient elements from the tree (unstable IDs and
  more churn).

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
  integers); a redundant `session.grid_size` (rejected: no reader —"
  density is view/persistence state, not session truth).

---

## ADR-014: Grid viewport culling with layout-preserving placeholders

- **Status:** Accepted

- **Context:** Large folders previously built every cell's thumbnail, label,
  and click listener even when the cell was outside the clipped grid
  viewport. Removing cells outright would change flex-wrap positions, manual
  scroll math, and selection behavior.

- **Decision:** `visible_row_range` computes the half-open range of rows that
  intersects the current manual-scroll viewport from the active integer
  geometry. The render loop builds full cell content only for those rows and
  emits fixed-size placeholders for every other index. Culling is therefore a
  render-time optimization, not a `uniform_list` or masonry migration.

- **Consequences:** Off-screen cells do not allocate thumbnail elements or
  click listeners, while every index still occupies its original footprint.
  Scroll clamping, row positions, selection, and the S/M/L geometry table
  remain deterministic and independent of culling.

- **Alternatives considered:** render every cell (wastes frame work); remove
  off-screen cells (breaks wrap and scroll positions); migrate to a full
  virtual-list model now (larger behavior and testing surface than this
  release).

---

## ADR-015: Versioned settings contract for interval and reduced motion

- **Status:** Accepted

- **Context:** The persisted settings file already carried viewer flags, but
  the release adds a configurable slideshow interval and a reduced-motion
  preference. Older files must keep their existing preferences, and the
  platform has no OS reduced-motion query to seed a user value.

- **Decision:** `sh-core::settings` advances the current schema to v10.
  `slideshow_interval_secs` is a v9 field with a serde default of `3` and an
  inclusive validation range of `1..=60`; invalid persisted values fall back
  to `3` on load. `reduce_motion` is a v10 field whose serde default is
  `true`, and the App's shared persistence path stamps the current version.
  Existing atomic temporary-file-plus-rename writes remain the only settings
  writer.

- **Consequences:** v8 and v9 files load with safe defaults and their other
  preferences intact; an explicit `reduce_motion: false` round-trips. The
  interval remains a pure, testable core contract, while the App re-arms the
  timer after a valid change. No OS preference integration is implied.

- **Alternatives considered:** a version bump that discards unknown fields
  (would lose user preferences); defaulting reduced motion to false (unsafe
  without an OS query); separate settings files (fragmented source of truth);
  accepting and persisting arbitrary intervals (could create pathological
  timer loops).

---

## ADR-016: App-owned cancellable slideshow task

- **Status:** Accepted

- **Context:** The earlier permanent fixed-delay loop could not represent a
  changed interval and left task ownership implicit. Viewer entry, folder
  replacement, crop mode, and explicit stop all need a clear cancellation
  boundary without blocking the frame loop.

- **Decision:** `App` owns an optional slideshow `Task<()>`. `rearm_slideshow_timer`
  cancels the previous handle, then schedules a background-executor delay
  only when playback is active and the Viewer is visible. After waking, the
  task rechecks state, navigates once, and reads the current interval for the
  next delay. The boundary clamps even corrupted in-memory values to at least
  one second. Opening Settings pauses the task while preserving playback
  intent; returning to Viewer re-arms it, while folder changes, crop, leaving
  Viewer, and stop clear the pending task.

- **Consequences:** Interval changes take effect from a fresh delay, no
  zero-delay loop is possible, and each transition has an explicit owner.
  Next/previous navigation remains a normal playback step; the task does not
  block rendering or perform synchronous I/O.

- **Alternatives considered:** keep the permanent fixed loop (cannot apply a
  live interval); poll a clock from render (wasteful and imprecise); use a
  separate thread or external scheduler (extra synchronization for a task
  GPUI already owns).

---

## ADR-017: Reduced-motion-aware background hover transitions

- **Status:** Accepted

- **Context:** Existing controls already change background color on hover,
  but a broad animation pass would add layout churn and could conflict with
  the platform's lack of an OS reduced-motion query. Motion must be optional,
  bounded, and observable without changing element geometry.

- **Decision:** `ui::motion` provides one shared background-hover helper with
  a 150 ms duration and a single ease-out-quint curve. Stable static element
  IDs and direction-specific keys keep enter and leave animations separate.
  Only selected existing control backgrounds use the helper. When
  `reduce_motion` is `true`, the helper applies the hover color instantly;
  when it is `false`, it animates the background only. Text hover, press
  feedback, layout, grid staggering, and window transitions remain outside
  this layer.

- **Consequences:** The safe default is instant and keyboard/layout behavior
  is unchanged. Opting out adds one restrained color transition with no
  double-easing, while stable selectors and the App-owned phase state remain
  testable. GPUI 0.2.2's headless harness still verifies structure and state,
  not rendered pixels.

- **Alternatives considered:** animate every UI property (unbounded motion);
  add layout/grid transitions (risks geometry and scroll churn); introduce a
  new animation dependency (unnecessary for the existing GPUI API); assume
  an OS preference that the platform does not expose (not available here).

---

## ADR-018: Per-user Inno Setup packaging as the distribution and upgrade channel

- **Status:** Accepted; introduced on `installer/inno-setup`, now on `main`

- **Context:** The application had no distribution channel. The user chose
  update-by-reinstalling: there is no in-app updater, and a user upgrades by
  downloading and running a new installer over the existing installation.
  Settings and themes already live in `%APPDATA%\sh_images`
  (`crates/sh-app/src/main.rs`), outside any install directory, which is what
  makes in-place replacement viable. The audience is non-technical Windows
  users, so an install that demands administrator rights or triggers UAC is a
  real adoption barrier, and the app must be discoverable and removable
  without instructions. The release workflow also had to produce a durable,
  verifiable artifact rather than a build result that vanishes with the
  runner.

- **Decision:** `installer/sh-images.iss` (Inno Setup 6) is the distribution and
  upgrade channel. It installs per-user to
  `{localappdata}\Programs\Sh Images` with `PrivilegesRequired=lowest`, so no
  administrator rights and no UAC elevation are required. `AppId` is a fixed
  literal GUID: it is the upgrade identity Inno uses to recognize an existing
  installation, run its uninstaller, and replace the program files in place, so
  it must never change. The MIT `LICENSE` is shown during setup and installed
  alongside the executable, and `CreateUninstallRegKey` gives users a
  discoverable Add/Remove Programs entry. `[Icons]` creates a Start Menu group
  plus an optional desktop shortcut, both using
  `assets/branding/sh-images.ico`, which is installed into `{app}` by `[Files]`
  so the shortcuts reference a file that provably exists. **No file
  associations are registered**, even though `crates/sh-app/src/main.rs` already
  reads a path from `argv` and would open it: per-user `HKCR` handler entries
  take over the user's default handler for those extensions, and that deserves
  its own reviewed change with manual QA. `[Run]` offers a post-install launch
  that is skipped under silent install. In `.github/workflows/release.yml` a
  single `cargo build --release -p sh-app` produces both a portable ZIP and the
  installer, so the published artifacts cannot disagree about which executable
  they ship; the installer is then compiled, smoke-tested with a real silent
  install and uninstall, and a `SHA256SUMS.txt` is computed last so it covers
  every published file. Those Actions artifacts are per-run, need a login to
  download and expire after 30 days, so
  `.github/workflows/publish-release.yml` repeats the same packaging steps on a
  `v*` tag and attaches the ZIP, the installer and the checksum file to a real
  GitHub Release, which is the durable copy. It gates on the tag before it
  builds: the pushed tag and the `[workspace.package]` version must agree,
  because the tag is the release identity while Cargo.toml is what was actually
  compiled, and a Release that announces `v0.1.1` while shipping the `0.1.0`
  build is worse than no Release at all. `.github/workflows/ci.yml` validates
  the script on every pull request using an explicit stub payload, because Inno
  Setup hard-errors at compile time when a `[Files] Source:` file is missing.

- **Consequences:** User data in `%APPDATA%\sh_images` survives install,
  upgrade, and uninstall untouched, so upgrading never costs a user their
  settings or custom themes. There is no UAC prompt and no machine-wide write.
  The `AppId` is now load-bearing for every future release: changing it would
  give every existing user a second, conflicting installation and a second
  uninstall entry, which is why the script documents it as immutable. The
  branding icon is installed into `{app}` rather than referenced from the
  repository, because `[Icons] IconFilename` is a **runtime** path that Inno
  writes verbatim into the `.lnk` and never validates — at compile time or at
  install time. A wrong icon path is therefore a **silent failure that reports
  success**: the setup log records "Successfully created the icon" with no
  warning while the shortcut points at a path that resolves to nothing. Related:
  `[Run]` entries execute programs and cannot create shortcuts, so the desktop
  shortcut is created solely by `[Icons]` under the `desktopicon` task. Because
  of the icon behavior, installer verification must assert the installed
  artifact — the real `IconLocation` read back from the created `.lnk` — and
  never the installer process exit code alone; the procedure is in
  `docs/RELEASE_QA.md`. `unins000.exe` also re-launches itself from a temp copy
  and can return before the install directory is gone, so uninstall checks must
  poll. Script-only validation needs a stub payload, which is a test fixture
  and never a substitute for an end-to-end test against a real binary.

- **Alternatives considered:** machine-wide install under `Program Files`
  (requires administrator rights and UAC, breaking the no-elevation goal);
  MSIX (packaging identity and signing requirements plus sideload friction for
  a public download); an in-app or Squirrel-style updater (explicitly rejected
  by the user in favor of update-by-reinstalling); a portable ZIP alone (no
  upgrade path, no Add/Remove Programs entry, and no Start Menu entry, leaving
  users to guess where the executable went).

## ADR-019: User themes discovered on Settings open, merged into one picker list

- **Status:** Accepted

- **Context:** AGENTS.md §10 requires the app to scan
  `~/.config/sh_images/themes` for user themes and to let users share a
  theme as a single JSON file. Neither happened: `theme::discover` had zero
  callers outside its own tests, the picker iterated only the four compiled
  in `BUILTIN_THEMES`, and the app *wrote* into that directory without ever
  listing it. Three problems had to be solved at once. (1) Discovery is
  `read_dir` plus a JSON parse per file, and the settings panel renders on
  the main thread, so it cannot run in `render` (AGENTS.md §7.1). (2) The
  first launch bootstraps a copy of the active built-in into that same
  directory (`theme_startup`), so any correct listing shows a built-in
  twice. (3) `ThemeStore::path` is what the hot-reload watcher polls, so a
  user theme must carry a real path or hot reload is silently dead for it.

- **Decision:** Three separable pieces, each with one job.
  **`sh_core::theme::load_discovered(dir)`** enumerates via the existing
  `discover` and returns `DiscoveredTheme { path, file_name, theme:
  Option<Theme>, text, error }` — a parse failure is *reported*, never
  dropped. **`appearance::merge_theme_entries(themes_dir, discovered)`** is
  pure and total: built-ins first, then user themes, deduped by
  (trimmed, lowercased) theme `name`, with a **discovered file replacing the
  built-in row in place** — the user's copy is the editable, watchable one,
  and preferring the built-in would hide it. An invalid file has no name to
  match on, so it can never displace a row; it is appended, rendered dimmed,
  and takes no click. **`App::theme_entries`** caches the merged list and
  `App::theme_load_seq` guards refreshes, mirroring `list_load_seq`.
  `open_settings` triggers the scan, which runs entirely on the background
  executor; `App::new` seeds the built-ins so the picker is never empty.

- **Consequences:** The Appearance row count is now user-unbounded, and
  `appearance_content_h(theme_count)` is still exact: both the render and
  the wheel handler's clamp read the same `theme_entries.len()`, so a tall
  list scrolls correctly for any count. The height is deliberately neither
  clamped nor capped — clamping the height would strand the last row, and
  capping the list would need a "more" affordance (new i18n string, new
  focus target) to bound a directory of ~500-byte files. Hot reload works
  for user themes because applying an entry sets `ThemeStore::new` with the
  row's real path AND its real file text, so the watcher's first read is
  deduped instead of re-applied. Known cost: two different themes declaring
  the same `name` collapse to one row. Re-scanning on every Settings open
  (rather than once at startup) is what makes "drop a file in, reopen
  Settings" work; the cost is one directory read per open, off the frame
  loop.

- **Alternatives considered:** a background theme *watcher* mirroring the
  hot-reload poller (rejected — a second poll loop for a list nobody
  watches continuously; open-time discovery covers the real workflow);
  hiding invalid files (rejected — "not discovered" and "discovered but
  broken" become indistinguishable, so a user's dropped file appears to
  vanish); dedupe by file identity or content hash (rejected — the
  duplicate IS a different file with identical content, so neither
  distinguishes it; the theme `name` is the only identity the two copies
  share); a "themes" folder in the picker (rejected — duplicates the
  existing file-open path for no gain over "drop the JSON in the folder").

---

## ADR-020: gpui-component for the top bar, on gpui-pre 0.3.7

- **Status:** Accepted on `spike/gpui-component`; supersedes ADR-003

- **Context:** ADR-003 pinned `gpui = "0.2.2"` because GPUI is pre-1.0 and its
  API moves between releases. That reasoning still holds, but 0.2.2 cannot host
  `gpui-component` 0.7.0, which is the only mature styled-component layer for
  GPUI. Adopting it forces the framework question first, so the pin and the
  component layer have to be decided together rather than sequentially.

  Four breaking changes apply when moving to `gpui-pre` 0.3.7, all verified
  against the crate source rather than against the published examples, which
  do not compile as written:
  `Application::with_platform(gpui_pre_platform::current_platform(false))`,
  `Window::focus` gaining a second `&mut App` argument (14 call sites),
  `KeyDownEvent` gaining `prefer_character_input`, and
  `VisualTestContext::update` handing its closure both the window and the app.

  Three defects were found by running the build and reading pixels. None is
  reachable by reading source, and none is covered by the existing suite:

  1. **`Application::with_assets` replaces the registered `AssetSource`; it does
     not compose** (`gpui-pre/src/app.rs:202-206` assigns
     `context_lock.asset_source` and rebuilds the `SvgRenderer`). Registering the
     kit's bundle as a second source therefore *disabled* all twelve of the app's
     own icons. The kit's `Assets` embeds a 104-icon default bundle that
     contains `arrow-left` and `settings` but **not** `scissors`, and its
     `AssetSource::load` answers a miss with `Err` — which GPUI paints as an
     empty `svg`: no panic, no log line, no failing test. The visible symptom was
     one invisible Crop button; the actual damage was every app icon.
  2. **`ThemeColor::default()` is not a palette.** All 134 fields are
     `{ h: 0, s: 0, l: 0, a: 0 }` — fully transparent black. A theme projected
     from it leaves every unmapped slot invisible, so any kit component outside
     the top bar would paint nothing, silently, with no error anywhere.
  3. **`Selectable::toggled` does not paint.** It sets accessibility metadata
     only; `Selectable::selected` is what paints. The density segments called
     `toggled` alone on a comment claiming otherwise, so the active preset was
     correct for screen readers and invisible to everyone else — resting and
     active segments both measured `25252B`.

  Two capabilities were probed and are **not** available, so they are recorded as
  negative results rather than left to be rediscovered: there is no per-element
  backdrop blur (`blur_radius` exists only on `BoxShadow`, so a "frosted" bar
  over an image can only ever be plain alpha), and although
  `WindowBackgroundAppearance::{Blurred, MicaBackdrop, MicaAltBackdrop}` are all
  genuinely implemented in `gpui-pre-windows`, with a saturated window behind
  the app neither composited — the backdrop does not reach the wgpu swapchain.
  The bar sampled `15151A` and the grid `101014` against a `FF00FF` window.

- **Decision:** Pin `gpui = { package = "gpui-pre", version = "=0.3.7" }` — exact,
  not caret. Adopt `gpui-component` 0.7.0 for the **top bar only**, and keep the
  grid, viewer, filmstrip and settings panel as hand-built `div`s. Three
  separable mechanisms keep that from creating a second source of truth:

  **One asset source.** `AppAssets` owns the app's twelve SVGs and falls back to
  `gpui_kit_assets::Assets`; `main` registers that one source instead of two.
  Composition is impossible at the API level, so the fallback is what stands in
  for it.

  **One theme, projected.** `kit_theme::sync_kit_theme` maps the app's JSON
  colors onto the slots the top-bar controls read, and applies the mapping
  **over the kit's resolved palette** rather than over `ThemeColor::default()`.
  `Theme::update` reconciles `ThemeColor` into `ThemeTokens`, and the components
  read the latter — so the projection is only correct if it starts from a real
  palette. It runs at startup, on every `apply_theme_entry`, and from the
  hot-reload watcher.

  **One constructor.** `App::new` takes `&mut Context<Self>` because it arms the
  thumbnail batch for the session it is born with. `main.rs` seeds that session
  from a synchronous CLI scan, so no async commit handler ever arms the batch on
  the startup path; arming from `main` instead would have left a regression test
  passing while `main` stopped doing it.

  `window_background` stays `Opaque`, and the bar's translucency is kept
  deliberately subtle: it reads against the app's own background, which is what
  actually composites.

- **Consequences:** The release binary goes from **10.10 MB on `main` to
  18.39 MB**, over the `< 15 MB` target in AGENTS.md §6.1, though under the
  25 MB hard ceiling. Measured attempts to pay that back, both of which failed,
  so the cost is stated as accepted rather than deferred:

  - **`default-features = false` on `gpui-component` is a no-op.** The crate
    declares no `default` feature at all; `decimal`, `inspector`, `test-support`
    and all 30 `tree-sitter-*` entries are opt-in and already off. An earlier
    draft of this ADR named this as the obvious lever. It was not, and the
    assumption is recorded here so it is not repeated.
  - **`lto = "fat"` is not worth it.** The release profile already runs
    `lto = "thin"`, `codegen-units = 1` and `strip = true`. Switching to fat LTO
    measured 18.39 MB → **18.18 MB** for a build time of **1m45s → 9m26s**: 0.21
    MB for 5.4x the compile time. Left on thin.

  What actually costs the 8.3 MB is 151 new transitive crates, and none of them
  is optional. Roughly half is the framework move itself — `gpui-pre` 0.3.7
  pulls in a `wgpu` backend (`wgpu`, `wgui-core`, `wgui-hal`, `naga`,
  `gpu-allocator`) that 0.2.2 did not — which is the price of ADR-020's own
  premise. The rest is unconditional to `gpui-component`: `lsp-types` +
  `ropey` + `notify` for its code editor, `html5ever` + `markup5ever` +
  `xml5ever` for rich content, `rust-i18n` for i18n, and `accesskit`/`atspi`
  for accessibility. There is no feature flag that turns them off. Recovering
  the size would mean giving up either the framework version or the component
  layer, which is the decision this ADR already made.

  The exact pin means a `cargo update` cannot move the framework underneath the
  app, and every 0.3.x bump has to re-verify those four break sites plus the two
  asset paths by hand.

  The theme bridge is now three systems in conversation — the app's four JSON
  colors, the kit's ~134 slots, and `gpui_base::SemanticThemeTokens`, which the
  kit itself calls the layer for application-owned components. Migrating the grid
  to kit components grows the bridge rather than removing it, so the schema
  decision (widen `sh-core`, or keep it curated and accept a ceiling on what
  "modern" can express) is still open and is deliberately not decided here.

  Migrating further is not free. `Selectable` needs importing for `selected` /
  `toggled`, and `Sizable` for `with_size`; element ids move into `Button::new`;
  and `on_click` takes a plain closure rather than `Context::listener`. The
  existing `debug_selector` contracts survived, so the visual test suite needed
  no edits.

  Three pre-existing ADRs are touched and are labeled where affected: ADR-005
  (the `img()` path is where the startup-thumbnail defect lived), ADR-009 (the
  `AssetSource` that `with_assets` replaces rather than extends), and ADR-014
  (the placeholder that every grid cell rendered).

- **Alternatives considered:** **stay on 0.2.2 and hand-build** (rejected — it
  preserves the binary size and needs no bridge, but the request was a coherent
  component layer, and hand-building one is the status quo that prompted it);
  **`guise`** (Mantine-flavored, 120+ components, MIT — rejected because it
  builds against crates.io `gpui 0.2.2`, so adopting it would undo the framework
  move entirely); **`adabraka-ui`** (~140 components — rejected because it
  requires a custom GPUI fork, which no upstream PR can survive);
  **`gpui-base` alone** (unstyled behavior and state — attractive for owning
  every pixel, but it means building the visual system from nothing, which is
  the thing this decision was made to stop doing); **`gpui-shell`** (rejected
  out of hand — its JavaScript extension runtime violates AGENTS.md §7.1);
  **map the theme onto `ThemeColor::default()`** (rejected — defect 2 above);
  **arm the thumbnail batch from `main.rs`** (rejected — makes the regression
  test vacuous).

---

## ADR-021: Themes declare semantic slots; four colors stay the required core

- **Status:** Accepted on `spike/gpui-component`; amended in place to fold in
  `Theme::interaction`, which this ADR named as its own follow-up

- **Context:** ADR-020 migrated the top bar to a component layer and left one
  question open on purpose: whether `sh-core`'s theme schema widens or stays
  curated. The question is forced by a real defect, not by preference.

  The app **computes** its interaction states instead of declaring them.
  `hover_fill` and `hover_fill_strong` each pick a mix ratio from a color's
  luminance (~10% on dark, easing to ~7% on light) and `viewer_control_hover_fill`
  does the same for the viewer chrome. Three consequences:

  - A theme author who wanted a specific hover had no way to ask for one. The
    only way to change it was to edit Rust.
  - Every component outside the top bar fell through to the component layer's
    own neutral on hover, because the kit has no slot the app was feeding. That
    is the split-source-of-truth failure ADR-020's bridge exists to prevent,
    one layer out.
  - The ratio logic is luminance-adaptive by design, which is correct and worth
    keeping as a *default*. What is missing is the ability to override it.

  The tempting fix — make `sh-core`'s `Theme` BE the component library's
  `ThemeConfig` — is unavailable. `sh-core` declares no GPUI dependency and
  `lib.rs` documents that the compiler enforces it (ADR-001, AGENTS.md §3.2). A
  `sh-core` type that named a kit type would make the pure crate depend on the
  UI toolkit, which is exactly the inversion the workspace split exists to
  prevent.

- **Decision:** `ThemeColors` keeps its four required colors and gains six
  **optional** slots: `muted_text`, `border`, `danger`, `on_accent`, `elevated`
  and `ring`. They are declared in `sh-core` in the crate's own vocabulary, not
  borrowed from any toolkit.

  Optional is load-bearing. `#[serde(default)]` plus
  `ThemeColors::with_derived_defaults()`, called inside `parse()` *before*
  validation, means a theme file written against the four-color schema renders
  exactly as it did, with no edit. `validate()` therefore always sees a complete
  palette and never has to know which schema the file was written against.

  Derivation is a fallback, never a normalizer: a slot the file declared is left
  alone, always. Re-deriving would silently discard an author's explicit choice
  on every reload.

  All four built-in themes declare every slot explicitly. They are the reference
  palettes, and a derived value in one of them makes the shipped design a
  function of the ratios rather than something a reader can see and tune.
  `builtin_themes_declare_every_optional_slot` inspects the raw JSON rather than
  the parsed theme, because parsing fills the gaps — the only way to tell a
  declared slot from a derived one.

  The bridge maps them onto the component layer: `muted`, `muted_foreground`,
  `border`, `ring`, `title_bar`, and the whole `danger` family rather than just
  its idle color.

  **Amended.** `Theme` also carries `interaction { hover_ratio,
  hover_ratio_strong }`. `hover_fill` and `hover_fill_strong` took their mix
  ratio from constants in `sh-app` (0.10 dark / 0.07 light, and 0.135 for chips
  brighter than the page). The luma ramp is kept — it is calibrated, and its
  comment records user feedback behind the light-theme floor — but the **anchor**
  is now the theme's. All eight production call sites read it from the live
  theme; `viewer_control_hover_fill` takes the whole struct so a caller
  mid-hot-reload cannot pair a stale ratio with fresh colors. Defaults are the
  constants replaced, asserted against those exact values and verified on
  screen: bar `15151A`, chip `25252B`, hairline `2D2D32` and the thumbnail all
  unchanged to the byte. Out-of-range ratios are rejected at parse — a control
  would otherwise blend past its own text and invert, and `"hover_ratio": 10`
  for `0.10` is exactly the typo that would do it quietly.

  A ratio was chosen over declaring hover *colors* because the luma ramp is
  calibrated against real feedback and encoding it per surface would need four
  color slots duplicating it. The cost: a theme sets how strongly controls
  react, not what color they become.

- **Consequences:** A user's theme file now controls the muted text, hairlines,
  destructive color, on-accent contrast, one elevated surface and the focus
  ring — across every surface, not just the top bar. Migrating the grid, viewer
  or settings panel to kit components no longer grows the bridge with
  hand-written mappings, because the vocabulary already exists on both sides.

  Two derivations needed care rather than a formula:

  - **`danger` is polarity-dependent.** A fixed red is right on a dark theme and
    a pale sticker on a light one, so it anchors toward `text` when the page is
    light — the same luminance branch `hover_fill` already used. A test asserting
    "further from the page" was written first and was wrong: on `light-clean`
    the text color is darker than the surface, so the blend moves down and the
    derived value ends up numerically *closer* to the page while still being the
    correct next surface in the ramp. The assertion now states the mechanism —
    one step from `surface` toward `text`, in the correct direction.
  - **`border` and `ring` must stay translucent.** An opaque hairline would draw
    a hard line where the design intends a 18% fade, so both are asserted to
    derive with alpha below 1.

  The derivation ratios are now part of the theme contract: they decide what a
  user's *un-updated* theme looks like, so changing `CHIP_RESTING_LIFT`-style
  constants later is a visible change for anyone still on four colors. That is a
  real cost of making derivation reachable, and it is why the built-ins declare
  explicitly — an update to the ratios cannot silently restyle the shipped
  themes, only user ones.

  **No literal colors remain in production rendering.** `hover_fill`,
  `hover_fill_strong` and `viewer_control_hover_fill` account for every control
  hover, and all three now take their strength from the theme. The one place
  that was still painting a hardcoded color — the shortcuts panel's error text,
  which read `rgb(0xff5555)` under a comment explaining that themes had no
  error token — now reads `danger`, and the comment is replaced. Auditing for
  literals rather than assuming the amendment was complete is what found it;
  the amendment had been written believing hover was the last gap.

  What remains is `hover_tint`, the pure blend primitive behind all of it, and
  that is correct where it is: it is the mechanism, not a decision. Pressed and
  selected states have no hand-computed fills left at all — the top bar's are
  the kit's, and the sort chip and crop toggle set `selected` for exactly that
  reason.

  Six color slots is still a curated subset of the component library's ~134. The
  ceiling ADR-020 described has moved rather than disappeared: it is now
  "expressed in the app's own vocabulary" instead of "cannot be expressed at
  all".

- **Alternatives considered:** **keep four colors** (rejected — the status quo
  ADR-020 identified, where any component outside the top bar uses a palette
  that does not belong to the user's theme); **adopt the toolkit's
  `ThemeConfig` as `sh-core`'s schema** (rejected — makes the pure crate depend
  on the UI toolkit, against ADR-001 and AGENTS.md §3.2, and the compiler
  enforces it); **mirror all ~134 toolkit slots in `sh-core`** (rejected — a
  pure business-logic crate should not carry a UI toolkit's vocabulary, and most
  of those slots describe components this app does not have); **make the six
  slots required** (rejected — would break every existing theme file, including
  the four built-ins, for no gain over deriving them); **derive every slot at
  render time from the four colors** (rejected — same failure as ADR-020 defect
  2, one step removed: correct colors that the author has no way to influence).
---

## ADR-022: Visual baselines are a human-reviewed artifact, never a gate

- **Status:** Accepted on `spike/gpui-component`

- **Context:** A grid cell sized itself from its content, so a photo taller than
  the thumbnail slot grew its whole flex line: 146px cells against a 170px preset,
  images painting over the row above and the label below. Every gate was green.
  `grid_max_scroll` and `visible_row_range` passed because they test arithmetic,
  and nothing in the suite tested the layout that arithmetic describes.

  The response was to test relations instead of absolutes. `debug_bounds` reads
  real computed bounds out of `window.rendered_frame.debug_bounds`, so
  `grid_rows_are_uniform_and_never_overlap` and
  `viewer_chrome_stays_inside_its_band` (ADR-020, `spike/gpui-component`) are
  deterministic, need no GPU, no driver, no font fallback and no DPI, and already
  run in CI. Those guards were proven to fail: reverting `.h(px(row_h))` alone
  reproduces `grid-cell-0 height 146 != preset row height 170`.

  What they still cannot catch is recorded in `docs/spike-gpui-component.md`:
  `overflow_hidden()` clips PAINTING without changing BOUNDS, so a missing clip
  leaves every geometric assertion passing while pixels still spill. Bounds are
  not the only thing that determines what a user sees, and no arrangement of
  assertions over bounds can see a painted pixel that lies outside them.

  That leaves one option, and it is a well-known trap. Golden-image comparison is
  what every UI toolkit reaches for, and it is the standard answer to "compare
  pixels". Rendered output here comes from a `wgpu` swapchain composited by
  Windows, so it varies with GPU driver, Windows build, DPI scale and font
  fallback. A diff therefore fails on changes that are not regressions, and the
  second-order effect is worse than the first: a gate that cries wolf is a gate
  whose failures get re-baselined instead of read. The gate stops being a gate and
  becomes a ritual.

  So the question is not whether to capture pixels. It is whether a pixel
  comparison is allowed to have an opinion about whether a build is good.

- **Decision:** **Pixels are captured, compared and reported. They never decide.**
  Two mechanisms with two different jobs.

  **Geometry invariants gate CI.** Unchanged and still required. They are the
  automated answer, because they are deterministic and they assert the property
  that actually broke rather than the appearance of it.

  **Visual baselines are advisory.** `tools/visual/capture.ps1` launches the real
  release binary against a generated fixture set in a hermetic profile, captures
  each window with `PrintWindow(hwnd, hwnd, PW_RENDERFULLCONTENT)` and writes PNGs
  plus `summary.json`. `-Compare` reports a per-image pixel-delta percentage and
  writes diff PNGs. **It cannot change the exit code on a delta.** There is no
  `--update-baselines` flag, and `.github/workflows/visual-baseline.yml` is
  explicitly not a required status check and never fails on a delta. Regenerating
  a baseline is a human decision, made in the same commit as the change that
  justified it.

  Three guards exist because a wrong frame is worse than no frame, since a human
  reviewing a screenshot calls a broken half-painted image a regression:

  - **A stability gate.** A frame is accepted only after two consecutive captures
    are pixel-identical, so a mid-load frame (thumbnails still decoding, half the
    window painted) is never written.
  - **Blank detection.** A frame whose sampled pixels are essentially uniform is a
    capture FAILURE and is never written as an artifact. A CI runner with no
    compositor produces solid black frames that look like valid PNGs.
  - **Process hygiene.** The launched process is killed in a `finally`, on success
    and on every error path.

  Two inputs that reach the renderer are pinned, because they are not app state
  and neither is obvious in a diff:

  - **The mouse pointer is parked** outside the window for the duration of a
    scenario and restored afterwards. This was not theoretical. An unpinned run
    produced two different "stable" frames for identical code, because a hovered
    grid cell paints an elevated card 180px wide against the 160px that every
    other cell measures. A human reading that diff sees a cell that grew and calls
    it a layout regression. It is not one.
  - **The OS-drawn band above the client area is excluded from every delta**,
    measured with `GetClientRect` and `ClientToScreen` rather than hardcoded. The
    native title bar paints a warm gradient when the window is focused and flat
    `#202020` when it is not: 31252 pixels, 4.05% of a 1016x759 frame, for a
    difference the app did not make. Forcing activation was tried and rejected.
    Windows' foreground lock refuses a background process, borrowing the
    foreground thread's input queue still failed on a desktop in active use, and
    stealing a developer's focus four times per run is its own problem.

  With those pinned, four consecutive runs of all four scenarios were **byte
  identical**, and `-Compare` against the committed baselines reported **0.00% on
  every image**. That is what makes the report worth reading on one machine. It is
  explicitly not a claim about any other machine.

- **Consequences:** The gap `overflow_hidden()` opens is now closed for humans and
  left open for machines, which is the honest shape of it. The harness catches
  what bounds cannot: paint escaping its box, an element drawing the wrong colour
  or at the wrong offset, a control present in the tree and invisible on screen,
  which is the class ADR-020 defect 3 was, where `Selectable::toggled` set
  accessibility metadata only and every test referencing the element id passed.

  It still cannot catch: any view with no scenario (Settings, crop mode,
  slideshow, drag-and-drop, the top bar under a narrow window); interaction states
  reachable only through real input rather than a CLI argument; theme changes,
  because the harness runs on bootstrapped defaults and a custom theme would need
  a hand-built profile; multi-window or cross-process behaviour; and correctness
  in motion, because the stability gate deliberately refuses frames that are still
  changing. It also cannot say *why* pixels differ, only that they do, which is
  exactly why the number is advisory.

  The measured portability caveat is why this cannot be promoted later without new
  evidence. Across machines the captured rect itself changes: window borders, DPI
  scale and the Windows build all alter it. A size difference is reported as
  `dimension_mismatch` at 100% with the non-overlapping area painted magenta, so a
  cross-machine delta is not a regression signal in either direction. Promoting
  this to a gate would first require pinning a rendering environment the project
  does not control.

  The fixture set is generated rather than committed, for a specific reason: it
  cycles wide (800x200), square (400x400) and tall (200x800) in solid distinct
  colours, so any consecutive subset contains a mix and a tall image always sits
  next to shorter ones in the same row. That adjacency is the precondition for the
  original defect being visible at all. Generation is byte-stable across runs,
  verified by SHA-256, and the profile is hermetic: `config_dir()` reads the
  `APPDATA` environment variable directly, so pointing the child process at a
  fresh temporary directory gives it bootstrapped defaults and a bootstrapped
  theme without reading or writing the developer's real profile. `settings.json`
  is deliberately never hand-authored, because the app writes current defaults into
  a missing profile and a hand-written file would drift the day the schema version
  moves.

- **Alternatives considered:** **make golden images a required status check**
  (rejected — non-reproducible across GPU driver, Windows build, DPI and font
  fallback; fails on changes that are not regressions and trains re-baselining
  instead of reading failures); **gate only same-machine runs** (rejected — a gate
  that silently stops applying the moment the machine changes is worse than no
  gate, because it still looks like coverage); **assert paint containment in the
  layout tests instead** (rejected — `overflow_hidden()` produces no observable
  signal in the bounds tree, so there is nothing to assert against; the absence
  of the clip is not expressible as a property of bounds); **drop visual capture
  entirely and rely on the geometry guards** (rejected — leaves the paint-overflow
  gap open permanently and forgoes the class of defect that bounds structurally
  cannot see, which is how the Crop button shipped invisible); **capture on every
  CI run and let humans diff** (accepted — this is what the advisory workflow does,
  minus the automated comparison, which across machines would report a large delta
  on every run and teach everyone to ignore it).
# Sh_Images 2.0 — V1 Design

> Native, GPU-accelerated image viewer built with **Rust + GPUI** (Zed's framework).
> Replaces the discarded GPUIX/React/Bun implementation. Total restart from zero.
> Date: 2026-09-09 · Status: Approved design (brainstorming)

---

## 1. Context

Sh_Images is a native image viewer for Windows, built from scratch in Rust + GPUI.
The previous version (GPUIX + React + Bun) is **discarded** and must not be ported.
The V1 targets a solid core single-image viewer before any grid/crop/export work.

Core priorities (from AGENTS.md, in order): performance, efficiency, customization, reliability.

### Decisions locked during brainstorming

| Decision | Value |
|---|---|
| Stack | Rust + GPUI pure (crates.io `gpui = "0.2.2"`) — no JS/WASM/web tech |
| Platform | Windows-first (DirectX), code kept portable |
| V1 scope | Solid core: single image, circular navigation, zoom/pan, fit, themes, shortcuts |
| Opening | CLI args + drag & drop + file dialog |
| Formats | PNG, JPEG, GIF (still), BMP, WebP, TIFF |
| Aesthetic | Minimalist pure: zero chrome, ephemeral overlays |
| Default theme | Deep Neutral (#0d0d0f) |
| Mouse wheel | Zoom anchored to cursor; drag = pan; double-click = toggle 100%/fit |

---

## 2. Architecture: Cargo workspace with 2 crates

```
Sh_ImagesR 2.0/
├── Cargo.toml            # workspace: members = ["crates/sh-core", "crates/sh-app"]
├── crates/
│   ├── sh-core/          # Pure library. ZERO gpui. Headless tests.
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── navigation.rs   # folder scan, natural sort, circular navigation
│   │   │   ├── decode.rs       # decode abstraction via `image` crate (multiformat)
│   │   │   ├── theme.rs        # theme JSON parse/validate + hot reload support
│   │   │   ├── cache.rs        # LRU of decoded images (V1: small, 2 slots)
│   │   │   ├── transform.rs    # pure zoom/pan/fit math (f32, no GPU)
│   │   │   ├── errors.rs       # ShImagesError (thiserror)
│   │   │   └── settings.rs     # schema for config serialization (serde)
│   │   └── tests/              # fixtures + integration tests
│   └── sh-app/           # GPUI binary. UI + workers + platform.
│       ├── src/
│       │   ├── main.rs         # entry: app, window, cli args
│       │   ├── app.rs          # root component: global state, theme, key handling
│       │   ├── ui/             # GPUI components (viewer, overlays, ephemeral bar)
│       │   ├── workers/        # decode threads (std::thread + channels)
│       │   ├── platform/       # Windows: drag&drop, file dialog, config dir
│       │   └── state/          # session state (current image, zoom, index)
│       └── tests/              # smoke integration tests
├── themes/               # built-in themes JSON + user theme discovery
│   └── deep-neutral.json # default
├── docs/
│   ├── ARCHITECTURE.md   # ADRs (AGENTS.md §9.2)
│   └── superpowers/specs/
└── AGENTS.md
```

### Key architecture rules

1. **`sh-core` is a library without gpui.** Its `Cargo.toml` does NOT declare `gpui`.
   The compiler enforces the AGENTS.md §3.2 separation: core never imports UI/platform code.
   Core tests run headless and fast.
2. **`sh-app` is the binary.** Everything touching GPUI, decode threads, and platform lives here.
   It depends on `sh-core`.
3. **Crate boundary:** `sh-app` consumes `sh-core` as a black box — calls `decode::load(path)`,
   receives `DecodedImage` (dimensions + RGBA bytes). Format internals stay inside core.
4. **`transform.rs` lives in core because it is pure math** — zoom/pan/fit with `f32`, no GPU.
   UI only applies the transform core computes. Fully testable (proptest per AGENTS.md §4.4).
5. **Theme hot reload logic lives in core** (`theme.rs`): parse + validate + change detection.
   `sh-app` only re-renders when core signals a change. UI never parses JSON.

### Suggested dependencies

- `sh-core`: `image`, `thiserror`, `serde` + `serde_json`, `natord` (or regex-based natural sort),
  dev: `proptest`, `tempfile`.
- `sh-app`: `gpui = "0.2.2"` (default features include Windows manifest), `sh-core`,
  `tracing` + `tracing-subscriber`, `rfd` (native file dialog).
  Drag & drop uses GPUI's native desktop `on_drop` — no extra crate.

---

## 3. App components and data flow

### Session state and components (GPUI)

```
App (root component)
├── state::Session        # current image, index, zoom, pan, fit_mode
├── state::ThemeStore     # active theme + hot reload (listens to core::theme)
├── ui::ImageViewer       # canvas: renders texture with the transform
├── ui::Overlay           # ephemeral overlays (filename, zoom %, position counter)
└── platform::handlers    # drag&drop, dialog, cli args
```

### Minimalist-pure behavior

- At rest: ONLY the image over theme background (Deep Neutral `#0d0d0f`).
- Mouse toward top edge → top overlay appears: `name.png · 4/23`.
- Mouse toward bottom edge → bottom overlay appears: `◀ 🔍 35% ▶` + controls.
- Overlays fade out after ~1.5s without mouse movement.
- Tab cycles: none → top overlay → bottom overlay → both.

### Image open flow (the hot path)

```
1. Origin: cli arg | drag&drop | dialog
   └─ platform receives PathBuf → core::navigation::resolve()

2. core::navigation::resolve(path)
   └─ scans directory, filters supported extensions, natural sort,
      computes position of the file
   └─ returns ImageList { paths: Vec<PathBuf>, current: usize }

3. sh-app dispatches decode to a worker thread (NEVER the frame loop)
   └─ worker: core::decode::load(path) -> Result<DecodedImage>
   └─ DecodedImage { width: u32, height: u32, rgba: Vec<u8> }
   └─ sends result back over a channel to the main thread

4. Main thread (GPUI):
   └─ converts DecodedImage → gpui texture/image source
   └─ stores in Session, requests re-render
   └─ core::transform::fit_to_window(img, viewport) -> Transform
```

### Decode workers — golden rule

- Never decode inside the frame loop. `std::thread` + `mpsc::channel` + `cx.spawn` to return
  to the main thread.
- One worker per operation, reused via channel — no spawn-per-image.
- While decoding: subtle placeholder (theme background + minimal spinner or none).
- Decode failure → `ShImagesError::Decode/Io` → clear error overlay (AGENTS.md §2.1:
  no panics in user-facing code).

### Navigation (prev/next)

- Arrows ◀/▶, keys ←/→, or clicks on screen edges (wheel stays zoom).
- Navigation is circular (AGENTS.md §3.1).
- The worker decodes the new image; the previous goes to the LRU slot for fast return.

### Zoom/pan math

```
core::transform:
  ZoomState { scale: f32, offset: Vec2<f32> }
  - zoom_at(cursor, delta) -> ZoomState        # wheel anchored to cursor
  - pan(delta) -> ZoomState
  - fit_to_window(img, viewport) -> ZoomState  # on open / double-click
  - toggle_fit_100() -> ZoomState
```

- Double-click toggles 100% ↔ fit.
- Wheel zoom anchors to cursor: the point under the cursor stays fixed.
- Zoom clamped between fit and 8x (testable limits in core).
- While zooming/panning, the name overlay hides; the % shows ephemerally.

### Event → action map

| Input | Action |
|---|---|
| ←/→ or ◀/▶ | prev/next |
| Wheel | zoom anchored to cursor |
| Drag | pan |
| Double-click | toggle 100%/fit |
| Tab | cycle overlays |
| F11 | fullscreen |
| Esc | exit fullscreen (or clear overlay) |
| Mouse toward edge | show corresponding overlay |

### Testing the flow

- core: unit tests ≥90% (circular nav with Unicode names, transform via proptest,
  theme with invalid JSON).
- app: smoke integration test — open real image, navigate, verify no crash.
  GPUI UI tested at logical-component level where feasible.

---

## 4. Cache, themes, config, errors

### Decode cache (V1: minimum viable)

Goal (AGENTS.md): prev/next with decoded neighbor < 16ms.

- **V1: LRU with 2 slots** (`core::cache`): current slot + adjacent slot (per nav direction).
- On decode, prefill the likely neighbor slot (current+1 if going →, current-1 if ←).
- Next navigation: if target is cached → instant (texture upload only);
  otherwise → worker decode with placeholder.
- Cache memory limit configurable (default ≤ 128MB of decoded pixels),
  keeps RAM under thresholds (<150MB with 4K).
- V1 has no thumbnails/grid → no thumbnail cache (V2 with grid).
- Thread safety: slots shared via `Arc<Mutex<...>>` or worker returns via channel and
  main decides cache insertion — implementation detail; core API stays pure/testable.

### Themes with hot reload

```
themes/
├── deep-neutral.json     # default (embedded via include_str!)
├── + 2 built-in more (V1 requires 3 per AGENTS.md §10.2):
│     "dark-clinical", "light-clean"
└── user themes: %APPDATA%\sh_images\themes\*.json (Windows)
```

**core::theme (pure, testable):**
- `parse(json: &str) -> Result<Theme, ShImagesError::Theme>` — schema + validation
  of colors/spacing/radii/typography.
- `discover(config_dir) -> Vec<PathBuf>` — detect user themes in config dir.
- Hot reload: `sh-app` watches the theme dir (polling ~1s or notify crate —
  implementation detail); on change:
  - re-parse JSON
  - valid → apply
  - invalid → KEEP last valid theme + error overlay ("invalid theme: line X, col Y").
    AGENTS.md §10.2 says "fall back to default", but keeping the last valid theme is
    less jarring; the binary default applies only when no valid theme has loaded.
- Schema tokens map directly to GPUI colors (`gpui::hsla`/`rgb`).

### Configuration (`state::settings` in sh-app)

- `settings.json` in config dir (`%APPDATA%\sh_images\settings.json` on Windows).
- V1 stores: `theme` selection, `last_dir` (optional to remember folder),
  cache memory limit, `show_hidden_files` (bool, default false).
- `core` defines the serialization schema (serde) — `sh-app` persists it.
- Tolerant read: corrupt JSON → defaults + warn (`tracing`, AGENTS.md §7.4). Never crash.
- Atomic write: write to `.tmp` + rename (AGENTS.md §4.2 atomic writes).

### Errors (single type)

```
ShImagesError  (core::errors, thiserror)
├── Decode(image::ImageError)
├── Io(std::io::Error)
├── UnsupportedFormat(String)
├── Config(String)
├── Theme(String)
└── Unknown(String)
```

- `sh-app` maps `ShImagesError` → user-facing overlay (never panic).
- Worker error → channel returns `Result`; main thread displays it.
- Logging: `tracing` levels (error!/warn!/info!/debug!) — AGENTS.md §7.4.
- V1: `unwrap()` only in tests; production always `?` + context.

Coverage targets (AGENTS.md §4.1): theme ≥90%, cache ≥90%, settings ≥85%.

---

## 5. Project bootstrap, milestones, acceptance criteria

### Bootstrap

1. Workspace cargo init (`sh-core` + `sh-app`), git init, `.gitignore`
   (target/, .superpowers/), first commit.
2. Dependencies as listed in §2.
3. Built-in theme JSONs in `themes/` + `include_str!` in core.

### Milestones (dependency order)

| # | Milestone | Verifiable deliverable |
|---|---|---|
| 1 | Workspace + scaffold | `cargo build` passes; `cargo clippy -D warnings` and `cargo fmt --check` clean; `cargo test` runs (may be empty). |
| 2 | core::errors + navigation | Tests: folder scan, extension filter, natural sort, circular nav, Unicode. ≥90% cover. |
| 3 | core::theme + built-in themes | Tests: valid/invalid parse, schema, colors, fallback. 3 theme JSONs. |
| 4 | core::transform | Tests + proptest: anchored zoom, pan, fit, clamps, limits. |
| 5 | core::decode + cache | Tests: supported formats, corrupt, missing, large. LRU 2 slots with memory limit. |
| 6 | sh-app: window + image | Open via CLI arg → image on screen with fit. No crash. |
| 7 | sh-app: navigation + workers | ←/→ and edge clicks navigate with async decode; cache accelerates neighbor. |
| 8 | sh-app: zoom/pan/double-click | Wheel anchors cursor, drag, 100%/fit. Ephemeral counter. |
| 9 | sh-app: minimalist overlays + Tab | Top/bottom overlays with fade, Tab cycle, auto-hide. |
| 10 | sh-app: drag&drop + dialog + settings | D&D and dialog opening; settings persistence with atomic writes; theme hot reload. |
| 11 | Final QA | AGENTS.md §11 checklist complete: clippy/fmt/test/build release, RAM measurement, cold-start benchmark, manual QA. |

### V1 acceptance criteria (from AGENTS.md, verifiable)

- `cargo check`, `clippy -- -D warnings`, `fmt --check`, `test` all green.
- Core coverage ≥90% (tarpaulin), app ≥80%.
- Cold start to interactive <200ms; open 1080p <50ms; 4K <150ms; cached nav <16ms.
- RAM idle <30MB; with 4K <150MB.
- Frame time <8ms (120fps) pan/zoom 4K.
- Zero panics in user paths (corrupt, missing, no-permission).
- No `unsafe` without `// SAFETY:`; no `unwrap()` in production.
- Themes: 3 built-in + hot reload + safe fallback.

### Risks and mitigations

| Risk | Mitigation |
|---|---|
| GPUI 0.2.2 API differs from Zed main examples | Verify real API via docs.rs before each UI milestone; iterate with real feedback |
| First GPUI compile is slow (blade/vk/wgpu) | Milestone 1 isolated to build only; use `cargo build -p sh-core` to iterate logic without recompiling GPU |
| Huge images (100MP) decode | Worker thread + cache memory limit; decode whole and warn if it exceeds |
| GPUI drag&drop may vary by platform | Windows-first: validate in milestone 10 with manual test |

---

## 6. Out of scope for V1

- Grid/thumbnail view (V2; thumbnail cache deferred)
- Crop mode, export, clipboard copy (later)
- GIF animation frames
- SVG rasterization
- Default-viewer registration / Windows registry
- Video playback (was GPUIX-era; not in AGENTS.md scope)
- macOS/Linux official support (code stays portable, untested)
- In-app settings panel (hand-edited JSON in V1, panel in later phase)

---

## 7. References

- AGENTS.md (project rules, QA, metrics, theme schema §10)
- GPUI 0.2.2 on crates.io / docs.rs
- Memory: previous GPUIX implementation discarded — reference for UX decisions only
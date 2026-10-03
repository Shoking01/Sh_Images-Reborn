# AGENTS.MD — Sh_Images

> Working rules for AI Agents contributing to Sh_Images
> Project: Native GPU-accelerated Image Viewer built with Rust + GPUI (Zed's framework)
> Version: 0.1.0 — read from `Cargo.toml` (`[workspace.package] version`), which is the single source of truth; do not duplicate or restate it here.
> Stack: Rust + GPUI (https://github.com/zed-industries/zed/tree/main/crates/gpui) — No JavaScript, no Electron, no web views, no runtime GC.

---

## 1. Project Context

Sh_Images is a native, GPU-accelerated image viewer built with **Rust + GPUI**: declarative UI rendered directly to the GPU through Metal (macOS), DirectX (Windows), or Vulkan (Linux). This document defines the constraints, QA procedures, quality metrics, and coding standards every agent must follow when contributing to the project.

**Core priorities (in order):**
1. **Performance**: 120fps UI, <50ms navigation, zero frame drops
2. **Efficiency**: <30MB RAM idle, <150MB with 4K image, minimal CPU
3. **Customization**: User-defined themes via JSON with hot reload
4. **Reliability**: No crashes, graceful error handling, type safety

---

## 2. Code Philosophy

### 2.1 Safety First
- **No `unsafe` without justification**: Every `unsafe` block must have a `// SAFETY:` comment explaining why it's sound. Prefer safe abstractions.
- **No `unwrap()` in production paths**: Use `?`, `ok_or()`, `expect()` with context, or `match`. `unwrap()` is acceptable only in tests or with proven invariants.
- **No panics in user-facing code**: Recover from errors gracefully. Use `Result<T, ShImagesError>` for all fallible operations.
- **Strict types**: Leverage Rust's type system. No `String` for paths (use `PathBuf`/`Path`), no `f64` for pixel coordinates (use `f32` or integer types).
- **Input validation**: Every public function must validate preconditions and return typed errors (see `errors.rs`).

### 2.2 Idiomatic Rust
- Follow the **Rust API Guidelines** and **Rust Style Guide**.
- Types: `PascalCase`. Functions/variables: `snake_case`. Constants: `SCREAMING_SNAKE_CASE`. Files: `snake_case.rs`.
- Run `cargo clippy -- -D warnings` before every commit. CI fails on warnings.
- Run `cargo fmt` and resolve all diffs before considering a task done.
- Prefer `&str` over `&String`, `&[T]` over `&Vec<T>`, `impl Into<PathBuf>` for path parameters.
- Document all public items with `///` doc comments. Explain the *why*, not just the *what*.
- Use `thiserror` for error types, `serde` for serialization, `tracing` for logging.

### 2.3 Performance Conscious
- **Zero-cost abstractions**: Prefer iterators over loops, avoid unnecessary allocations, use `SmallVec`/`ArrayVec` for small collections.
- **No blocking the frame loop**: All I/O and decoding must be async or on worker threads. Use `tokio` or `async-std` for async runtime.
- **Memory efficiency**: Use `Arc<T>` for shared data, `Cow<str>` for borrowed strings, avoid deep clones in hot paths.
- **Cache-friendly**: Structure data for cache locality. Use `Vec<T>` with contiguous storage over `HashMap` when iteration matters.
- **Profile before optimizing**: Use `cargo flamegraph`, `perf`, or `valgrind` to identify bottlenecks. Measure, don't guess.

---

## 3. Mandatory Code Structure

### 3.1 Module Organization

src/
├── main.rs           # Entry point only. Initializes App, opens window. Max ~50 lines.
├── app.rs            # Root component: global state, theme provider, key handling.
├── core/             # Pure business logic (no GPUI imports).
│   ├── navigation.rs # Folder scanning, sorting, circular navigation
│   ├── cache.rs      # LRU cache for thumbnails and decoded images
│   ├── decode.rs     # Image decoding abstraction (image-rs backends)
│   ├── crop.rs       # Crop math, coordinate transforms
│   └── theme.rs      # Theme parsing, validation, application
├── ui/               # GPUI components (depend on gpui crate).
│   ├── components/   # Reusable widgets (buttons, sliders, toolbars)
│   ├── views/        # Main views (grid, single image, crop overlay)
│   └── theme/        # Theme system integration
├── state/            # Global state management (theme store, settings)
├── workers/          # Background threads for decode/crop/export
└── platform/         # OS-specific code (Windows registry, clipboard, dialogs)

### 3.2 Separation of Concerns
- **`core/`**: Pure, testable logic with no side effects. Must not import `gpui`, `tokio`, or any UI/platform code.
- **`ui/`**: Presentation only. Reads state, renders elements, dispatches actions. No business logic.
- **`state/`**: Global state containers (theme, settings, cache). Uses `gpui::Global` for app-wide state.
- **`workers/`**: CPU-intensive work (decode, resize, encode). Communicates via channels.
- **`platform/`**: OS-specific implementations behind traits. Windows registry, macOS clipboard, etc.

### 3.3 Error Types
- Define a global error enum in `src/errors.rs`:
  ```rust
  use thiserror::Error;
  
  #[derive(Error, Debug)]
  pub enum ShImagesError {
      #[error("decode error: {0}")]
      Decode(#[from] image::ImageError),
      
      #[error("I/O error: {0}")]
      Io(#[from] std::io::Error),
      
      #[error("unsupported format: {0}")]
      UnsupportedFormat(String),
      
      #[error("config error: {0}")]
      Config(String),
      
      #[error("theme error: {0}")]
      Theme(String),
      
      #[error("unknown error: {0}")]
      Unknown(String),
  }
  
  pub type Result<T> = std::result::Result<T, ShImagesError>;
  ```
- Each module may define narrower error types that convert into `ShImagesError`.
- Never panic in production paths. Use `Result` everywhere.

## 4. Unit Tests — Mandatory Standards

### 4.1 Minimum Coverage
- Business logic (core/): ≥ 90% coverage.
- State management (state/): ≥ 80% coverage.
- Utilities: ≥ 85% coverage.
- UI components: Integration tests for critical flows.

### 4.2 What MUST Always Be Tested

| Component       | Required Tests                                                                                        |
| --------------- | ----------------------------------------------------------------------------------------------------- |
| `decode.rs`     | Decoding of every supported format; corrupt file handling; missing file handling; large file handling |
| `cache.rs`      | Insertion, LRU eviction, memory limit enforcement, hit/miss ratio, thread safety                      |
| `navigation.rs` | Circular folder navigation; extension filtering; natural sorting; Unicode filenames                   |
| `crop.rs`       | Coordinate transformation math; zoom limits; aspect ratio calculations                                |
| `theme.rs`      | JSON parsing; schema validation; color format validation; fallback behavior                           |
| `settings.rs`   | Serialization/deserialization; default values; version migration; atomic writes                       |

### 4.3 Test Style
- Use descriptive names: opening_corrupt_png_returns_error() instead of test_open_png().
- Use rstest or test-case for parameterized tests.
- Every test must be independent: no reliance on execution order or shared state.
- Use tempfile crate for I/O tests, and clean up after each test.
- Mock the filesystem when possible for faster, hermetic tests (use mockall or traits).

### 4.4 Property-Based Tests
- Use proptest for mathematical operations (crop transforms, color conversions).
- Use quickcheck for simple invariants (LRU cache behavior).

## 5. QA Procedures
### 5.1 Pre-Commit Checklist (Mandatory)
Before marking any task as "complete", the agent must verify:
  - [ ] `cargo check` passes with zero errors
  - [ ] `cargo clippy -- -D warnings` passes with zero warnings
  - [ ] `cargo fmt --check` passes
  - [ ] `cargo test` passes (all unit, integration, and doc tests)
  - [ ] `cargo build --release` succeeds
  - [ ] New public items have `///` doc comments
  - [ ] No `unsafe` without `// SAFETY:` justification
  - [ ] No `unwrap()`/`expect()` in production paths without proven invariant
  - [ ] No blocking operations on the main thread
  - [ ] Test coverage of new code ≥ 80%
  - [ ] Benchmarks show no regression (&gt; 5% degradation)
  - [ ] No TODOs or FIXMEs in new code

### 5.2 Manual QA (for UI features)
  - For each UI feature, perform these manual checks (document in the PR):
  - Functionality: Does it do what it should?
  - Edge cases: What happens with corrupt, empty, or very large files (100MP+)?
  - Cross-platform: Does it work on Windows, macOS, and Linux?
  - Accessibility: Is it usable with keyboard only? (Verify focus order and shortcuts.)
  - Performance: No perceptible lag? Check frame time histogram.
  - Memory: No leaks? Monitor RSS over 10 minutes of continuous use.

### 5.3 Code Review (Agent ↔ Agent)
  - Any change to core/ requires review from another agent.
  - Any change that modifies the architecture requires updating docs/ARCHITECTURE.md.
  - Reviews must verify: correct logic, adequate tests, documentation, performance implications, and adherence to this AGENTS.MD.

## 6. Quality Metrics and Thresholds
### 6.1 Automated Metrics (CI)
| Metric                  | Minimum Threshold | Target Threshold | Tool                     |
| ----------------------- | ----------------- | ---------------- | ------------------------ |
| Test coverage (total)   | 75%               | 85%              | `cargo tarpaulin`        |
| Test coverage (`core/`) | 85%               | 95%              | `cargo tarpaulin`        |
| Clippy warnings         | 0                 | 0                | `cargo clippy`           |
| Compiler warnings       | 0                 | 0                | `cargo check`            |
| Format conformance      | 100%              | 100%             | `cargo fmt --check`      |
| Build time (dev, cold)  | < 120s            | < 60s            | CI timer                 |
| Build time (release)    | < 5 min           | < 3 min          | CI timer                 |
| Binary size (release)   | < 25MB            | < 20MB           | `ls -lh target/release/` |
| Packaged installer size | < 10MB            | < 7MB            | Inno Setup output        |
- Note: GPUI statically links the renderer. Binary size is larger than typical Rust apps but smaller than Electron. Validate against release builds.
- **Baseline shifted with ADR-020.** The binary target was `< 15MB`, set when the
  framework was `gpui 0.2.2` and no component layer was present. Moving to
  `gpui-pre 0.3.7` to host `gpui-component` measured **10.10MB → 18.39MB**;
  `default-features = false` is a no-op (the crate declares no `default`
  feature) and `lto = "fat"` saves 0.21MB for 5.4x the build time, so 18.39MB
  is accepted rather than deferred. See ADR-020's Consequences.
- **What the user actually downloads is the installer, and it is small.** The
  Inno Setup script already runs `lzma2/max` with `SolidCompression=yes`; the
  18.39MB binary measures **6.51MB compressed (2.83x)**. The packaged-install
  row is the number that reflects download and install cost, and it is
  comfortably inside the target.
- Reference point, measured on the same machine: Chrome with one blank tab
  runs 12 processes at 661MB, and Discord (Electron) runs 6 at 662MB. The
  comparison that matters for a native app is that.

### 6.2 Performance Metrics (Benchmarks)
| Metric                                           | Maximum Threshold | Tool                           |
| ------------------------------------------------ | ----------------- | ------------------------------ |
| Cold start to interactive                        | < 200ms           | `cargo bench`                  |
| Open time (1080p image)                          | < 50ms            | `cargo bench`                  |
| Open time (4K image)                             | < 150ms           | `cargo bench`                  |
| Open time (8K image)                             | < 400ms           | `cargo bench`                  |
| Navigation latency (next image, cached)          | < 16ms            | `cargo bench`                  |
| Navigation latency (next image, decode required) | < 100ms           | `cargo bench`                  |
| Idle RAM usage                                   | < 80MB            | OS process monitor             |
| RAM with one 4K image                            | < 150MB           | OS process monitor             |
| RAM with 100 thumbnails                          | < 80MB            | OS process monitor             |
| Frame time (UI idle)                             | < 4ms (250fps)    | GPUI frame instrumentation     |
| Frame time (pan/zoom 4K)                         | < 8ms (120fps)    | GPUI frame instrumentation     |
| CPU usage (idle)                                 | < 2%              | OS process monitor             |
- **Baseline shifted with ADR-020, for the renderer and not the component
  layer.** The idle figures were measured against `gpui 0.2.2`; `gpui-pre`
  0.3.7 replaces the renderer with a `wgpu` backend, which costs memory and
  threads on its own. Measured on Windows 11, `sh-app.exe` release build, one
  process throughout:
  - idle RAM **54.7MB** with a one-image folder, **59.5MB** with six
  - CPU idle **1.87%** of one core over a 15s sample
  - **1 process, 24-25 threads**
  The old targets (`< 30MB`, `< 1%`) are not reachable under `gpui-pre` 0.3.7
  by any configuration, so they were recalibrated rather than left as a
  standard nothing can meet. Context for judging them: Chrome with one blank
  tab is 661MB across 12 processes, Discord 662MB across 6.
- Attribution between the framework move and `gpui-component` is **not yet
  measured** — it needs a `main`-line binary built against `gpui-pre` 0.3.7
  without the component layer. Until that exists, treat the split as unknown
  rather than assuming the kit is innocent or guilty.
- Frame-time and open-time rows are still unmeasured on this branch; they were
  unmeasured before it too.

### 6.3 Regressions
- Any regression > 10% in performance metrics blocks the merge.
- Any drop in test coverage blocks the merge.
- Any new clippy warning blocks the merge.
- Any increase > 5MB in binary size requires justification.

## 7. Agent Restrictions
### 7.1 Absolute Prohibitions
| Restriction                                      | Reason                                               |
| ------------------------------------------------ | ---------------------------------------------------- |
| ❌ No `unsafe` without `// SAFETY:` justification | Memory safety is priority #1                         |
| ❌ No `unwrap()`/`expect()` in production paths   | Panics kill UX and reliability                       |
| ❌ No blocking I/O on the main thread             | The app must feel fluid at all times                 |
| ❌ No deep clones in hot paths                    | Performance degradation                              |
| ❌ No `println!` in production                    | Use `tracing` with levels                            |
| ❌ No new dependencies without justification      | Every dependency is compile time + supply chain risk |
| ❌ No `TODO` without a ticket/issue               | Every TODO must have an associated issue             |
| ❌ No JavaScript, no WASM, no web tech            | This is a native Rust application                    |

## 7.2 Dependencies — Approval Process
- Before adding any crate to Cargo.toml:
- Verify no solution exists with current dependencies or std library.
- Evaluate: Is it maintained? > 500 downloads/month? Last commit < 6 months?
- Verify the license (must be compatible: MIT, Apache-2.0, BSD, etc.).
- Check compile time impact (cargo build --timings).
- Document the justification in a comment above the dependency in Cargo.toml:
- # image: Pure Rust image decoding, no C dependencies, supports all V1 formats
image = "0.24"

## 7.3 GPUI-Specific Rules
- Element tree is code: UI is built with method chaining, not markup. Keep trees readable with helper functions.
- Text requires explicit color: GPUI does not inherit color from parents. Always set .text_color() on text elements.
- Async by default: Use cx.spawn() for async operations. Never block the UI thread.
- Global state: Use cx.set_global() / cx.global::<T>() for app-wide state (theme, settings, cache).
- Window options: Set vsync: true, show_menubar: false for image viewer aesthetic.
- No DOM APIs: This is not a web app. No document, no window, no fetch.

## 7.4 Logging and Observability
- Use tracing (structured logging) instead of println!.
- Levels:
  - error!: Errors affecting functionality (image cannot open, crash avoided).
  - warn!: Recoverable situations (unsupported format, corrupt EXIF).
  - info!: Significant user events (image opened, folder changed, theme applied).
  - debug!: Development details (cache hits/misses, decode timings).
  - trace!: Very detailed info (per-frame UI events, element tree diffs).
- In release builds, the minimum level must be info (set via RUST_LOG or config).

## 8. Mandatory Integration Tests

### 8.1 Critical Flows
- Each of these flows must have an integration test:
  - Open Flow:
    - Launch app → File dialog → Select image → Render → Close
  - Navigation Flow:
    - Open folder with N images → Navigate forward/backward → Verify correct order and caching
  - Zoom/Pan Flow:
    - Open image → Zoom in → Pan → Zoom out → Fit to window → Verify transformations
  - Error Flow:
    - Open corrupt image → Verify no crash → Verify visible error message
  - Config Flow:
    - Modify setting → Close app → Reopen → Verify persistence
  - Theme Flow:
    - Load custom theme JSON → Verify colors applied → Edit theme file → Verify hot reload

### 8.2 Test Fixtures
- Store test images in tests/fixtures/.
- Include: valid PNG, valid JPEG, valid GIF, valid WebP, corrupt file (random bytes with .png extension), empty file, 1x1 pixel image, 10000x10000 large image.
- Test images must be small (< 100KB) except the large image test.

## 9 Mandatory Documentation

### 9.1 Doc Comments
/// Loads an image from the filesystem asynchronously.
///
/// # Arguments
///
/// * `path` - Absolute path to the image file.
///
/// # Returns
///
/// The decoded image dimensions and pixel data.
///
/// # Errors
///
/// Returns `ShImagesError::Decode` if the format is unsupported or corrupt.
/// Returns `ShImagesError::Io` if there are filesystem read problems.
///
/// # Example
///
/// ```no_run
/// let img = load_image(Path::new("tests/fixtures/sample.png")).await?;
/// assert_eq!(img.dimensions(), (1920, 1080));
/// ```
pub async fn load_image(path: &Path) -> Result<DecodedImage> {
    // ...
}

### 9.2 Architectural Decisions
- Every significant architectural decision (framework choice, caching strategy, theme system, etc.) must be documented in docs/ARCHITECTURE.md using the ADR (Architecture Decision Record) format:
  - Context
  - Decision
  - Consequences
  - Alternatives considered

## 10. Theme System Requirements

### 10.1 Theme Format
- Themes are JSON files with schema validation:  
```json
{
  "name": "Neon Nights",
  "author": "username",
  "version": 1,
  "colors": {
    "background": "#0a0a1a",
    "surface": "#12122a",
    "text": "#e0e0ff",
    "accent": "#00ffff"
  },
  "spacing": { "xs": 4, "sm": 8, "md": 16, "lg": 24 },
  "radii": { "sm": 2, "md": 6, "lg": 12 },
  "typography": {
    "family": "Inter",
    "sizes": { "caption": 11, "body": 14, "title": 18 }
  }
}
```

### 10.2 Theme Requirements
- Hot reload: Changes to theme files apply without restart.
- Validation: Invalid themes fall back to default with error message.
- Distribution: Users can share themes as single JSON files.
- Discovery: App scans ~/.config/sh_images/themes/ for user themes.
- Built-in: At least 3 high-quality themes included in binary.


## 11. Final Checklist Before Delivery
- Before considering a task or sprint complete:
### Code Verification
- [ ] All new code has unit tests
- [ ] Tests pass locally (`cargo test`)
- [ ] Tests pass in CI
- [ ] Clippy passes with zero warnings
- [ ] Format is correct (`cargo fmt --check`)
- [ ] No `unsafe` without justification
- [ ] No `unwrap()` in production paths
- [ ] Doc comments on public items

### Quality Verification
- [ ] Test coverage ≥ 80% for new code
- [ ] Benchmarks show no regression
- [ ] No memory leaks (verify with valgrind or OS tools)
- [ ] App does not crash with corrupt files
- [ ] Binary size increase justified

### Performance Verification
- [ ] Frame time &lt; 8ms under load
- [ ] RAM usage within thresholds
- [ ] No blocking operations on UI thread
- [ ] CPU usage &lt; 1% at idle

### Documentation Verification
- [ ] `docs/ARCHITECTURE.md` updated if applicable
- [ ] CHANGELOG.md updated
- [ ] README.md updated if usage changed
- [ ] Theme schema documented if modified

### UX Verification
- [ ] Feature works with keyboard
- [ ] Feature works with mouse/trackpad
- [ ] Error messages are clear to the user
- [ ] No perceptible lag
- [ ] Theme changes apply instantly

## 12. Quick Reference Commands
# Verify everything before commit
cargo check && cargo clippy -- -D warnings && cargo fmt --check && cargo test

# Run tests with coverage
cargo tarpaulin --out Html

# Run benchmarks
cargo bench

# Development (debug build with logging)
RUST_LOG=debug cargo run

# Type check only
cargo check

# Production build (optimized)
cargo build --release

# Check binary size
ls -lh target/release/sh_images

# Profile performance
cargo flamegraph --bin sh_images

# Clippy with all features
cargo clippy --all-targets --all-features -- -D warnings

- Remember: Performance over features. It is better to deliver fewer, perfectly optimized features than many slow ones. Sh_Images must be an example of idiomatic, safe, performant Rust on a truly native GPU stack. Every millisecond and every megabyte matters.

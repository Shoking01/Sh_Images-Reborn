# Spike: gpui-component top bar

**Goal**: modernise the UI's look and make the theme more customizable, by
adopting gpui-kit's component layer on the top bar first.

**Status**: complete and verified. `main` untouched.

## What changed

| Surface | Before | After |
| --- | --- | --- |
| UI framework | `gpui 0.2.2` (crates.io) | `gpui-pre 0.3.7` (upstream Zed snapshot behind gpui-kit) |
| Top-bar controls | 6 hand-built `div`s | 6 `gpui_component::button::Button` |
| Hover / pressed | hand-tuned `btn_bg` / `btn_hover` / `btn_pressed` | the kit's own states, fed by the theme bridge |
| Icon set | 12 app SVGs | app SVGs + the kit's 1830-SVG Lucide bundle |
| Theme source | app JSON | app JSON, projected onto the kit's global `Theme` |

## Verification

| Gate | Result |
| --- | --- |
| `cargo test --workspace` | 519 passed / 0 failed (514 baseline + 5 new) |
| `cargo check --workspace --all-targets` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean |
| `cargo build --release` | 18.3 MB (budget 25 MB) |
| Visual | captured the running app; buttons render with the app's theme, gear visible |

Zero test assertions were edited. The harness contracts (`topbar-back`,
`topbar-sort`, `("grid-size", n)`, `topbar-crop`) are unchanged, which is why
the layout tests pass untouched.

## The one thing that had to be built: `kit_theme.rs`

gpui-component components read `cx.global::<Theme>()`, not the app's colors:

```rust
pub fn new(id: impl Into<ElementId>) -> Self { ... }
// color/foreground/hover/active all seeded from cx.theme()
```

Adopting components without a bridge leaves **two sources of truth** — the JSON
theme drives every `div()` while the kit global drives every component, so
editing a theme file restyles half the UI and leaves the rest stale.

`kit_theme.rs` projects the app's four JSON colors onto the slots the top bar
reads, and runs at startup, on theme switch, and on hot reload. The JSON file
stays authoritative.

Hover and pressed are derived from the theme's own surface toward its text at
0.10 / 0.20 in gamma-encoded sRGB — the same arithmetic and ratios as the tint
they replace, so the visual weight is unchanged.

## gpui-component API facts (verified by compile probe, not by the website)

The site's examples do not compile as written:

| Site shows | Reality |
| --- | --- |
| `Button::new(cx)` | `Button::new(id: impl Into<ElementId>)` — the id goes IN the constructor |
| — | `on_click` takes `Fn(&ClickEvent, &mut Window, &mut App)`, not `Context::listener` |
| — | `selected` (paints) and `toggled` (a11y only) live on `Selectable`, which must be in scope |
| — | `with_size` lives on `Sizable`, also needing an import. An icon-only button does NOT collapse without it — it sizes itself to `size_8` |
| — | `ThemeColor` slots are FLAT in Rust (`secondary_foreground`) though nested in the kit's JSON |
| — | **`Application::with_assets` REPLACES the registered source; it does not compose.** A second call silently disables the first |

`gpui_component::IconName` is a small subset; the 1830-icon Lucide catalog is
`gpui_kit_assets::IconName`. `Scissors` and `Settings` are not in the subset.

## The invisible-Crop bug, and what it really was

Symptom: in the Viewer, the Crop control reserved its layout space and painted
nothing. Every test passed — no assertion reads pixels.

| Step | Evidence |
| --- | --- |
| Gear sits where Crop should be | Gear ends at x=947 in Viewer vs x=987 in Grid — a 40px shift, exactly 32px of Crop plus its 8px gap |
| Crop occupies space, paints nothing | Scanning the region it owns (x950-1006, y34-72) returned **0** non-background pixels; the same region in Grid returned 208 |
| The icon file exists | `icons/scissors.svg` is present in both the kit's catalog and the app's own `assets/icons/` |
| Root cause | `with_assets` replaces rather than composes, so the second call left only `gpui_kit_assets::Assets` alive. That bundle embeds **104** icons — `arrow-left` and `settings` are in it, `scissors` is not — and it answered every path `AppAssets` owns with `Err`. |

The kit returning `Err` for a miss is its normal "no such file" answer, but
GPUI paints an empty `svg` for it: no panic, no log line, no test failure.
Every one of the app's 12 icons was dead at that point, not just Crop — the
others simply had no visible control in the captured frame.

Fix: `AppAssets` owns the app's SVGs and falls back to the kit's bundle, and
`main` registers that one source.

**Detection method worth keeping:** comparing the same pixel row between two
views of the same bar. The element that moved told us the size of the element
that did not paint.

## Aesthetic probe: what the platform can and cannot do

Asked whether the bar could be prettier — transparency, custom hovers. Measured
rather than assumed, because two of the three answers were not what the API
names suggest.

| Capability | Verdict | Evidence |
| --- | --- | --- |
| Window backdrop material | **wired up but does nothing here** | `WindowOptions::window_background` really is implemented on Windows (`gpui-pre-windows/src/window.rs`): `Blurred` → `ACCENT_ENABLE_ACRYLICBLURBEHIND`, `MicaBackdrop` → `dwm_set_window_composition_attribute(hwnd, 2)`. But with a saturated window placed directly behind the app, the bar sampled `15151A` and the grid `101014` — neither picked up the backdrop. It does not reach the wgpu swapchain. Left `Opaque`. |
| Per-element backdrop blur | **does not exist** | `blur_radius` lives only on `BoxShadow` (`gpui/src/style.rs`). There is no CSS-style `backdrop-filter`, so a "frosted" bar over an image can only ever be plain alpha — the image shows through unblurred. |
| Component hover / pressed states | **yes, and they were broken** | See below. |

### The resting chip was invisible, and the reason was arithmetic

Mapping the chip to the app's raw `surface` looked right on paper. Rendered, the
bar is that same `surface` at 0.72 alpha over the app background, so the chip
landed **2/255 below the bar it sits on**:

| | before | after |
| --- | --- | --- |
| bar | `15151A` | `15151A` |
| resting chip | `17171D` | `25252B` |
| hairline | — | `2D2D32` |
| grid | `101014` | `101014` |

So the resting chip is lifted toward the theme's text, and the hover ladder is
spaced for headroom above it (`0.07 / 0.16 / 0.28`). Named constants, because
these are tuning knobs, not magic numbers.

### Two bugs this probe uncovered

**1. `ThemeColor::default()` is not a palette.** Every field is
`{h:0, s:0, l:0, a:0}` — fully transparent black. The bridge built its result
from it, which meant the ~124 slots it does not map were left *transparent*, so
any kit component outside the top bar would have painted nothing at all, with no
error anywhere. The bar looked perfect, which is how this class of bug hides.
Fixed by mapping **over** the kit's resolved palette instead of over `default()`,
and locked with `unmapped_slots_survive_the_mapping`.

**2. `toggled()` does not paint.** The density segments called `.toggled(active)`
alone, on a comment claiming it "paints the active segment". It only announces
the pressed state to assistive tech. Measured: the resting and active segments
both painted `25252B` — the active preset was correct for screen readers and
invisible for everyone else. `.selected(active)` is what paints; the sort chip
and crop toggle already had both.

### A third thing worth knowing

`Button::bg_color` resolves its resting fill from `theme.tokens.button`, never
from `theme.colors.button`. `Theme::update` reconciles the two, but that is the
kit's code, not ours — so a mapping tested only against `ThemeColor` passes while
the component keeps painting kit defaults. `mapped_colors_reach_the_tokens_the_components_read`
asserts the end state the component actually observes.

## Known scope boundaries

- Only the top bar uses kit components. The grid, viewer, filmstrip and settings
  panel still build `div`s, which is why the theme bridge maps only the slots
  those controls read rather than all 134. It now maps over the kit's own
  palette instead of over `ThemeColor::default()`, so unmapped slots keep real
  colors rather than becoming transparent.
- `taffy` moves 0.9.0 → 0.13.0. Harmless here: all 82 `debug_bounds` calls and
  every layout assertion passed unchanged.
- The app's Windows icon still works; `WM_GETICON` returns 0 on both `main` and
  this branch because the icon lives on the window class.

## Not committed

The work is uncommitted in the worktree by design — commit or discard is a
decision for the maintainer.
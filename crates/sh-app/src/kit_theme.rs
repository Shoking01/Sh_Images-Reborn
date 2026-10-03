//! Bridge between Sh Images' own JSON theme and gpui-component's global
//! [`Theme`].
//!
//! WHY THIS FILE EXISTS
//!
//! gpui-component components do not read the app's colors. Each one starts
//! from `cx.theme()` â€” the kit's own `Theme` global, installed as
//! `cx.global::<Theme>()` (see `Theme::global` in the kit). So the moment a
//! button in the top bar is a kit `Button` rather than a hand-built `div`, it
//! paints from that global instead of from `App::theme_store`.
//!
//! That would leave two sources of truth: the JSON theme file drives every
//! `div()` the app still builds, and the kit global drives every component.
//! Editing the theme file would then change half the UI and leave the other
//! half stale â€” the exact failure the hot-reload watcher is there to prevent.
//!
//! This module removes the second source of truth: it projects the app's
//! four-color JSON theme onto the subset of `ThemeColor` that the top bar's
//! components actually consume. Everything stays driven by the JSON file, so
//! custom themes keep working unchanged and hot reload keeps one path.
//!
//! Only the fields a top-bar control reads are mapped. The kit ships 134
//! colors; mapping all of them would be noise for an image viewer whose own
//! theme has four, and every unmapped field falls back to the kit default,
//! which is a coherent light/dark pair of its own.

use gpui::Hsla;
use gpui_component::theme::{Theme, ThemeColor, ThemeMode};
use sh_core::theme::Theme as AppTheme;

/// Corner radius applied to every kit component.
///
/// The kit's own default is tuned for its web-origin design system and reads
/// as almost-square at bar density. A 6px radius is the smallest step that
/// makes the top-bar chips read as controls rather than as text with a
/// background, and it matches the radius the app's own hand-built controls
/// (`settings_panel` rows) already use, so migrating a control to the kit does
/// not visibly change its shape.
const KIT_RADIUS_PX: f32 = 6.0;

/// How far the resting control chip is blended toward the theme's text color.
///
/// This is the smallest of the three steps on purpose. Its only job is to make
/// a control distinguishable from the bar it sits on at rest; the hover and
/// pressed steps need room above it, so a large resting lift would flatten the
/// whole ladder into one indistinguishable surface.
const CHIP_RESTING_LIFT: f32 = 0.07;

/// Blend toward `fg` for the pointer-over state. Chosen to be several times the
/// resting lift so a hover is unmistakable even on a low-contrast theme.
const CHIP_HOVER_LIFT: f32 = 0.16;

/// Blend toward `fg` for the pressed state â€” the top of the ladder, kept short
/// of the text color itself so a pressed chip still reads as a surface.
const CHIP_ACTIVE_LIFT: f32 = 0.28;

/// Project the active app theme onto the kit's global `Theme`.
///
/// Called on startup and on every hot-reload apply, so the kit global and the
/// JSON theme never drift. `parse` is injected because the app already has a
/// hex parser with the fallback chain its `div()` code uses â€” reusing it keeps
/// one definition of "what happens when a color in the JSON is unparseable"
/// instead of two that could disagree.
///
/// `mode` is passed explicitly rather than inferred from luminance: the app
/// already knows which theme file is active, and its built-in themes declare
/// their own intent. Inferring would make a mid-gray custom theme flip the
/// whole component set to light or dark based on a rounding decision.
pub fn sync_kit_theme(cx: &mut gpui::App, app_theme: &AppTheme, mode: ThemeMode) {
    // The kit's globals must exist before a component reads them. `Theme::change`
    // below assumes both are installed.
    if !cx.has_global::<Theme>() {
        crate::kit_theme::init_kit(cx);
    }
    // `change` first, on purpose: it loads the kit's registered palette for this
    // mode, which is the base the mapping below is applied over. Doing it the
    // other way round would leave the mapping with nothing to inherit from.
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        theme.colors = kit_colors_for(theme.colors, &app_theme.colors);
        theme.mode = mode;
        theme.radius = gpui::px(KIT_RADIUS_PX);
    });
}

/// Idempotent kit initialization. `gpui_component::init` installs the kit's
/// globals and action handlers; several of those `init` functions assert on
/// being called once, so this must only run when the globals are absent.
pub fn init_kit(cx: &mut gpui::App) {
    gpui_component::init(cx);
}

/// The kit's globals must exist before any component reads them, which is at
/// element-construction time rather than at render time. Called from `App::render`
/// so a bare `#[gpui::test]` gets a working kit for free, and from `main` at
/// startup so the first frame never constructs a component against an
/// uninitialized global.
pub fn ensure_kit_initialized(cx: &mut gpui::App) {
    if !cx.has_global::<Theme>() {
        init_kit(cx);
    }
}

/// The pure mapping, separated from the global write so the role assignment is
/// unit-testable without an `App`.
///
/// `base` is the palette to map ON TOP OF, and at runtime that is the kit's own
/// resolved theme for the active mode â€” never `ThemeColor::default()`.
///
/// That distinction is load-bearing, and it was a real bug before this signature
/// existed. `ThemeColor::default()` is not a palette: every one of its fields is
/// `{ h: 0, s: 0, l: 0, a: 0 }`, i.e. fully transparent black. Building the
/// result from it therefore mapped the ten-odd slots below and left the other
/// ~124 as transparent â€” so any kit component outside the top bar would have
/// painted nothing at all, with no error anywhere. The bar happened to look
/// right, which is exactly how that class of bug hides.
///
/// `ThemeColor` has 134 slots, so this mutates the base rather than spelling all
/// of them out: unmapped fields keep the kit's own coherent values.
///
/// The slots are FLAT (`secondary_foreground`, `button_hover`), even though the
/// kit's `default-theme.json` nests them (`secondary.foreground`,
/// `button.hover`) — the JSON nesting is flattened on deserialization. Read off
/// the struct, not the JSON, or every assignment fails to compile.
fn kit_colors_for(mut kit: ThemeColor, colors: &sh_core::theme::ThemeColors) -> ThemeColor {
    let background = parse(&colors.background);
    let surface = parse(&colors.surface);
    let text = parse(&colors.text);
    let accent = parse(&colors.accent);

    // The page behind everything, and the bar's own text.
    kit.background = background;
    kit.foreground = text;

    // The control surface: a raised chip on the page. `secondary` is what a
    // plain `Button` paints, so this is what keeps the top bar legible against
    // its own background.
    //
    // It is deliberately NOT the raw surface color. The bar paints the surface
    // at 0.72 alpha over the app background, which lands the bar itself about
    // 2/255 above that background â€” so a chip painted in the raw surface
    // measured 2/255 *below* the bar it sits on, i.e. invisible. Measured on a
    // rendered build, not reasoned about. The resting chip is lifted a step
    // toward the text so a control reads as a control at rest, not only once
    // the pointer is on it.
    kit.secondary = mix_toward(surface, text, CHIP_RESTING_LIFT);
    kit.secondary_foreground = text;
    // Hover and active keep deriving from the same surface so the ladder stays
    // theme-adaptive and monotonic. The steps are spaced further apart than the
    // hand-calibrated 0.10 / 0.20 they replace, because a hover now has to be
    // clearly distinct from BOTH the lifted resting chip and the pressed
    // state; 0.10 against a 0.07 resting chip put the first hover barely 3/255
    // above where the pointer already was, which reads as nothing.
    kit.secondary_hover = mix_toward(surface, text, CHIP_HOVER_LIFT);
    kit.secondary_active = mix_toward(surface, text, CHIP_ACTIVE_LIFT);

    // The `button_*` slots are the same control family seen from the Button's own
    // defaults. `Button::bg_color` resolves the resting fill from
    // `theme.tokens.button` (not `theme.colors.button`), but `Theme::update`
    // reconciles the two, so writing the colors here is what reaches the
    // component. They must agree with the pair above or the Button family would
    // paint a different chip than `secondary`.
    kit.button = mix_toward(surface, text, CHIP_RESTING_LIFT);
    kit.button_foreground = text;
    kit.button_hover = mix_toward(surface, text, CHIP_HOVER_LIFT);
    kit.button_active = mix_toward(surface, text, CHIP_ACTIVE_LIFT);

    // The active/selected state: the open sort menu, the active density
    // segment. The kit paints `accent` there.
    kit.accent = accent;
    kit.accent_foreground = parse(&colors.on_accent);

    // Semantic slots the app now declares in its own theme file. These are the
    // ones that would otherwise fall through to the kit's defaults, which do not
    // belong to the user's theme — the exact split-source-of-truth problem this
    // module exists to prevent, one component layer out.
    kit.muted = parse(&colors.background);
    kit.muted_foreground = parse(&colors.muted_text);
    kit.border = parse(&colors.border);
    kit.ring = parse(&colors.ring);
    // The kit names these `danger*`, not `destructive*`. The whole hover/active
    // family is mapped because a destructive control that only has an idle color
    // would light up in the kit's neutral on hover — the same class of mismatch
    // the `button_*` mapping exists to prevent.
    let danger = parse(&colors.danger);
    kit.danger = danger;
    kit.danger_foreground = parse(&colors.on_accent);
    kit.danger_hover = mix_toward(danger, text, 0.12);
    kit.danger_active = mix_toward(danger, text, 0.24);
    kit.button_danger = danger;
    kit.button_danger_foreground = parse(&colors.on_accent);
    kit.button_danger_hover = kit.danger_hover;
    kit.button_danger_active = kit.danger_active;

    // The bar's own surface, for the kit's bar-aware components. `title_bar`
    // takes the declared `elevated` rather than `surface` so a bar-aware
    // component lands one step above a panel instead of flush with it.
    kit.title_bar = parse(&colors.elevated);
    kit.title_bar_border = parse(&colors.border);
    kit.window_border = parse(&colors.border);

    kit
}

/// Blend `bg` toward `fg` by `ratio`, in gamma-encoded sRGB, and return it as an
/// opaque `Hsla`.
///
/// Not named `hover_tint` any more: it now builds the whole resting / hover /
/// pressed ladder, and a function called `hover_tint` that also produces the
/// resting chip is a name that lies to whoever tunes the constants above.
///
/// Deliberately the same naive channel mix the app's own `hover_tint` used, in
/// the same RGBA space, so the steps reproduce the visual weight the
/// hand-tuned tint had. Switching to linear-light blending here would be "more
/// correct" and would change how strong every step reads.
fn mix_toward(bg: Hsla, fg: Hsla, ratio: f32) -> Hsla {
    let b: gpui::Rgba = bg.into();
    let f: gpui::Rgba = fg.into();
    let mix = |x: f32, y: f32| x + (y - x) * ratio;
    gpui::Rgba {
        r: mix(b.r, f.r),
        g: mix(b.g, f.g),
        b: mix(b.b, f.b),
        a: 1.0,
    }
    .into()
}

/// Parse an app-theme color, falling back to the kit's own default when the
/// JSON carries something unusable.
///
/// The fallback is deliberately opaque rather than one of the app's hex
/// constants: a theme whose colors are all unparseable has already failed
/// validation in `sh-core`, so by the time a `Theme` reaches this function the
/// hexes are well-formed. The fallback only guards a hand-constructed `Theme`
/// in a test, and borrowing the kit's default keeps that case visually
/// coherent instead of inventing a gray.
fn parse(hex: &str) -> Hsla {
    crate::app::parse_hex(hex).unwrap_or_else(|| ThemeColor::default().primary)
}

#[cfg(test)]
mod tests {
    use super::{kit_colors_for, parse, AppTheme, KIT_RADIUS_PX};
    use gpui::Hsla;
    use gpui_component::theme::{Theme, ThemeColor, ThemeMode};

    /// The four-color starting point a theme file written against the original
    /// schema provides, with every optional slot derived exactly as production
    /// derives them.
    ///
    /// Building fixtures this way rather than spelling out ten slots means these
    /// tests also exercise `ThemeColors::with_derived_defaults`, so a derivation
    /// that produces an unusable color fails here instead of only showing up on
    /// screen.
    fn four_colors(
        background: &str,
        surface: &str,
        text: &str,
        accent: &str,
    ) -> sh_core::theme::ThemeColors {
        sh_core::theme::ThemeColors {
            background: background.into(),
            surface: surface.into(),
            text: text.into(),
            accent: accent.into(),
            ..Default::default()
        }
        .with_derived_defaults()
    }

    #[test]
    fn parse_accepts_valid_hex_and_falls_back_on_garbage() {
        let valid = parse("#c8c8c8");
        assert!(valid.a > 0.0, "a valid hex must produce an opaque color");

        let garbage = parse("not-a-color");
        let fallback = ThemeColor::default().primary;
        assert_eq!(
            format!("{:?}", garbage),
            format!("{:?}", fallback),
            "unparseable hex must fall back to the kit default, not panic"
        );
    }

    /// The four JSON colors must land on the four kit slots the top bar reads,
    /// in the right roles. This is the whole contract of the bridge: swapping
    /// two of them would render the top bar surface-on-background, and no
    /// compile error or test elsewhere would catch it.
    #[test]
    fn json_colors_land_on_the_expected_kit_slots() {
        let colors = four_colors("#101010", "#202020", "#f0f0f0", "#00ffff");
        let kit = kit_colors_for(ThemeColor::default(), &colors);

        let bg: Hsla = parse(&colors.background);
        let text: Hsla = parse(&colors.text);
        let accent: Hsla = parse(&colors.accent);

        assert_eq!(kit.background, bg, "background -> background");
        assert_eq!(kit.foreground, text, "text -> foreground");
        assert_eq!(kit.secondary_foreground, text, "text labels on a chip");
        // The `button_*` family must track `secondary`, or the Button component
        // would paint a different chip than `secondary` describes.
        assert_eq!(kit.button, kit.secondary, "button family matches secondary");
        assert_eq!(kit.button_foreground, text, "button labels match");
        assert_eq!(kit.accent, accent, "accent -> accent");
    }

    /// The resting chip must be visibly lighter than the bar it sits on.
    ///
    /// This is the one assertion that exists because a rendered build
    /// contradicted the obvious design: mapping the chip to the raw surface
    /// looked correct on paper and measured 2/255 below the translucent bar â€”
    /// invisible. Asserting the lift keeps a later "simplification" back to the
    /// raw surface from silently shipping an unreadable toolbar.
    #[test]
    fn resting_chip_is_lifted_above_the_bare_surface() {
        let colors = four_colors("#101010", "#202020", "#f0f0f0", "#00ffff");
        let kit = kit_colors_for(ThemeColor::default(), &colors);
        let surface = parse(&colors.surface);

        assert!(
            kit.secondary.l > surface.l,
            "resting chip must sit above the bare surface, else it vanishes into the bar"
        );
        assert!(
            kit.secondary.l < kit.secondary_hover.l,
            "the resting lift must leave headroom for hover"
        );
    }

    /// The hover and pressed states must be derived from the chip surface, not
    /// left at the kit's defaults: a custom theme's controls would otherwise
    /// lighten toward the kit's neutral gray on hover instead of toward the
    /// theme's text, which is what the hand-tuned tint this replaced did.
    #[test]
    fn hover_and_pressed_derive_from_the_theme_not_the_kit() {
        let colors = four_colors("#101010", "#202020", "#f0f0f0", "#00ffff");
        let kit = kit_colors_for(ThemeColor::default(), &colors);
        let text = parse(&colors.text);

        // Idle < hover < pressed, each a step toward the text.
        let as_l = |c: Hsla| c.l;
        assert!(
            as_l(kit.secondary_hover) > as_l(kit.secondary),
            "hover must lighten off the idle surface"
        );
        assert!(
            as_l(kit.secondary_active) > as_l(kit.secondary_hover),
            "pressed must lighten further than hover"
        );
        assert!(
            as_l(kit.secondary_hover) < as_l(text),
            "hover must stay short of the text color"
        );
        assert_eq!(
            kit.secondary_hover.a, 1.0,
            "hover must be opaque so it reads as a solid chip"
        );
        assert_eq!(kit.button_hover, kit.secondary_hover);
        assert_eq!(kit.button_active, kit.secondary_active);
    }

    /// Two custom themes must produce two different global colors. If the
    /// bridge ever stopped writing a slot, both themes would render
    /// identically â€” the failure the hot-reload watcher exists to prevent.
    #[test]
    fn distinct_themes_produce_distinct_kit_colors() {
        let a = four_colors("#000000", "#111111", "#ffffff", "#ff0000");
        let b = four_colors("#ffffff", "#eeeeee", "#000000", "#0000ff");
        let ka = kit_colors_for(ThemeColor::default(), &a);
        let kb = kit_colors_for(ThemeColor::default(), &b);

        for (field, (x, y)) in [
            ("background", (ka.background, kb.background)),
            ("foreground", (ka.foreground, kb.foreground)),
            ("secondary", (ka.secondary, kb.secondary)),
            (
                "secondary_foreground",
                (ka.secondary_foreground, kb.secondary_foreground),
            ),
            ("button", (ka.button, kb.button)),
            ("accent", (ka.accent, kb.accent)),
            ("secondary_hover", (ka.secondary_hover, kb.secondary_hover)),
            ("title_bar", (ka.title_bar, kb.title_bar)),
        ] {
            assert_ne!(
                format!("{:?}", x),
                format!("{:?}", y),
                "{field} did not change between themes"
            );
        }
    }

    #[test]
    fn border_is_a_translucent_version_of_the_text_color() {
        let text = parse("#ffffff");
        let border = text.alpha(0.18);
        assert!(border.a < text.a, "border must be more transparent");
        assert!(border.a > 0.0, "border must still be visible");
    }

    fn probe_theme() -> AppTheme {
        AppTheme {
            name: "probe".into(),
            author: "probe".into(),
            version: 1,
            colors: four_colors("#101010", "#202020", "#f0f0f0", "#00ffff"),
            spacing: sh_core::theme::ThemeSpacing {
                xs: 4,
                sm: 8,
                md: 16,
                lg: 24,
            },
            radii: sh_core::theme::ThemeRadii {
                sm: 2,
                md: 6,
                lg: 12,
            },
            typography: sh_core::theme::ThemeTypography {
                family: "Inter".into(),
                sizes: sh_core::theme::ThemeSizes {
                    caption: 11,
                    body: 14,
                    title: 18,
                },
            },
            interaction: sh_core::theme::ThemeInteraction::default(),
        }
    }

    /// The end-to-end contract: what the bridge writes on `ThemeColor` must be
    /// observable on `ThemeTokens`, because that is the struct the components
    /// actually read.
    ///
    /// `Button::bg_color` resolves its resting fill from `theme.tokens.button`
    /// and its hover from `theme.tokens.button_hover` â€” never from
    /// `theme.colors`. The two are kept in step by `Theme::update`'s internal
    /// reconcile step, which is the kit's code and not ours. Every other test
    /// in this file asserts on `kit_colors_for`'s return value, so all of them
    /// would keep passing even if that propagation silently stopped and the
    /// toolbar went back to painting kit defaults while the app's own `div()`s
    /// stayed correctly themed â€” which is exactly the split-source-of-truth
    /// this module exists to prevent.
    /// Slots the bridge does not own must come through untouched.
    ///
    /// This guards the bug that made every unmapped slot transparent: the
    /// mapping used to start from `ThemeColor::default()`, which is not a
    /// palette but 134 copies of fully transparent black. A component outside
    /// the top bar would then paint nothing, silently, while the bar looked
    /// perfect. Applying the mapping over a non-default base is the fix, and
    /// this is the assertion that keeps it fixed.
    #[test]
    fn unmapped_slots_survive_the_mapping() {
        // Slots the bridge has no business touching.
        let base = ThemeColor {
            sidebar: parse("#123456"),
            popover: parse("#654321"),
            ..ThemeColor::default()
        };

        let colors = four_colors("#101010", "#202020", "#f0f0f0", "#00ffff");
        let kit = kit_colors_for(base, &colors);

        assert_eq!(
            kit.sidebar,
            parse("#123456"),
            "an unmapped slot must not be reset by the mapping"
        );
        assert_eq!(kit.popover, parse("#654321"), "same for popover");
        // ...while the mapped ones really did change.
        assert_eq!(kit.accent, parse("#00ffff"), "accent is mapped");
    }

    #[gpui::test]
    fn mapped_colors_reach_the_tokens_the_components_read(cx: &mut gpui::App) {
        let app_theme = probe_theme();
        crate::kit_theme::sync_kit_theme(cx, &app_theme, ThemeMode::Dark);
        let theme = Theme::global(cx);

        assert_eq!(
            theme.tokens.button.color, theme.colors.button,
            "resting chip: tokens.button must track the mapped colors.button"
        );
        assert_eq!(
            theme.tokens.button_hover.color, theme.colors.button_hover,
            "hover chip: tokens.button_hover must track the mapped colors.button_hover"
        );
        assert_eq!(
            theme.colors.accent,
            parse("#00ffff"),
            "accent must survive as the app theme's own accent"
        );
        assert_eq!(
            theme.radius,
            gpui::px(KIT_RADIUS_PX),
            "the bridge owns the kit's global corner radius"
        );
    }

    /// Author-declared slots must land in their own kit roles, keeping both the
    /// value and the alpha the file asked for.
    ///
    /// Declared rather than derived on purpose: if the mapping kept recomputing
    /// these from the four base colors, this would compare two equal values and
    /// pass while the author's explicit choice was discarded.
    #[test]
    fn declared_slots_reach_the_kit_in_their_own_roles() {
        let mut colors = four_colors("#101010", "#202020", "#f0f0f0", "#00ffff");
        colors.danger = "#ff0066".into();
        colors.muted_text = "#8899aa".into();
        colors.ring = "#00ff88".into();
        colors.border = "#ffffff22".into();
        let kit = kit_colors_for(ThemeColor::default(), &colors);

        assert_eq!(kit.danger, parse("#ff0066"), "danger -> kit danger");
        assert_eq!(kit.muted_foreground, parse("#8899aa"), "muted_text");
        assert_eq!(kit.ring, parse("#00ff88"), "ring");
        assert_eq!(
            kit.border.a,
            parse("#ffffff22").a,
            "border keeps the author's alpha, which is the point of declaring it"
        );
        assert_ne!(
            kit.danger_hover, kit.danger,
            "danger_hover must be a distinct state, not the idle color"
        );
    }
}

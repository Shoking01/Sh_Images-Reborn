//! Theme parsing, validation, and discovery (AGENTS.md §10).
//!
//! Note (approved design deviation): on invalid theme hot-reload input, the app
//! keeps the last valid theme instead of falling back to the default per
//! AGENTS.md §10. Hot-reload orchestration lives in a later task; this module
//! only parses, validates, and discovers themes.

use crate::errors::{Result, ShImagesError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A validated theme.
///
/// Invariant: values are validated when constructed via [`parse`]; direct
/// construction bypasses [`validate`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Theme {
    /// Display name of the theme.
    pub name: String,
    /// Author of the theme.
    pub author: String,
    /// Schema version of the theme file.
    pub version: u32,
    /// Color palette for the theme.
    pub colors: ThemeColors,
    /// Spacing scale for layout gutters.
    pub spacing: ThemeSpacing,
    /// Corner radius scale for UI surfaces.
    pub radii: ThemeRadii,
    /// Typography settings for UI text.
    pub typography: ThemeTypography,
}

/// A color parsed out of a theme hex string, with its alpha kept separate so
/// mixing can decide what to do with it rather than silently dropping it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rgba8 {
    r: u8,
    g: u8,
    b: u8,
    a: f32,
}

/// Parse a 3-, 6- or 8-digit hex color, with or without a leading `#`.
/// Returns `None` for anything else, which the callers treat as "no opinion"
/// so a malformed optional slot falls back to the required colors rather than
/// poisoning the whole palette.
fn parse_hex_color(hex: &str) -> Option<Rgba8> {
    let t = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    let expand = |s: &str| -> Option<u8> { u8::from_str_radix(s, 16).ok() };
    match t.len() {
        3 | 4 => {
            let d: Vec<u8> = (0..t.len())
                .map(|i| expand(&t[i..i + 1]))
                .collect::<Option<Vec<u8>>>()?;
            // `#abc` means `#aabbcc`; the fourth digit, when present, is alpha.
            Some(Rgba8 {
                r: d[0] * 17,
                g: d[1] * 17,
                b: d[2] * 17,
                a: if d.len() == 4 {
                    f32::from(d[3] * 17) / 255.0
                } else {
                    1.0
                },
            })
        }
        6 | 8 => {
            let bytes: Vec<u8> = (0..t.len() / 2)
                .map(|i| expand(&t[i * 2..i * 2 + 2]))
                .collect::<Option<Vec<u8>>>()?;
            Some(Rgba8 {
                r: bytes[0],
                g: bytes[1],
                b: bytes[2],
                a: if bytes.len() == 4 {
                    f32::from(bytes[3]) / 255.0
                } else {
                    1.0
                },
            })
        }
        _ => None,
    }
}

fn to_hex(c: Rgba8) -> String {
    if (c.a - 1.0).abs() < f32::EPSILON {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        let a = (c.a.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, a)
    }
}

/// Blend `from` toward `to` by `ratio`, in gamma-encoded sRGB — the same naive
/// channel mix the app's own `hover_tint` uses, so derived slots land on the
/// same visual weight as the states they replace.
fn mix_hex(from: &str, to: &str, ratio: f32) -> Option<String> {
    let a = parse_hex_color(from)?;
    let b = parse_hex_color(to)?;
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * ratio).round() as u8;
    Some(to_hex(Rgba8 {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: 1.0,
    }))
}

fn set_alpha(hex: &str, alpha: f32) -> Option<String> {
    let mut c = parse_hex_color(hex)?;
    c.a = alpha;
    Some(to_hex(c))
}

/// Perceived luminance, Rec. 709 weights. Duplicated from the app's `luma` on
/// purpose: `sh-core` cannot depend on `sh-app`, and a 3-line weighting formula
/// is cheaper than inverting the crate layering.
fn luma(hex: &str) -> f32 {
    let Some(c) = parse_hex_color(hex) else {
        return 0.0;
    };
    (0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b)) / 255.0
}

impl ThemeColors {
    /// Fill every optional slot the theme file omitted, deriving it from the
    /// four required colors.
    ///
    /// Called on every parse and on every theme adoption, so a four-color theme
    /// written before these slots existed keeps working with no edit — the whole
    /// point of making them optional rather than required.
    ///
    /// A slot the file *did* provide is left alone, always. Derivation is a
    /// fallback, not a normalizer: re-deriving would silently discard a theme
    /// author's explicit choice on every reload.
    pub fn with_derived_defaults(mut self) -> Self {
        let background = self.background.clone();
        let surface = self.surface.clone();
        let text = self.text.clone();
        let accent = self.accent.clone();

        if self.muted_text.trim().is_empty() {
            // Labels must recede but stay readable: most of the way to the page
            // color, not all of it.
            self.muted_text = mix_hex(&text, &background, 0.38).unwrap_or_else(|| text.clone());
        }
        if self.border.trim().is_empty() {
            self.border = set_alpha(&text, 0.18).unwrap_or_else(|| text.clone());
        }
        if self.on_accent.trim().is_empty() {
            // Accents are chosen to contrast with the page, so page-colored
            // text on top of one is the consistent choice.
            self.on_accent = background.clone();
        }
        if self.elevated.trim().is_empty() {
            // A hair above `surface`, matching the resting-chip lift the top
            // bar needs in order to read against a translucent bar.
            self.elevated = mix_hex(&surface, &text, 0.07).unwrap_or_else(|| surface.clone());
        }
        if self.ring.trim().is_empty() {
            // Keyboard focus should read as the theme's own accent, not a new
            // color, and translucent enough not to compete with the control.
            self.ring = set_alpha(&accent, 0.55).unwrap_or_else(|| accent.clone());
        }
        if self.danger.trim().is_empty() {
            // Anchored differently by polarity, the same way `hover_fill`
            // branches on luminance: on a light theme the text color is dark,
            // so a raw red would be a pale sticker. Pull it toward text instead.
            const RED: &str = "#e5484d";
            let anchor = if luma(&background) > 0.5 {
                &text
            } else {
                &background
            };
            self.danger = mix_hex(RED, anchor, 0.25).unwrap_or_else(|| RED.to_string());
        }
        self
    }
}

/// Color palette for a theme.
///
/// The first four colors are required. The rest are **semantic slots the app
/// derives its own states from**, and every one of them is optional in the
/// JSON: `#[serde(default)]` fills any that the file omits by deriving them
/// from `background` / `surface` / `text` / `accent`, so a theme file written
/// against the four-color schema keeps rendering exactly as it did.
///
/// They exist because the app's interaction states were being *computed* rather
/// than declared. `hover_fill`, `hover_fill_strong` and
/// `viewer_control_hover_fill` each pick a mix ratio from a color's luminance,
/// which means a theme author who wanted a specific hover had no way to say so.
/// These slots put the decision in the file.
///
/// They are declared here rather than borrowed from a UI toolkit on purpose:
/// `sh-core` has no GPUI dependency (see ADR-001 and `lib.rs`), and it must not
/// acquire one. The bridge in `sh-app` maps these onto whatever component
/// library is in use.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ThemeColors {
    /// Window background color (hex, e.g. `#0d0d0f`).
    pub background: String,
    /// Elevated surface color for panels, toolbars, and cards.
    pub surface: String,
    /// Primary text color.
    pub text: String,
    /// Accent color for highlights and selection.
    pub accent: String,
    /// Secondary text for labels, captions and metadata that must recede
    /// behind [`ThemeColors::text`].
    #[serde(default)]
    pub muted_text: String,
    /// Hairlines and control outlines. Expected to be a low-alpha hex
    /// (8-digit, e.g. `#ffffff2e`), not an opaque color.
    #[serde(default)]
    pub border: String,
    /// Destructive actions: the reset button, the trash affordance, the error
    /// state of a failed crop or export.
    #[serde(default)]
    pub danger: String,
    /// Text drawn on top of [`ThemeColors::accent`].
    #[serde(default)]
    pub on_accent: String,
    /// Surface one step above [`ThemeColors::surface`] — cards sitting on
    /// panels, popovers, hovered rows.
    #[serde(default)]
    pub elevated: String,
    /// Focus ring around keyboard-focused controls.
    #[serde(default)]
    pub ring: String,
}

/// Spacing scale for layout gutters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeSpacing {
    /// Extra-small spacing token.
    pub xs: u32,
    /// Small spacing token.
    pub sm: u32,
    /// Medium spacing token.
    pub md: u32,
    /// Large spacing token.
    pub lg: u32,
}

/// Corner radius scale for UI surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeRadii {
    /// Small radius token.
    pub sm: u32,
    /// Medium radius token.
    pub md: u32,
    /// Large radius token.
    pub lg: u32,
}

/// Typography settings for UI text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeTypography {
    /// Font family name (e.g. `Inter`).
    pub family: String,
    /// Font size scale for UI text.
    pub sizes: ThemeSizes,
}

/// Font size scale for UI text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeSizes {
    /// Caption text size token.
    pub caption: u32,
    /// Body text size token.
    pub body: u32,
    /// Title text size token.
    pub title: u32,
}

/// Image extensions recognized by the viewer.
///
/// Temporarily duplicated from `navigation`'s `is_supported` logic; a later
/// refactor makes navigation reuse this list.
const SUPPORTED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff"];

/// Parse and validate a theme from JSON.
///
/// Optional color slots are filled from the four required ones *before*
/// validation, so `validate` always sees a complete palette and never has
/// to care whether the file was written against the four-color schema or a
/// newer one. Without this, a four-color theme would fail validation
/// against slots it never knew it needed to declare.
pub fn parse(json: &str) -> Result<Theme> {
    let mut theme: Theme =
        serde_json::from_str(json).map_err(|e| ShImagesError::Theme(e.to_string()))?;
    theme.colors = theme.colors.with_derived_defaults();
    validate(&theme)?;
    Ok(theme)
}

/// Validate a theme's values, returning `Theme` error on any violation.
pub fn validate(theme: &Theme) -> Result<()> {
    if theme.name.trim().is_empty() {
        return Err(ShImagesError::Theme("name must not be empty".into()));
    }
    validate_color(&theme.colors.background)?;
    validate_color(&theme.colors.surface)?;
    validate_color(&theme.colors.text)?;
    validate_color(&theme.colors.accent)?;
    validate_color(&theme.colors.muted_text)?;
    validate_color(&theme.colors.border)?;
    validate_color(&theme.colors.danger)?;
    validate_color(&theme.colors.on_accent)?;
    validate_color(&theme.colors.elevated)?;
    validate_color(&theme.colors.ring)?;
    if theme.typography.family.trim().is_empty() {
        return Err(ShImagesError::Theme(
            "typography.family must not be empty".into(),
        ));
    }
    Ok(())
}

/// Returns true if `hex` is a valid 3, 6, or 8-digit hex color (with optional leading `#`).
pub fn validate_color(hex: &str) -> Result<()> {
    let trimmed = hex.strip_prefix('#').unwrap_or(hex);
    let valid =
        matches!(trimmed.len(), 3 | 6 | 8) && trimmed.chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(ShImagesError::Theme(format!("invalid color: {hex}")))
    }
}

/// Discover theme files in a config directory (`.json` extension, sorted).
pub fn discover(config_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(config_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// One theme file found on disk, parsed if it was valid.
///
/// `theme` is `None` for a file that exists but fails [`parse`]. Such an
/// entry is reported rather than dropped: a user who drops a broken file
/// into their themes directory must be able to SEE that it was found, or
/// "not discovered" and "discovered but broken" are indistinguishable from
/// the outside and the feature looks broken. Callers decide how to render
/// it; this type only refuses to lie about which of the two happened.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredTheme {
    /// Absolute path to the theme file. This is the path a hot-reload
    /// watcher must follow for THIS theme.
    pub path: PathBuf,
    /// File name, which is also the key persisted in `Settings::theme`.
    pub file_name: String,
    /// Parsed and validated theme, or `None` when the file is invalid.
    pub theme: Option<Theme>,
    /// The exact text that produced `theme`. Carried so the caller can seed
    /// a hot-reload dedupe baseline from the read it already performed
    /// instead of re-reading the file on the main thread.
    pub text: String,
    /// Why the file was rejected, for logging. `None` when `theme` is
    /// `Some`; also `None` when the file was merely unreadable (an
    /// unreadable file yields no theme and no parse error to report).
    pub error: Option<String>,
}

/// Enumerate and parse every theme file in `config_dir`.
///
/// Performs filesystem I/O and JSON parsing, so it belongs on a worker
/// thread — never on the render/frame loop (AGENTS.md §7.1). A missing
/// directory yields an empty list rather than an error, matching
/// [`discover`], because "the user has no custom themes yet" is the normal
/// first-run state and must not be reported as a failure.
pub fn load_discovered(config_dir: &Path) -> Vec<DiscoveredTheme> {
    discover(config_dir)
        .into_iter()
        .map(|path| {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Read and parse independently: an unreadable file and an
            // invalid one both yield `theme: None`, but only the invalid
            // one has a message worth logging.
            let (text, theme, error) = match std::fs::read_to_string(&path) {
                Ok(text) => match parse(&text) {
                    Ok(theme) => (text, Some(theme), None),
                    Err(e) => (text, None, Some(e.to_string())),
                },
                Err(e) => (String::new(), None, Some(format!("unreadable: {e}"))),
            };
            DiscoveredTheme {
                path,
                file_name,
                theme,
                text,
                error,
            }
        })
        .collect()
}

/// Supported extensions, public for navigation reuse.
pub fn supported_extensions() -> &'static [&'static str] {
    SUPPORTED_EXTENSIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r##"{
        "name": "Neon Nights",
        "author": "username",
        "version": 1,
        "colors": { "background": "#0a0a1a", "surface": "#12122a", "text": "#e0e0ff", "accent": "#00ffff" },
        "spacing": { "xs": 4, "sm": 8, "md": 16, "lg": 24 },
        "radii": { "sm": 2, "md": 6, "lg": 12 },
        "typography": { "family": "Inter", "sizes": { "caption": 11, "body": 14, "title": 18 } }
    }"##;

    #[test]
    fn parses_valid_theme() {
        let t = parse(VALID).unwrap();
        assert_eq!(t.name, "Neon Nights");
        assert_eq!(t.colors.background, "#0a0a1a");
        assert_eq!(t.spacing.md, 16);
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse("{ not json").unwrap_err();
        assert!(matches!(err, ShImagesError::Theme(_)));
    }

    #[test]
    fn rejects_missing_required_field() {
        let json = r##"{"name":"X","author":"a","version":1,"colors":{"background":"#000","surface":"#111","text":"#fff","accent":"#0f0"},"spacing":{"xs":1,"sm":2,"md":3,"lg":4},"radii":{"sm":1,"md":2,"lg":3},"typography":{"family":"Arial","sizes":{"caption":1,"body":2,"title":3}}}"##;
        // Remove a required field (spacing) to prove missing-field mismatches fail:
        let arr = serde_json::from_str::<serde_json::Value>(json).unwrap();
        let mut obj = arr.as_object().unwrap().clone();
        obj.remove("spacing");
        let bad = serde_json::to_string(&obj).unwrap();
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn rejects_invalid_color_formats() {
        let bad = VALID.replace("#0a0a1a", "#12");
        assert!(parse(&bad).is_err());
        let bad2 = VALID.replace("#0a0a1a", "red");
        assert!(parse(&bad2).is_err());
    }

    #[test]
    fn accepts_three_six_eight_digit_colors() {
        assert!(validate_color("#abc").is_ok());
        assert!(validate_color("#aabbcc").is_ok());
        assert!(validate_color("#aabbccdd").is_ok());
        assert!(validate_color("abc").is_ok());
        assert!(validate_color("#ab").is_err());
        assert!(validate_color("#zzz").is_err());
    }

    #[test]
    fn rejects_multiple_leading_hashes() {
        assert!(validate_color("##abc").is_err());
        assert!(validate_color("##aabbcc").is_err());
    }

    #[test]
    fn rejects_empty_name() {
        let json = VALID.replace("Neon Nights", "");
        assert!(parse(&json).is_err());
    }

    #[test]
    fn rejects_empty_typography_family() {
        let json = VALID.replace("Inter", "");
        assert!(parse(&json).is_err());
    }

    #[test]
    fn parses_eight_digit_hex_color() {
        let json = VALID.replace("#0a0a1a", "#0a0a1aff");
        let t = parse(&json).unwrap();
        assert_eq!(t.colors.background, "#0a0a1aff");
    }

    #[test]
    fn supported_extensions_has_expected_list() {
        assert_eq!(
            supported_extensions(),
            &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff"]
        );
    }

    #[test]
    fn discover_finds_theme_files_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.json"), VALID).unwrap();
        std::fs::write(dir.path().join("a.json"), VALID).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let found = discover(dir.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].file_name().unwrap(), "a.json");
        assert_eq!(found[1].file_name().unwrap(), "b.json");
    }

    #[test]
    fn discover_filters_directories_and_case() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.JSON"), VALID).unwrap();
        std::fs::write(dir.path().join("b.json"), VALID).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        std::fs::write(dir.path().join("noext"), "x").unwrap();
        std::fs::create_dir(dir.path().join("fake.json")).unwrap();
        let found = discover(dir.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].file_name().unwrap(), "a.JSON");
        assert_eq!(found[1].file_name().unwrap(), "b.json");
    }

    #[test]
    fn discover_missing_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(discover(&dir.path().join("nope")).is_empty());
    }

    // ── load_discovered: the entry point the App's picker calls ──

    fn valid_theme_json(name: &str) -> String {
        format!(
            r##"{{
            "name": "{name}",
            "author": "test",
            "version": 1,
            "colors": {{ "background": "#000", "surface": "#111", "text": "#fff", "accent": "#0f0" }},
            "spacing": {{ "xs": 4, "sm": 8, "md": 16, "lg": 24 }},
            "radii": {{ "sm": 2, "md": 6, "lg": 12 }},
            "typography": {{ "family": "Inter", "sizes": {{ "caption": 11, "body": 14, "title": 18 }} }}
        }}"##
        )
    }

    #[test]
    fn load_discovered_parses_valid_files_with_their_exact_text() {
        let dir = tempfile::tempdir().unwrap();
        let body = valid_theme_json("Mine");
        let path = dir.path().join("mine.json");
        std::fs::write(&path, &body).unwrap();

        let found = load_discovered(dir.path());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name, "mine.json");
        assert_eq!(found[0].path, path);
        assert_eq!(found[0].theme.as_ref().unwrap().name, "Mine");
        assert_eq!(found[0].text, body, "the read text is carried verbatim");
        assert!(found[0].error.is_none());
    }

    /// An unparseable file is REPORTED, not dropped: the picker must be
    /// able to distinguish "not discovered" from "discovered but broken".
    #[test]
    fn load_discovered_reports_invalid_files_instead_of_dropping_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ not json").unwrap();

        let found = load_discovered(dir.path());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name, "broken.json");
        assert!(found[0].theme.is_none());
        assert!(found[0].error.is_some(), "the reason must be reported");
    }

    /// Valid JSON that is not a valid THEME is the same class of failure as
    /// unparseable JSON: present, rejected, explained.
    #[test]
    fn load_discovered_rejects_schema_invalid_themes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.json"), r#"{"name": ""}"#).unwrap();

        let found = load_discovered(dir.path());

        assert_eq!(found.len(), 1);
        assert!(found[0].theme.is_none());
        assert!(found[0].error.is_some());
    }

    #[test]
    fn load_discovered_missing_dir_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_discovered(&dir.path().join("nope")).is_empty());
    }

    #[test]
    fn load_discovered_is_sorted_by_path() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["c.json", "a.json", "b.json"] {
            std::fs::write(dir.path().join(name), valid_theme_json(name)).unwrap();
        }

        let found = load_discovered(dir.path());

        let names: Vec<&str> = found.iter().map(|d| d.file_name.as_str()).collect();
        assert_eq!(names, vec!["a.json", "b.json", "c.json"]);
    }

    #[test]
    fn load_discovered_ignores_non_json_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
        std::fs::create_dir(dir.path().join("nested.json")).unwrap();
        std::fs::write(dir.path().join("ok.json"), valid_theme_json("Ok")).unwrap();

        let found = load_discovered(dir.path());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name, "ok.json");
    }

    #[test]
    fn parses_all_builtin_themes() {
        for (src, name, background) in [
            (
                include_str!("../../../themes/deep-neutral.json"),
                "Deep Neutral",
                "#0d0d0f",
            ),
            (
                include_str!("../../../themes/dark-clinical.json"),
                "Dark Clinical",
                "#101014",
            ),
            (
                include_str!("../../../themes/light-clean.json"),
                "Light Clean",
                "#f4f4f6",
            ),
            (
                include_str!("../../../themes/noir-gallery.json"),
                "Noir Gallery",
                "#050507",
            ),
        ] {
            let t = parse(src).unwrap();
            assert_eq!(t.name, name);
            assert_eq!(t.colors.background, background);
        }
    }

    // ── Optional semantic slots: derived, never required ──

    /// The promise that makes the schema change safe: a theme file written
    /// before these slots existed parses unchanged and gains a full palette.
    ///
    /// Every optional slot is asserted, not just "some defaults appeared" —
    /// each one is used by the UI, so a slot that silently derived to an empty
    /// string would render an invisible control rather than fail.
    #[test]
    fn four_color_theme_gains_every_optional_slot() {
        let t = parse(VALID).unwrap();
        for (slot, value) in [
            ("muted_text", &t.colors.muted_text),
            ("border", &t.colors.border),
            ("danger", &t.colors.danger),
            ("on_accent", &t.colors.on_accent),
            ("elevated", &t.colors.elevated),
            ("ring", &t.colors.ring),
        ] {
            assert!(
                validate_color(value).is_ok(),
                "{slot} must derive to a valid color, got {value:?}"
            );
            assert!(!value.trim().is_empty(), "{slot} must not stay empty");
        }
    }

    /// A slot the author declared is theirs. Derivation runs on every parse, so
    /// if it ever overwrote an explicit value the author's choice would be
    /// discarded silently on every reload — the kind of bug that only shows up
    /// as "my theme stopped working after I restarted".
    #[test]
    fn explicit_slots_survive_derivation() {
        let src = VALID.replace(
            r##""accent": "#00ffff""##,
            r##""accent": "#00ffff", "danger": "#ff00aa", "elevated": "#334455""##,
        );
        let t = parse(&src).unwrap();
        assert_eq!(
            t.colors.danger, "#ff00aa",
            "declared danger was overwritten"
        );
        assert_eq!(
            t.colors.elevated, "#334455",
            "declared elevated was overwritten"
        );
        // ...while the ones left out are still derived.
        assert!(validate_color(&t.colors.ring).is_ok());
    }

    /// `danger` is the one slot whose derivation is not a plain blend, because a
    /// single fixed red cannot serve both polarities: on a light theme the text
    /// color is dark, so an unblended red would be a pale sticker with almost no
    /// contrast. The rule is the same polarity branch `hover_fill` already uses.
    #[test]
    fn danger_derivation_adapts_to_theme_polarity() {
        let dark = four("#0d0d0f", "#18181c", "#e8e8ee");
        let light = four("#f4f4f6", "#ffffff", "#1a1a1e");

        let dark_danger = parse_hex_color(&dark.danger).unwrap();
        let light_danger = parse_hex_color(&light.danger).unwrap();
        assert!(
            dark_danger.r > dark_danger.b,
            "dark danger must stay reddish"
        );
        assert!(
            light_danger.r > light_danger.g && light_danger.r > light_danger.b,
            "light danger must stay reddish"
        );
        // Pulled toward the (dark) text color, so it lands darker than the raw red
        // the dark branch starts from.
        assert!(
            light_danger.r < 0xe5,
            "light danger must be darkened toward text, got {:#04x}",
            light_danger.r
        );
    }

    /// `elevated` must be a small, polarity-correct step away from `surface`.
    ///
    /// What is asserted here is the *mechanism* — one small blend toward
    /// `text` — and not an outcome like "further from the page". An earlier
    /// version of this test asserted the latter and was wrong: on `light-clean`
    /// the text color is darker than the surface, so the blend moves *down*, and
    /// the derived value ends up numerically closer to the page while still
    /// being the correct next surface in the ramp. Distance from the page is not
    /// what "elevated" means; adjacency to `surface` in the direction of `text`
    /// is.
    #[test]
    fn elevated_steps_from_surface_toward_text() {
        for (name, page, surface, text) in [
            ("noir-gallery", "#050507", "#101016", "#e8e8ee"),
            ("dark-clinical", "#101014", "#17171d", "#dfdfe5"),
            ("light-clean", "#f4f4f6", "#ffffff", "#1a1a1e"),
        ] {
            let c = four(page, surface, text);
            let text = parse_hex_color(text).unwrap();
            let surface = parse_hex_color(&c.surface).unwrap();
            let elevated = parse_hex_color(&c.elevated).unwrap();

            let step = i32::from(elevated.r) - i32::from(surface.r);
            let toward_text = i32::from(text.r) - i32::from(surface.r);

            assert_ne!(step, 0, "{name}: elevated must differ from surface");
            assert_eq!(
                step.signum(),
                toward_text.signum(),
                "{name}: elevated must move toward text (step={step}, text is {toward_text} away)"
            );
            assert!(
                step.abs() <= 20,
                "{name}: the step must stay subtle, got {step}"
            );
        }
    }

    /// A derived alpha slot must actually be translucent. An opaque `border`
    /// would draw a hard line where the design intends a hairline.
    #[test]
    fn border_and_ring_derive_with_transparency() {
        let c = four("#0d0d0f", "#18181c", "#e8e8ee");
        for (name, hex) in [("border", &c.border), ("ring", &c.ring)] {
            let a = parse_hex_color(hex).unwrap().a;
            assert!(a > 0.0, "{name} must still be visible");
            assert!(a < 1.0, "{name} must be translucent, got alpha {a}");
        }
    }

    fn four(background: &str, surface: &str, text: &str) -> ThemeColors {
        ThemeColors {
            background: background.into(),
            surface: surface.into(),
            text: text.into(),
            accent: "#00ffff".into(),
            ..Default::default()
        }
        .with_derived_defaults()
    }

    /// Every shipped theme declares every optional slot.
    ///
    /// Derivation exists so a *user's* four-color file keeps working. The
    /// built-ins are not in that position: they are the reference palettes, and
    /// a derived value in one of them means the shipped design is a function of
    /// the derivation ratios rather than something a reader can see and tune.
    ///
    /// This asserts the declaration is present in the *file*, not just non-empty
    /// after parsing — parsing fills the gaps, so only inspecting the raw JSON
    /// can tell a declared slot from a derived one.
    #[test]
    fn builtin_themes_declare_every_optional_slot() {
        for (name, src) in [
            (
                "noir-gallery",
                include_str!("../../../themes/noir-gallery.json"),
            ),
            (
                "dark-clinical",
                include_str!("../../../themes/dark-clinical.json"),
            ),
            (
                "deep-neutral",
                include_str!("../../../themes/deep-neutral.json"),
            ),
            (
                "light-clean",
                include_str!("../../../themes/light-clean.json"),
            ),
        ] {
            let raw: serde_json::Value = serde_json::from_str(src).unwrap();
            let colors = &raw["colors"];
            for slot in [
                "muted_text",
                "border",
                "danger",
                "on_accent",
                "elevated",
                "ring",
            ] {
                assert!(
                    colors.get(slot).is_some(),
                    "{name}: colors.{slot} must be declared in the file, not derived"
                );
            }
            // And the declared values must survive parsing untouched.
            let t = parse(src).unwrap();
            for slot in [
                "muted_text",
                "border",
                "danger",
                "on_accent",
                "elevated",
                "ring",
            ] {
                assert_eq!(
                    colors[slot].as_str().unwrap(),
                    match slot {
                        "muted_text" => &t.colors.muted_text,
                        "border" => &t.colors.border,
                        "danger" => &t.colors.danger,
                        "on_accent" => &t.colors.on_accent,
                        "elevated" => &t.colors.elevated,
                        _ => &t.colors.ring,
                    },
                    "{name}: colors.{slot} was altered on the way in"
                );
            }
        }
    }
}

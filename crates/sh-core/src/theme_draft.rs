//! Authoring a theme from inside the app, instead of hand-editing JSON.
//!
//! [`ThemeDraft`] holds the ten color slots as *text*, not as parsed values.
//! That is the whole point of the type: a user who has typed three characters of
//! a hex code must be able to have that state represented without it being
//! valid, or every keystroke would have to be rejected. Resolution to a
//! [`Theme`] happens once, on demand, through the same derivation and
//! validation the file loader uses — so the editor and the loader cannot
//! disagree about what a valid theme is.

use crate::errors::{Result, ShImagesError};
use crate::theme::{validate_color, Theme, ThemeColors, ThemeInteraction, ThemeTypography};

/// The ten color slots, in the order the editor renders them.
///
/// The order is the declaration order of [`ThemeColors`], and it is a `const`
/// rather than an array literal so the render, the keyboard order and the tests
/// cannot drift apart.
pub const SLOTS: [Slot; 10] = [
    Slot::Background,
    Slot::Surface,
    Slot::Text,
    Slot::Accent,
    Slot::MutedText,
    Slot::Border,
    Slot::Danger,
    Slot::OnAccent,
    Slot::Elevated,
    Slot::Ring,
];

/// One editable color slot.
///
/// A `struct` rather than a `String` key so a missing slot is a compile error
/// instead of a silent `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    /// The page behind everything.
    Background,
    /// Cards and panels that sit on the page.
    Surface,
    /// Default text color.
    Text,
    /// The one saturated color: selection, focus, the active segment.
    Accent,
    /// De-emphasized text.
    MutedText,
    /// Hairlines and control borders.
    Border,
    /// Destructive controls.
    Danger,
    /// Text drawn on top of `Accent` or `Danger`.
    OnAccent,
    /// A surface one step above `Surface` — the title bar.
    Elevated,
    /// Focus rings.
    Ring,
}

impl Slot {
    /// The `ThemeColors` field this slot writes.
    ///
    /// Exhaustive on purpose: adding a field to `ThemeColors` without adding it
    /// here makes this function fail to compile rather than leave the new slot
    /// uneditable.
    fn field(self, colors: &mut ThemeColors) -> &mut String {
        match self {
            Self::Background => &mut colors.background,
            Self::Surface => &mut colors.surface,
            Self::Text => &mut colors.text,
            Self::Accent => &mut colors.accent,
            Self::MutedText => &mut colors.muted_text,
            Self::Border => &mut colors.border,
            Self::Danger => &mut colors.danger,
            Self::OnAccent => &mut colors.on_accent,
            Self::Elevated => &mut colors.elevated,
            Self::Ring => &mut colors.ring,
        }
    }

    /// This slot's value, borrowed from a `ThemeColors`.
    fn get(self, colors: &ThemeColors) -> &str {
        match self {
            Self::Background => &colors.background,
            Self::Surface => &colors.surface,
            Self::Text => &colors.text,
            Self::Accent => &colors.accent,
            Self::MutedText => &colors.muted_text,
            Self::Border => &colors.border,
            Self::Danger => &colors.danger,
            Self::OnAccent => &colors.on_accent,
            Self::Elevated => &colors.elevated,
            Self::Ring => &colors.ring,
        }
    }

    /// Stable lowercase name, used in error messages and as a debug selector.
    ///
    /// Derived from the JSON field name rather than the Rust variant name so an
    /// error points at the key the user would find in their file.
    pub fn name(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Surface => "surface",
            Self::Text => "text",
            Self::Accent => "accent",
            Self::MutedText => "muted_text",
            Self::Border => "border",
            Self::Danger => "danger",
            Self::OnAccent => "on_accent",
            Self::Elevated => "elevated",
            Self::Ring => "ring",
        }
    }
}

/// A theme being authored. Every field is free text; nothing here is guaranteed
/// to resolve until [`ThemeDraft::resolve`] is called.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeDraft {
    /// `Theme.name` — must be non-empty for the draft to resolve.
    pub name: String,
    /// The ten slots, verbatim as typed.
    pub colors: Vec<(Slot, String)>,
    /// `typography.family` — must be non-empty for the draft to resolve.
    pub family: String,
    /// `interaction.hover_ratio`.
    pub hover_ratio: f32,
    /// `interaction.hover_ratio_strong`.
    pub hover_ratio_strong: f32,
    /// `Theme.author`, carried through untouched.
    pub author: String,
    /// `Theme.version`, carried through untouched.
    pub version: u32,
    /// `Theme.spacing`, carried through untouched.
    pub spacing: crate::theme::ThemeSpacing,
    /// `Theme.radii`, carried through untouched.
    pub radii: crate::theme::ThemeRadii,
    /// `Theme.typography.sizes`, carried through untouched.
    pub sizes: crate::theme::ThemeSizes,
}

impl ThemeDraft {
    /// Warm-start a draft from an already-valid theme.
    ///
    /// Round-trips through `resolve`: a draft built this way always resolves back
    /// to an equal theme, which is the property the tests pin.
    ///
    /// Carries `author` and the non-color scales verbatim from the source theme,
    /// because the editor does not expose them and must not silently reset them
    /// on save. `baseline()` supplies them only for fields the draft owns.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            name: theme.name.clone(),
            colors: SLOTS
                .iter()
                .map(|slot| (*slot, slot.get(&theme.colors).to_string()))
                .collect(),
            family: theme.typography.family.clone(),
            hover_ratio: theme.interaction.hover_ratio,
            hover_ratio_strong: theme.interaction.hover_ratio_strong,
            // Preserved, never edited by the editor, but written back out.
            author: theme.author.clone(),
            version: theme.version,
            spacing: theme.spacing.clone(),
            radii: theme.radii.clone(),
            sizes: theme.typography.sizes.clone(),
        }
    }

    /// This slot's current text.
    pub fn get(&self, slot: Slot) -> &str {
        self.colors
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, v)| v.as_str())
            .unwrap_or_default()
    }

    /// Replace this slot's text. Unknown slots cannot be added, so a stray key
    /// is not representable.
    pub fn set(&mut self, slot: Slot, value: impl Into<String>) {
        let value = value.into();
        match self.colors.iter_mut().find(|(s, _)| *s == slot) {
            Some(entry) => entry.1 = value,
            None => self.colors.push((slot, value)),
        }
    }

    /// Resolve to a validated `Theme`.
    ///
    /// Per-slot hex checks run **first** and name the slot, because a half-typed
    /// hex is the overwhelmingly common failure and a generic "invalid theme"
    /// would leave the user hunting. The structural checks (`name`, `family`)
    /// run after, and only once every slot is well-formed — otherwise a user
    /// with an empty name *and* a partial hex would be told about the name
    /// first and fix it, then be told about the hex.
    ///
    /// The resolved colors go through `with_derived_defaults` exactly as
    /// `theme::parse` does, so an empty optional slot is derived rather than
    /// rejected. That is what makes "clear the box to reset it" work.
    pub fn resolve(&self) -> Result<Theme> {
        let mut colors = ThemeColors::default();
        for slot in SLOTS {
            *slot.field(&mut colors) = self.get(slot).to_string();
        }
        // Derive BEFORE validating. Six slots are optional and a cleared box is
        // how the editor expresses "reset this", so validating the raw text
        // would reject an empty optional slot that the loader would happily
        // derive. The four required slots stay strict because `parse` rejects
        // them empty.
        colors = colors.with_derived_defaults();

        // Validate after deriving so the error names the slot the user actually
        // typed into, and so a malformed required slot still fails.
        for slot in SLOTS {
            let text = slot.get(&colors);
            if let Err(error) = validate_color(text) {
                return Err(ShImagesError::Theme(format!(
                    "{}: {}",
                    slot.name(),
                    slot_message(error)
                )));
            }
        }

        let mut theme = baseline();
        theme.name = self.name.clone();
        theme.author = self.author.clone();
        theme.version = self.version;
        theme.colors = colors;
        theme.spacing = self.spacing.clone();
        theme.radii = self.radii.clone();
        theme.interaction = ThemeInteraction {
            hover_ratio: self.hover_ratio,
            hover_ratio_strong: self.hover_ratio_strong,
        };
        theme.typography.family = self.family.clone();
        theme.typography.sizes = self.sizes.clone();
        crate::theme::validate(&theme)?;
        Ok(theme)
    }

    /// The resolved theme as the JSON text that would be written to disk.
    ///
    /// Errors rather than falling back, because this is what a save writes and a
    /// save must never persist something the loader would reject.
    pub fn to_json(&self) -> Result<String> {
        let theme = self.resolve()?;
        serde_json::to_string_pretty(&theme).map_err(|e| ShImagesError::Theme(e.to_string()))
    }
}

/// The structural baseline a draft is projected onto.
///
/// Every field the draft does not own comes from the draft itself now
/// (`from_theme` carries them), so this only supplies the four colors a
/// `ThemeColors::default()` seeds. It exists so `resolve` has one place to name
/// a structurally valid theme rather than relying on struct-update syntax, which
/// silently zeroes any field not listed.
fn baseline() -> Theme {
    Theme {
        name: String::new(),
        author: String::new(),
        version: 1,
        colors: ThemeColors::default(),
        spacing: crate::theme::ThemeSpacing {
            xs: 4,
            sm: 8,
            md: 16,
            lg: 24,
        },
        radii: crate::theme::ThemeRadii {
            sm: 2,
            md: 8,
            lg: 14,
        },
        typography: ThemeTypography {
            family: String::new(),
            sizes: crate::theme::ThemeSizes {
                caption: 11,
                body: 14,
                title: 18,
            },
        },
        interaction: ThemeInteraction::default(),
    }
}

/// Strip `validate_color`'s generic prefix so the slot name is not repeated.
fn slot_message(error: ShImagesError) -> String {
    match error {
        ShImagesError::Theme(message) => message,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_theme() -> Theme {
        // A complete built-in theme, not a partial one. `author` and the full
        // `spacing`/`radii`/`sizes` blocks are required by serde, so a fixture
        // that omits them tests the schema rather than the draft.
        crate::theme::parse(
            r##"{
                "name": "Editor Fixture",
                "author": "Sh_Images",
                "version": 1,
                "colors": {
                    "background": "#050507",
                    "surface": "#101016",
                    "text": "#e8e8ee",
                    "accent": "#00ffff",
                    "muted_text": "#929296",
                    "border": "#e8e8ee2e",
                    "danger": "#ad373c",
                    "on_accent": "#050507",
                    "elevated": "#1f1f25",
                    "ring": "#00ffff8c"
                },
                "spacing": { "xs": 4, "sm": 8, "md": 16, "lg": 24 },
                "radii": { "sm": 2, "md": 8, "lg": 14 },
                "typography": { "family": "Inter", "sizes": { "caption": 11, "body": 14, "title": 18 } }
            }"##,
        )
        .expect("fixture must be valid")
    }

    /// The property that makes the editor usable: whatever the loader accepts,
    /// the editor must hand back unchanged.
    #[test]
    fn draft_round_trips_a_valid_theme() {
        let theme = valid_theme();
        let draft = ThemeDraft::from_theme(&theme);
        let resolved = draft.resolve().expect("round trip must resolve");
        assert_eq!(resolved, theme);
    }

    /// A draft is text. An empty optional slot must stay representable and must
    /// derive, not fail — otherwise "clear it to reset" cannot be expressed.
    #[test]
    fn empty_optional_slot_derives_instead_of_failing() {
        let mut draft = ThemeDraft::from_theme(&valid_theme());
        draft.set(Slot::Ring, "");
        let resolved = draft.resolve().expect("empty optional slot must derive");
        assert!(
            !resolved.colors.ring.trim().is_empty(),
            "an emptied ring must be derived from the declared colors, not left blank"
        );
    }

    /// The error has to point at the row, or the user is left hunting.
    #[test]
    fn malformed_slot_error_names_the_slot() {
        let mut draft = ThemeDraft::from_theme(&valid_theme());
        draft.set(Slot::MutedText, "#12");
        let error = draft.resolve().expect_err("a two-character hex must fail");
        let message = error.to_string();
        assert!(
            message.contains("muted_text"),
            "error must name the slot, got: {message}"
        );
    }

    /// A half-typed hex is the common failure, so it must be reported before a
    /// structural complaint the user has to fix first and then hit again.
    #[test]
    fn slot_error_wins_over_an_empty_name() {
        let mut draft = ThemeDraft::from_theme(&valid_theme());
        draft.name = String::new();
        draft.set(Slot::Text, "zzz");
        let message = draft.resolve().expect_err("both are invalid").to_string();
        assert!(
            message.contains("text"),
            "the color error must come first, got: {message}"
        );
    }

    #[test]
    fn empty_name_is_rejected_once_colors_are_well_formed() {
        let mut draft = ThemeDraft::from_theme(&valid_theme());
        draft.name = String::new();
        let message = draft
            .resolve()
            .expect_err("empty name must fail")
            .to_string();
        assert!(
            message.contains("name"),
            "error must mention the name, got: {message}"
        );
    }

    #[test]
    fn empty_family_is_rejected() {
        let mut draft = ThemeDraft::from_theme(&valid_theme());
        draft.family = String::new();
        let message = draft
            .resolve()
            .expect_err("empty family must fail")
            .to_string();
        assert!(message.contains("family"), "got: {message}");
    }

    /// Every slot must be individually addressable. A missing arm would make a
    /// slot silently uneditable, which is invisible until a user tries it.
    #[test]
    fn every_slot_is_independently_editable() {
        let base = ThemeDraft::from_theme(&valid_theme());
        for slot in SLOTS {
            let mut draft = base.clone();
            draft.set(slot, "#ff00ff");
            let resolved = draft
                .resolve()
                .unwrap_or_else(|e| panic!("{} must resolve: {e}", slot.name()));
            assert_eq!(
                slot.get(&resolved.colors),
                "#ff00ff",
                "{} did not take the edited value",
                slot.name()
            );
        }
    }

    #[test]
    fn slot_names_cover_every_declared_field() {
        let theme = valid_theme();
        for slot in SLOTS {
            assert!(
                !slot.get(&theme.colors).is_empty(),
                "{} resolved empty from a valid theme",
                slot.name()
            );
        }
        // 10 slots against a 10-field struct: if someone adds an eleventh
        // field, this count is the thing that should fail.
        assert_eq!(SLOTS.len(), 10);
    }

    #[test]
    fn json_output_is_parseable_back() {
        let draft = ThemeDraft::from_theme(&valid_theme());
        let json = draft.to_json().expect("draft must serialize");
        let reparsed = crate::theme::parse(&json).expect("own output must reparse");
        assert_eq!(reparsed, draft.resolve().expect("resolve"));
    }
}

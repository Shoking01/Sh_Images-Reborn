//! Full-screen Settings surface: slim header + section sidebar + content.

use sh_core::i18n::StrKey;

pub mod scroll;
pub mod sections;
pub mod sidebar;

/// Settings sections in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsSection {
    /// Startup behavior, recent folders, hidden files.
    #[default]
    General,
    /// Theme picker + display toggles.
    Appearance,
    /// Per-slot theme color editor: the ten slots as editable hex, plus the
    /// name and typography family the schema requires alongside them.
    ///
    /// Separate from `Appearance` on purpose: Appearance *applies* a finished
    /// theme, this one *authors* one, and the two have different failure modes
    /// (a bad file vs. a half-typed hex) and different futures (WU-4
    /// copy-on-write, WU-5 live preview).
    ThemeEditor,
    /// Rebindable keyboard shortcuts.
    Shortcuts,
}

impl SettingsSection {
    /// Sidebar rows in order: (section, label key). Labels resolve via
    /// `t(settings.language, …)` at the render site so they live-switch.
    pub const ALL: &[(SettingsSection, StrKey)] = &[
        (SettingsSection::General, StrKey::SectionGeneral),
        (SettingsSection::Appearance, StrKey::SectionAppearance),
        (SettingsSection::ThemeEditor, StrKey::SectionThemeEditor),
        (SettingsSection::Shortcuts, StrKey::SectionShortcuts),
    ];
}

/// Keyboard selection order inside the Appearance section. The app root keeps
/// focus so the existing Enter and Space actions can activate the selected row;
/// Tab changes this selection without stealing the viewer's shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum AppearanceControl {
    /// Filmstrip visibility toggle.
    Filmstrip = 0,
    /// Transparency checkerboard toggle.
    Checkerboard = 1,
    /// Slideshow interval decrement button.
    SlideshowDecrement = 2,
    /// Slideshow interval increment button.
    SlideshowIncrement = 3,
    /// Reduced-motion toggle.
    ReduceMotion = 4,
}

impl AppearanceControl {
    /// Controls in keyboard focus order.
    pub const ALL: [AppearanceControl; 5] = [
        AppearanceControl::Filmstrip,
        AppearanceControl::Checkerboard,
        AppearanceControl::SlideshowDecrement,
        AppearanceControl::SlideshowIncrement,
        AppearanceControl::ReduceMotion,
    ];
}

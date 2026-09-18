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
    /// Rebindable keyboard shortcuts.
    Shortcuts,
}

impl SettingsSection {
    /// Sidebar rows in order: (section, label key). Labels resolve via
    /// `t(settings.language, …)` at the render site so they live-switch.
    pub const ALL: &[(SettingsSection, StrKey)] = &[
        (SettingsSection::General, StrKey::SectionGeneral),
        (SettingsSection::Appearance, StrKey::SectionAppearance),
        (SettingsSection::Shortcuts, StrKey::SectionShortcuts),
    ];
}

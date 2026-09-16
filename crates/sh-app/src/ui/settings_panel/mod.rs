//! Full-screen Settings surface: slim header + section sidebar + content.

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
    /// Sidebar rows in order: (section, label).
    pub const ALL: &[(SettingsSection, &str)] = &[
        (SettingsSection::General, "General"),
        (SettingsSection::Appearance, "Appearance"),
        (SettingsSection::Shortcuts, "Shortcuts"),
    ];
}

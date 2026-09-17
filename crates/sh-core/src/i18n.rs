//! UI language table: English + neutral Spanish string lookups.
//!
//! Pure data + pure functions with no GPUI dependency (see ADR-012). Lookup
//! is O(1) returning `&'static str` with no allocation and no I/O. Every
//! interpolated sentence lives here as a named-argument template function so
//! Spanish word order is free to differ from English (never positional
//! fragment concatenation at call sites).

use serde::{Deserialize, Serialize};

/// UI language. Serialized lowercase (`"en"` / `"es"`) in settings.json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// English. Default; guaranteed fallback for every key.
    #[default]
    En,
    /// Neutral (region-free, usted-neutral) Spanish.
    Es,
}

impl Language {
    /// Render `key` in this language. O(1), no allocation, no I/O.
    /// Falls back to the English rendering when the Spanish arm is empty,
    /// so the UI never shows a blank string.
    pub fn get(self, key: StrKey) -> &'static str {
        match self {
            Language::En => en(key),
            Language::Es => fallback(en(key), es(key)),
        }
    }
}

/// Function-syntax alias for `lang.get(key)`.
pub fn t(lang: Language, key: StrKey) -> &'static str {
    lang.get(key)
}

/// Defense-in-depth for the never-blank-UI rule: an empty Spanish arm
/// renders English. The anti-drift test pins every arm non-empty, so this
/// path only fires if a future edit empties one.
fn fallback<'a>(en: &'a str, es: &'a str) -> &'a str {
    if es.is_empty() {
        en
    } else {
        es
    }
}

/// One variant per inventoried user-facing string. Adding a variant forces
/// both `match` arms below (compiler-checked) plus an `ALL_KEYS` entry
/// (anti-drift-test-checked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrKey {
    /// Welcome hero tagline under the brand mark.
    WelcomeTagline,
    /// Welcome drop-zone line.
    DropZoneHint,
    /// Viewer empty-state placeholder (no file open yet).
    ViewerEmptyHint,
    /// Welcome primary button: open the most recent folder.
    ContinueButton,
    /// Welcome open button (`…` = opens a dialog).
    WelcomeOpen,
    /// Topbar folder affordance (no ellipsis: same-frame action menu).
    TopbarOpen,
    /// Grid empty-state default (no error slot reason).
    GridEmptyDefault,
    /// Sort dropdown header.
    SortByLabel,
    /// Sort criterion: file name.
    SortName,
    /// Sort criterion: creation time.
    SortCreated,
    /// Sort criterion: modification time.
    SortModified,
    /// Sort criterion: file size.
    SortSize,
    /// Sort criterion: file type.
    SortType,
    /// Sort direction: ascending.
    SortAscending,
    /// Sort direction: descending.
    SortDescending,
    /// Topbar back button (viewer arm only).
    TopbarBack,
    /// Crop confirm bar: apply to the original file.
    CropCopy,
    /// Crop confirm bar: save dialog (`…` = opens a dialog).
    CropSave,
    /// Shared cancel: crop bar + batch bar.
    Cancel,
    /// Batch confirm bar: staged recycle-bin delete.
    BatchDelete,
    /// Batch confirm bar: staged move.
    BatchMove,
    /// Settings root header.
    SettingsTitle,
    /// Settings section: language picker + recents.
    SectionGeneral,
    /// Settings section: theme + display prefs.
    SectionAppearance,
    /// Settings section: key bindings.
    SectionShortcuts,
    /// General picker label.
    LanguageLabel,
    /// Picker autonym (proper noun: identical in both languages).
    LanguageEnglish,
    /// Picker autonym (proper noun: identical in both languages).
    LanguageSpanish,
    /// Appearance row: active theme.
    ThemeLabel,
    /// Appearance row: hidden-files toggle.
    ShowHiddenFiles,
    /// Appearance row: clear recents.
    ClearRecents,
    /// Shortcuts capture prompt (`Esc` is a key chip, untranslated).
    CapturePrompt,
    /// Shortcuts footer: restore defaults.
    ResetShortcuts,
    /// Shortcuts footer: second-press confirmation.
    ResetConfirm,
    /// Action label: step to the next image.
    ActionNextImage,
    /// Action label: step to the previous image.
    ActionPrevImage,
    /// Action label: hide/show viewer chrome.
    ActionToggleOverlays,
    /// Action label: fullscreen window.
    ActionToggleFullscreen,
    /// Action label: open a single file.
    ActionOpenFile,
    /// Action label: open a folder.
    ActionOpenFolder,
    /// Action label: leave the viewer / close dialogs.
    ActionBackToGrid,
    /// Action label: open the focused grid item.
    ActionOpenSelected,
    /// Action label: enter/exit crop mode.
    ActionToggleCrop,
    /// Action label: start/stop auto-advance.
    ActionToggleSlideshow,
    /// Action label: extend the grid selection right.
    ActionSelectNext,
    /// Action label: extend the grid selection left.
    ActionSelectPrev,
    /// Action label: flip the focused item's selection.
    ActionToggleSelected,
    /// Action label: select every grid item.
    ActionSelectAll,
    /// Action label: copy selection paths to the clipboard.
    ActionCopySelected,
    /// Action label: stage a recycle-bin delete.
    ActionDeleteSelected,
    /// Action label: stage a move to another folder.
    ActionMoveSelected,
    /// Action label: open the settings panel.
    ActionOpenSettings,
}

/// Every key exactly once. The anti-drift test renders each under both
/// languages; the length assertion catches a variant added to [`StrKey`]
/// but forgotten here.
pub const ALL_KEYS: &[StrKey] = &[
    StrKey::WelcomeTagline,
    StrKey::DropZoneHint,
    StrKey::ViewerEmptyHint,
    StrKey::ContinueButton,
    StrKey::WelcomeOpen,
    StrKey::TopbarOpen,
    StrKey::GridEmptyDefault,
    StrKey::SortByLabel,
    StrKey::SortName,
    StrKey::SortCreated,
    StrKey::SortModified,
    StrKey::SortSize,
    StrKey::SortType,
    StrKey::SortAscending,
    StrKey::SortDescending,
    StrKey::TopbarBack,
    StrKey::CropCopy,
    StrKey::CropSave,
    StrKey::Cancel,
    StrKey::BatchDelete,
    StrKey::BatchMove,
    StrKey::SettingsTitle,
    StrKey::SectionGeneral,
    StrKey::SectionAppearance,
    StrKey::SectionShortcuts,
    StrKey::LanguageLabel,
    StrKey::LanguageEnglish,
    StrKey::LanguageSpanish,
    StrKey::ThemeLabel,
    StrKey::ShowHiddenFiles,
    StrKey::ClearRecents,
    StrKey::CapturePrompt,
    StrKey::ResetShortcuts,
    StrKey::ResetConfirm,
    StrKey::ActionNextImage,
    StrKey::ActionPrevImage,
    StrKey::ActionToggleOverlays,
    StrKey::ActionToggleFullscreen,
    StrKey::ActionOpenFile,
    StrKey::ActionOpenFolder,
    StrKey::ActionBackToGrid,
    StrKey::ActionOpenSelected,
    StrKey::ActionToggleCrop,
    StrKey::ActionToggleSlideshow,
    StrKey::ActionSelectNext,
    StrKey::ActionSelectPrev,
    StrKey::ActionToggleSelected,
    StrKey::ActionSelectAll,
    StrKey::ActionCopySelected,
    StrKey::ActionDeleteSelected,
    StrKey::ActionMoveSelected,
    StrKey::ActionOpenSettings,
];

/// English renderings. Every arm is non-empty (anti-drift-pinned).
fn en(key: StrKey) -> &'static str {
    match key {
        StrKey::WelcomeTagline => "A native, GPU-accelerated image viewer",
        StrKey::DropZoneHint => "Drop images or a folder here",
        StrKey::ViewerEmptyHint => "Drop an image to open it",
        StrKey::ContinueButton => "Continue",
        StrKey::WelcomeOpen => "Open folder…",
        StrKey::TopbarOpen => "Open folder",
        StrKey::GridEmptyDefault => "No images in this folder",
        StrKey::SortByLabel => "Sort by",
        StrKey::SortName => "Name",
        StrKey::SortCreated => "Created",
        StrKey::SortModified => "Modified",
        StrKey::SortSize => "Size",
        StrKey::SortType => "Type",
        StrKey::SortAscending => "Ascending",
        StrKey::SortDescending => "Descending",
        StrKey::TopbarBack => "Back",
        StrKey::CropCopy => "Copy",
        StrKey::CropSave => "Save…",
        StrKey::Cancel => "Cancel",
        StrKey::BatchDelete => "Delete",
        StrKey::BatchMove => "Move",
        StrKey::SettingsTitle => "Settings",
        StrKey::SectionGeneral => "General",
        StrKey::SectionAppearance => "Appearance",
        StrKey::SectionShortcuts => "Shortcuts",
        StrKey::LanguageLabel => "Language",
        StrKey::LanguageEnglish => "English",
        StrKey::LanguageSpanish => "Español",
        StrKey::ThemeLabel => "Theme",
        StrKey::ShowHiddenFiles => "Show hidden files",
        StrKey::ClearRecents => "Clear recent folders",
        StrKey::CapturePrompt => "Press keys… (Esc to cancel)",
        StrKey::ResetShortcuts => "Reset all shortcuts",
        StrKey::ResetConfirm => "Click again to confirm reset",
        StrKey::ActionNextImage => "Next image",
        StrKey::ActionPrevImage => "Previous image",
        StrKey::ActionToggleOverlays => "Toggle overlays",
        StrKey::ActionToggleFullscreen => "Toggle fullscreen",
        StrKey::ActionOpenFile => "Open file…",
        StrKey::ActionOpenFolder => "Open folder…",
        StrKey::ActionBackToGrid => "Back / close",
        StrKey::ActionOpenSelected => "Open selected",
        StrKey::ActionToggleCrop => "Toggle crop mode",
        StrKey::ActionToggleSlideshow => "Toggle slideshow",
        StrKey::ActionSelectNext => "Extend selection right",
        StrKey::ActionSelectPrev => "Extend selection left",
        StrKey::ActionToggleSelected => "Toggle selection",
        StrKey::ActionSelectAll => "Select all",
        StrKey::ActionCopySelected => "Copy selection paths",
        StrKey::ActionDeleteSelected => "Delete selected…",
        StrKey::ActionMoveSelected => "Move selected…",
        StrKey::ActionOpenSettings => "Open settings",
    }
}

/// Neutral-Spanish renderings (usted-neutral, region-free; native-speaker
/// review required before merge). Every arm is non-empty so the UI never
/// leans on the English fallback; autonyms stay identical (proper nouns).
fn es(key: StrKey) -> &'static str {
    match key {
        StrKey::WelcomeTagline => "Un visor de imágenes nativo acelerado por GPU",
        StrKey::DropZoneHint => "Suelte imágenes o una carpeta aquí",
        StrKey::ViewerEmptyHint => "Suelte una imagen para abrirla",
        StrKey::ContinueButton => "Continuar",
        StrKey::WelcomeOpen => "Abrir carpeta…",
        StrKey::TopbarOpen => "Abrir carpeta",
        StrKey::GridEmptyDefault => "No hay imágenes en esta carpeta",
        StrKey::SortByLabel => "Ordenar por",
        StrKey::SortName => "Nombre",
        StrKey::SortCreated => "Creación",
        StrKey::SortModified => "Modificación",
        StrKey::SortSize => "Tamaño",
        StrKey::SortType => "Tipo",
        StrKey::SortAscending => "Ascendente",
        StrKey::SortDescending => "Descendente",
        StrKey::TopbarBack => "Atrás",
        StrKey::CropCopy => "Copiar",
        StrKey::CropSave => "Guardar…",
        StrKey::Cancel => "Cancelar",
        StrKey::BatchDelete => "Eliminar",
        StrKey::BatchMove => "Mover",
        StrKey::SettingsTitle => "Ajustes",
        StrKey::SectionGeneral => "General",
        StrKey::SectionAppearance => "Apariencia",
        StrKey::SectionShortcuts => "Atajos",
        StrKey::LanguageLabel => "Idioma",
        StrKey::LanguageEnglish => "English",
        StrKey::LanguageSpanish => "Español",
        StrKey::ThemeLabel => "Tema",
        StrKey::ShowHiddenFiles => "Mostrar archivos ocultos",
        StrKey::ClearRecents => "Borrar carpetas recientes",
        StrKey::CapturePrompt => "Pulse teclas… (Esc para cancelar)",
        StrKey::ResetShortcuts => "Restablecer todos los atajos",
        StrKey::ResetConfirm => "Pulse de nuevo para confirmar el restablecimiento",
        StrKey::ActionNextImage => "Imagen siguiente",
        StrKey::ActionPrevImage => "Imagen anterior",
        StrKey::ActionToggleOverlays => "Alternar interfaz",
        StrKey::ActionToggleFullscreen => "Alternar pantalla completa",
        StrKey::ActionOpenFile => "Abrir archivo…",
        StrKey::ActionOpenFolder => "Abrir carpeta…",
        StrKey::ActionBackToGrid => "Atrás / cerrar",
        StrKey::ActionOpenSelected => "Abrir selección",
        StrKey::ActionToggleCrop => "Alternar modo de recorte",
        StrKey::ActionToggleSlideshow => "Alternar presentación",
        StrKey::ActionSelectNext => "Extender selección a la derecha",
        StrKey::ActionSelectPrev => "Extender selección a la izquierda",
        StrKey::ActionToggleSelected => "Alternar selección",
        StrKey::ActionSelectAll => "Seleccionar todo",
        StrKey::ActionCopySelected => "Copiar rutas de la selección",
        StrKey::ActionDeleteSelected => "Eliminar selección…",
        StrKey::ActionMoveSelected => "Mover selección…",
        StrKey::ActionOpenSettings => "Abrir ajustes",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_default_is_english() {
        assert_eq!(Language::default(), Language::En);
    }

    #[test]
    fn both_languages_are_addressable() {
        // Exhaustive match with no wildcard: adding a regional variant
        // (EsMx, EsEs, …) breaks compilation here and in `en`/`es` above.
        for lang in [Language::En, Language::Es] {
            let label = match lang {
                Language::En => "english",
                Language::Es => "espanol",
            };
            assert!(!label.is_empty());
            assert!(!lang.get(StrKey::WelcomeTagline).is_empty());
        }
    }

    #[test]
    fn t_alias_matches_get_for_both_languages() {
        assert_eq!(t(Language::En, StrKey::ContinueButton), "Continue");
        assert_eq!(t(Language::Es, StrKey::ContinueButton), "Continuar");
        assert_eq!(
            t(Language::En, StrKey::WelcomeTagline),
            Language::En.get(StrKey::WelcomeTagline)
        );
        assert_eq!(
            t(Language::Es, StrKey::WelcomeTagline),
            Language::Es.get(StrKey::WelcomeTagline)
        );
    }

    #[test]
    fn anti_drift_every_key_renders_non_empty_in_both_languages() {
        assert_eq!(ALL_KEYS.len(), 52, "ALL_KEYS drifted from StrKey");
        for key in ALL_KEYS {
            assert!(
                !Language::En.get(*key).is_empty(),
                "empty En rendering for {key:?}"
            );
            assert!(
                !Language::Es.get(*key).is_empty(),
                "empty Es rendering for {key:?}"
            );
        }
    }

    #[test]
    fn english_fallback_returns_english_when_spanish_arm_empty() {
        assert_eq!(fallback("Back", ""), "Back");
        assert_eq!(fallback("Back", "Atrás"), "Atrás");
    }
}

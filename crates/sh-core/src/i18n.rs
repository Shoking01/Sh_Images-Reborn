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

/// Plural category. `One` iff `n == 1` (zero selects `Other`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluralForm {
    /// Exactly one item.
    One,
    /// Zero or two-or-more items.
    Other,
}

/// Select the plural category for a count: `One` iff `n == 1`.
pub fn plural(n: usize) -> PluralForm {
    if n == 1 {
        PluralForm::One
    } else {
        PluralForm::Other
    }
}

/// Past-tense verb for batch reports. A typed enum (not `&str`) so each
/// language owns its full sentence including the verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchVerb {
    /// Files relocated to another folder.
    Moved,
    /// Files sent to the recycle bin.
    Deleted,
}

/// Batch result line, or `None` on full success (silent convention kept).
/// `first_failed` is a filename (proper noun, passed through untranslated).
/// Each language owns its full sentence: segments are skipped when their
/// count is zero and use the singular form when it is one.
pub fn batch_report(
    lang: Language,
    verb: BatchVerb,
    done: usize,
    total: usize,
    skipped: usize,
    failed: usize,
    first_failed: &str,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if skipped > 0 {
        parts.push(match lang {
            Language::En => format!("{skipped} skipped (already existed)"),
            Language::Es if skipped == 1 => "1 omitido (ya existía)".into(),
            Language::Es => format!("{skipped} omitidos (ya existían)"),
        });
    }
    if failed > 0 {
        parts.push(match lang {
            Language::En => format!("{failed} failed ({first_failed})"),
            Language::Es if failed == 1 => format!("1 con error ({first_failed})"),
            Language::Es => format!("{failed} con errores ({first_failed})"),
        });
    }
    if parts.is_empty() {
        return None;
    }
    let head = match (lang, verb) {
        (Language::En, BatchVerb::Moved) => format!("Moved {done} of {total}"),
        (Language::En, BatchVerb::Deleted) => format!("Deleted {done} of {total}"),
        (Language::Es, BatchVerb::Moved) if done == 1 => format!("Se movió 1 de {total}"),
        (Language::Es, BatchVerb::Deleted) if done == 1 => format!("Se eliminó 1 de {total}"),
        (Language::Es, BatchVerb::Moved) => format!("Se movieron {done} de {total}"),
        (Language::Es, BatchVerb::Deleted) => format!("Se eliminaron {done} de {total}"),
    };
    let sep = match lang {
        Language::En => " — ",
        Language::Es => ": ",
    };
    Some(format!("{head}{sep}{}", parts.join(", ")))
}

/// Staged-delete confirm bar: count with `n == 1` singular selection.
pub fn batch_bar_delete(lang: Language, n: usize) -> String {
    match lang {
        Language::En if n == 1 => "Delete 1 file to recycle bin?".into(),
        Language::En => format!("Delete {n} files to recycle bin?"),
        Language::Es if n == 1 => "¿Mover 1 archivo a la papelera?".into(),
        Language::Es => format!("¿Mover {n} archivos a la papelera?"),
    }
}

/// Staged-move confirm bar: count plus destination display name
/// (`dest` is a folder name — proper noun, passed through untranslated).
pub fn batch_bar_move(lang: Language, n: usize, dest: &str) -> String {
    match lang {
        Language::En if n == 1 => format!("Move 1 file to {dest}?"),
        Language::En => format!("Move {n} files to {dest}?"),
        Language::Es if n == 1 => format!("¿Mover 1 archivo a {dest}?"),
        Language::Es => format!("¿Mover {n} archivos a {dest}?"),
    }
}

/// `"Recent folders (N)"` header for the General section.
pub fn recents_header(lang: Language, count: usize) -> String {
    match lang {
        Language::En => format!("Recent folders ({count})"),
        Language::Es => format!("Carpetas recientes ({count})"),
    }
}

/// `"Already used by {label}"` shortcut-conflict message.
/// `incumbent` is the resolved action label (already localized by the caller).
pub fn conflict_text(lang: Language, incumbent: &str) -> String {
    match lang {
        Language::En => format!("Already used by {incumbent}"),
        Language::Es => format!("Ya en uso por {incumbent}"),
    }
}

/// Grid empty-state with the folder path interpolated.
/// `dir` is a path (proper noun, passed through untranslated).
pub fn no_images_in(lang: Language, dir: &str) -> String {
    match lang {
        Language::En => format!("No images in {dir}"),
        Language::Es => format!("No hay imágenes en {dir}"),
    }
}

/// Grid multi-selection suffix (`" (N selected)"`, empty when `n == 0`).
pub fn selected_suffix(lang: Language, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    match lang {
        Language::En => format!(" ({n} selected)"),
        Language::Es if n == 1 => " (1 seleccionado)".into(),
        Language::Es => format!(" ({n} seleccionados)"),
    }
}

/// Theme Editor: the destination for a built-in's user copy is occupied.
///
/// Not a generic "that name is taken": after the replace path landed, the only
/// way to reach a name collision is materializing a built-in's copy, so the
/// message has to name THAT situation or it will read as a bug in the editor.
/// `file_name` is a proper noun and passes through untranslated.
pub fn theme_copy_taken(lang: Language, file_name: &str) -> String {
    match lang {
        Language::En => format!("Can't copy to {file_name}: the name is already taken"),
        Language::Es => format!("No se puede copiar a {file_name}: el nombre ya existe"),
    }
}

/// Theme Editor: the file being edited is gone from disk.
///
/// Its own message rather than a reworded "already exists": the two outcomes
/// are opposites, and telling a user whose theme was deleted under them that a
/// name is taken sends them looking for a collision that does not exist.
pub fn theme_target_missing(lang: Language, file_name: &str) -> String {
    match lang {
        Language::En => format!("{file_name} no longer exists"),
        Language::Es => format!("{file_name} ya no existe"),
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
    /// Settings section: the per-slot theme color editor.
    SectionThemeEditor,
    /// General picker label.
    LanguageLabel,
    /// Picker autonym (proper noun: identical in both languages).
    LanguageEnglish,
    /// Picker autonym (proper noun: identical in both languages).
    LanguageSpanish,
    /// Appearance row: active theme.
    ThemeLabel,
    /// Appearance row: filmstrip visibility toggle.
    FilmstripLabel,
    /// Appearance row: transparency checkerboard toggle.
    CheckerboardLabel,
    /// Appearance row: slideshow interval control.
    SlideshowIntervalLabel,
    /// Appearance row: reduced-motion toggle.
    ReduceMotionLabel,
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
    /// Grid density chip: small preset label.
    GridSizeSmall,
    /// Grid density chip: medium preset label.
    GridSizeMedium,
    /// Grid density chip: large preset label.
    GridSizeLarge,
    /// Viewer zoom chip: fit-to-viewport preset label.
    ZoomPresetFit,
    /// Viewer zoom chip: actual-pixels preset label (locale-neutral numeral).
    ZoomPreset100,
    /// Viewer zoom chip: double-size preset label (locale-neutral numeral).
    ZoomPreset200,
    /// Viewer info-panel row: pixel dimensions label.
    InfoDimensionsLabel,
    /// Viewer info-panel row: file-size label.
    InfoFileSizeLabel,
    /// Viewer info-panel row: format label.
    InfoFormatLabel,
    /// Viewer overlay info button accessible label.
    InfoButtonLabel,
    /// Viewer info-panel error when facts cannot be resolved.
    InfoLoadError,
    /// Theme Editor header action: write the draft to its target.
    ///
    /// A distinct key from [`CropSave`](Self::CropSave) on purpose. `CropSave`
    /// renders `Save…`, and the ellipsis is load-bearing there: it promises a
    /// save DIALOG. This button writes immediately and opens nothing, so
    /// borrowing the crop key promised a second click that never comes.
    ThemeEditorSave,
    /// Accessible label for the `✎` glyph that opens the Theme Editor.
    ///
    /// The glyph is one character with no text of its own, so without this it
    /// is announced to a screen reader as an unlabelled button.
    ThemeEditAction,
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
    StrKey::ThemeEditorSave,
    StrKey::ThemeEditAction,
    StrKey::Cancel,
    StrKey::BatchDelete,
    StrKey::BatchMove,
    StrKey::SettingsTitle,
    StrKey::SectionGeneral,
    StrKey::SectionAppearance,
    StrKey::SectionShortcuts,
    StrKey::SectionThemeEditor,
    StrKey::LanguageLabel,
    StrKey::LanguageEnglish,
    StrKey::LanguageSpanish,
    StrKey::ThemeLabel,
    StrKey::FilmstripLabel,
    StrKey::CheckerboardLabel,
    StrKey::SlideshowIntervalLabel,
    StrKey::ReduceMotionLabel,
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
    StrKey::GridSizeSmall,
    StrKey::GridSizeMedium,
    StrKey::GridSizeLarge,
    StrKey::ZoomPresetFit,
    StrKey::ZoomPreset100,
    StrKey::ZoomPreset200,
    StrKey::InfoDimensionsLabel,
    StrKey::InfoFileSizeLabel,
    StrKey::InfoFormatLabel,
    StrKey::InfoButtonLabel,
    StrKey::InfoLoadError,
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
        StrKey::ThemeEditorSave => "Save",
        StrKey::ThemeEditAction => "Edit theme",
        StrKey::Cancel => "Cancel",
        StrKey::BatchDelete => "Delete",
        StrKey::BatchMove => "Move",
        StrKey::SettingsTitle => "Settings",
        StrKey::SectionGeneral => "General",
        StrKey::SectionAppearance => "Appearance",
        StrKey::SectionShortcuts => "Shortcuts",
        StrKey::SectionThemeEditor => "Theme editor",
        StrKey::LanguageLabel => "Language",
        StrKey::LanguageEnglish => "English",
        StrKey::LanguageSpanish => "Español",
        StrKey::ThemeLabel => "Theme",
        StrKey::FilmstripLabel => "Filmstrip",
        StrKey::CheckerboardLabel => "Transparency checkerboard",
        StrKey::SlideshowIntervalLabel => "Slideshow interval",
        StrKey::ReduceMotionLabel => "Reduce motion",
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
        StrKey::GridSizeSmall => "Small",
        StrKey::GridSizeMedium => "Medium",
        StrKey::GridSizeLarge => "Large",
        StrKey::ZoomPresetFit => "Fit",
        StrKey::ZoomPreset100 => "100%",
        StrKey::ZoomPreset200 => "200%",
        StrKey::InfoDimensionsLabel => "Dimensions",
        StrKey::InfoFileSizeLabel => "Size",
        StrKey::InfoFormatLabel => "Format",
        StrKey::InfoButtonLabel => "Show image info",
        StrKey::InfoLoadError => "Could not read image info",
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
        StrKey::ThemeEditorSave => "Guardar",
        StrKey::ThemeEditAction => "Editar tema",
        StrKey::Cancel => "Cancelar",
        StrKey::BatchDelete => "Eliminar",
        StrKey::BatchMove => "Mover",
        StrKey::SettingsTitle => "Ajustes",
        StrKey::SectionGeneral => "General",
        StrKey::SectionAppearance => "Apariencia",
        StrKey::SectionShortcuts => "Atajos",
        StrKey::SectionThemeEditor => "Editor de temas",
        StrKey::LanguageLabel => "Idioma",
        StrKey::LanguageEnglish => "English",
        StrKey::LanguageSpanish => "Español",
        StrKey::ThemeLabel => "Tema",
        StrKey::FilmstripLabel => "Franja de película",
        StrKey::CheckerboardLabel => "Damero de transparencia",
        StrKey::SlideshowIntervalLabel => "Intervalo de presentación",
        StrKey::ReduceMotionLabel => "Reducir movimiento",
        StrKey::ShowHiddenFiles => "Mostrar archivos ocultos",
        StrKey::ClearRecents => "Borrar carpetas recientes",
        StrKey::CapturePrompt => "Pulse teclas… (Esc para cancelar)",
        StrKey::ResetShortcuts => "Restablecer todos los atajos",
        StrKey::ResetConfirm => "Pulse de nuevo para confirmar",
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
        StrKey::GridSizeSmall => "Pequeño",
        StrKey::GridSizeMedium => "Mediano",
        StrKey::GridSizeLarge => "Grande",
        // "Ajustar": infinitive, matches the action-label idiom (Abrir/
        // Alternar). The numerals stay locale-neutral glyphs (see the
        // exemption test) — inventing word-forms would harm scannability.
        StrKey::ZoomPresetFit => "Ajustar",
        StrKey::ZoomPreset100 => "100%",
        StrKey::ZoomPreset200 => "200%",
        StrKey::InfoDimensionsLabel => "Dimensiones",
        StrKey::InfoFileSizeLabel => "Tamaño",
        StrKey::InfoFormatLabel => "Formato",
        StrKey::InfoButtonLabel => "Mostrar información de la imagen",
        StrKey::InfoLoadError => "No se pudo leer la información de la imagen",
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

    /// The Theme Editor's save button must not inherit the crop bar's ellipsis.
    /// `CropSave` promises a dialog; this action writes immediately, so the
    /// difference is the whole reason the key exists.
    #[test]
    fn the_theme_editor_save_label_promises_no_dialog() {
        assert_eq!(Language::En.get(StrKey::ThemeEditorSave), "Save");
        assert_eq!(Language::Es.get(StrKey::ThemeEditorSave), "Guardar");
        assert_ne!(
            Language::En.get(StrKey::ThemeEditorSave),
            Language::En.get(StrKey::CropSave),
            "a button that opens no dialog must not render the dialog ellipsis"
        );
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

    /// The two Theme Editor refusals must read as opposites, not as variants of
    /// one another: a user whose theme was deleted must not be sent hunting for
    /// a name collision that does not exist.
    #[test]
    fn the_two_theme_editor_refusals_name_different_problems() {
        let taken_en = theme_copy_taken(Language::En, "deep-neutral.json");
        let missing_en = theme_target_missing(Language::En, "deep-neutral.json");
        assert!(taken_en.contains("deep-neutral.json"));
        assert!(missing_en.contains("deep-neutral.json"));
        assert_ne!(taken_en, missing_en, "the two refusals must differ");
        assert!(missing_en.contains("no longer exists"));
        assert!(!missing_en.contains("already taken"));

        // Spanish must be translated, not an English passthrough.
        assert_ne!(
            theme_copy_taken(Language::Es, "a.json"),
            theme_copy_taken(Language::En, "a.json")
        );
        assert_ne!(
            theme_target_missing(Language::Es, "a.json"),
            theme_target_missing(Language::En, "a.json")
        );
    }

    #[test]
    fn anti_drift_every_key_renders_non_empty_in_both_languages() {
        assert_eq!(ALL_KEYS.len(), 70, "ALL_KEYS drifted from StrKey");
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
    fn settings_appearance_labels_are_localized() {
        assert_eq!(Language::En.get(StrKey::FilmstripLabel), "Filmstrip");
        assert_eq!(
            Language::Es.get(StrKey::FilmstripLabel),
            "Franja de película"
        );
        assert_eq!(
            Language::En.get(StrKey::CheckerboardLabel),
            "Transparency checkerboard"
        );
        assert_eq!(
            Language::Es.get(StrKey::CheckerboardLabel),
            "Damero de transparencia"
        );
        assert_eq!(
            Language::En.get(StrKey::SlideshowIntervalLabel),
            "Slideshow interval"
        );
        assert_eq!(
            Language::Es.get(StrKey::SlideshowIntervalLabel),
            "Intervalo de presentación"
        );
        assert_eq!(Language::En.get(StrKey::ReduceMotionLabel), "Reduce motion");
        assert_eq!(
            Language::Es.get(StrKey::ReduceMotionLabel),
            "Reducir movimiento"
        );
    }

    #[test]
    fn english_fallback_returns_english_when_spanish_arm_empty() {
        assert_eq!(fallback("Back", ""), "Back");
        assert_eq!(fallback("Back", "Atrás"), "Atrás");
    }

    // ── Zoomable grid: S/M/L chip labels ──

    #[test]
    fn grid_size_chip_labels_render_non_empty_english() {
        for key in [
            StrKey::GridSizeSmall,
            StrKey::GridSizeMedium,
            StrKey::GridSizeLarge,
        ] {
            assert!(
                !Language::En.get(key).is_empty(),
                "empty En rendering for {key:?}"
            );
        }
        assert_eq!(Language::En.get(StrKey::GridSizeSmall), "Small");
        assert_eq!(Language::En.get(StrKey::GridSizeMedium), "Medium");
        assert_eq!(Language::En.get(StrKey::GridSizeLarge), "Large");
    }

    #[test]
    fn grid_size_chip_labels_render_neutral_spanish() {
        // Neutral-Spanish words: non-empty, and never an English substring
        // (bare S/M/L letters would be identical in both languages).
        for key in [
            StrKey::GridSizeSmall,
            StrKey::GridSizeMedium,
            StrKey::GridSizeLarge,
        ] {
            let en = Language::En.get(key);
            let es = Language::Es.get(key);
            assert!(!es.is_empty(), "empty Es rendering for {key:?}");
            assert_ne!(es, en, "Es rendering equals English for {key:?}");
            assert!(
                !es.contains(en),
                "Es rendering contains English for {key:?}: {es:?}"
            );
        }
        assert_eq!(Language::Es.get(StrKey::GridSizeSmall), "Pequeño");
        assert_eq!(Language::Es.get(StrKey::GridSizeMedium), "Mediano");
        assert_eq!(Language::Es.get(StrKey::GridSizeLarge), "Grande");
    }

    #[test]
    fn grid_size_chip_labels_fall_back_to_english_when_spanish_arm_empty() {
        // The never-blank-UI rule holds for the new keys: an emptied Spanish
        // arm renders English (proves the fallback path with real key text).
        for key in [
            StrKey::GridSizeSmall,
            StrKey::GridSizeMedium,
            StrKey::GridSizeLarge,
        ] {
            assert_eq!(fallback(en(key), ""), en(key));
        }
    }

    // ── Zoom-preset chip labels (viewer-zoom-presets, Phase 2) ──

    #[test]
    fn zoom_preset_chip_labels_render_non_empty_english() {
        for key in [
            StrKey::ZoomPresetFit,
            StrKey::ZoomPreset100,
            StrKey::ZoomPreset200,
        ] {
            assert!(
                !Language::En.get(key).is_empty(),
                "empty En rendering for {key:?}"
            );
        }
        assert_eq!(Language::En.get(StrKey::ZoomPresetFit), "Fit");
        assert_eq!(Language::En.get(StrKey::ZoomPreset100), "100%");
        assert_eq!(Language::En.get(StrKey::ZoomPreset200), "200%");
    }

    #[test]
    fn zoom_preset_fit_renders_neutral_spanish() {
        // The only translatable word of the four: infinitive "Ajustar"
        // matches the action-label idiom (Abrir/Alternar). Non-empty, not an
        // English substring — same rules as the grid-chip words.
        let en = Language::En.get(StrKey::ZoomPresetFit);
        let es = Language::Es.get(StrKey::ZoomPresetFit);
        assert_eq!(es, "Ajustar");
        assert!(!es.is_empty());
        assert_ne!(es, en);
        assert!(!es.contains(en));
    }

    #[test]
    fn zoom_preset_numeral_labels_are_locale_neutral() {
        // EXEMPTION (not a weakening): numeral percentages are untranslatable
        // glyphs, so Es == En BY DESIGN — same precedent as the ↑/↓ arrows in
        // `sort_chip_label`. A silent weakening of the anti-drift not-equal
        // check would hide this; documenting it here keeps the drift alarm
        // for everything else.
        for key in [StrKey::ZoomPreset100, StrKey::ZoomPreset200] {
            let en = Language::En.get(key);
            let es = Language::Es.get(key);
            assert!(!en.is_empty());
            assert_eq!(
                es, en,
                "numeral labels must stay locale-neutral for {key:?}"
            );
        }
        assert_eq!(Language::Es.get(StrKey::ZoomPreset100), "100%");
        assert_eq!(Language::Es.get(StrKey::ZoomPreset200), "200%");
    }

    #[test]
    fn zoom_preset_chip_labels_fall_back_to_english_when_spanish_arm_empty() {
        // The never-blank-UI rule holds for the new keys too.
        for key in [
            StrKey::ZoomPresetFit,
            StrKey::ZoomPreset100,
            StrKey::ZoomPreset200,
        ] {
            assert_eq!(fallback(en(key), ""), en(key));
        }
    }

    // ── Viewer info-panel labels ──

    #[test]
    fn info_panel_labels_render_non_empty_english() {
        for key in [
            StrKey::InfoDimensionsLabel,
            StrKey::InfoFileSizeLabel,
            StrKey::InfoFormatLabel,
            StrKey::InfoButtonLabel,
            StrKey::InfoLoadError,
        ] {
            assert!(
                !Language::En.get(key).is_empty(),
                "empty En rendering for {key:?}"
            );
        }
        assert_eq!(Language::En.get(StrKey::InfoDimensionsLabel), "Dimensions");
        assert_eq!(Language::En.get(StrKey::InfoFileSizeLabel), "Size");
        assert_eq!(Language::En.get(StrKey::InfoFormatLabel), "Format");
        assert_eq!(Language::En.get(StrKey::InfoButtonLabel), "Show image info");
        assert_eq!(
            Language::En.get(StrKey::InfoLoadError),
            "Could not read image info"
        );
    }

    #[test]
    fn info_panel_labels_render_neutral_spanish() {
        // Neutral-Spanish words: non-empty, never equal to the English
        // rendering, and never a substring of it (Es ⊄ En — the anti-drift
        // direction). NOTE on direction: the grid-chip blocks assert
        // `!es.contains(en)`, but that direction cannot hold here —
        // `"Formato"` necessarily starts with `"Format"` (cognate overlap,
        // not drift). Asserting Es-is-not-a-substring-of-En keeps a real
        // drift tripwire (truncated/degenerate Es arms) while the mandated
        // neutral wording stays intact.
        for key in [
            StrKey::InfoDimensionsLabel,
            StrKey::InfoFileSizeLabel,
            StrKey::InfoFormatLabel,
            StrKey::InfoButtonLabel,
            StrKey::InfoLoadError,
        ] {
            let en = Language::En.get(key);
            let es = Language::Es.get(key);
            assert!(!es.is_empty(), "empty Es rendering for {key:?}");
            assert_ne!(es, en, "Es rendering equals English for {key:?}");
            assert!(
                !en.contains(es),
                "Es rendering is an English substring for {key:?}: {es:?}"
            );
        }
        assert_eq!(Language::Es.get(StrKey::InfoDimensionsLabel), "Dimensiones");
        assert_eq!(Language::Es.get(StrKey::InfoFileSizeLabel), "Tamaño");
        assert_eq!(Language::Es.get(StrKey::InfoFormatLabel), "Formato");
        assert_eq!(
            Language::Es.get(StrKey::InfoButtonLabel),
            "Mostrar información de la imagen"
        );
        assert_eq!(
            Language::Es.get(StrKey::InfoLoadError),
            "No se pudo leer la información de la imagen"
        );
    }

    #[test]
    fn info_panel_labels_fall_back_to_english_when_spanish_arm_empty() {
        // The never-blank-UI rule holds for the new keys: an emptied Spanish
        // arm renders English (proves the fallback path with real key text).
        for key in [
            StrKey::InfoDimensionsLabel,
            StrKey::InfoFileSizeLabel,
            StrKey::InfoFormatLabel,
            StrKey::InfoButtonLabel,
            StrKey::InfoLoadError,
        ] {
            assert_eq!(fallback(en(key), ""), en(key));
        }
    }

    #[test]
    fn plural_selects_one_only_for_single_item() {
        assert_eq!(plural(1), PluralForm::One);
    }

    #[test]
    fn plural_selects_other_for_zero_two_and_large_counts() {
        assert_eq!(plural(0), PluralForm::Other);
        assert_eq!(plural(2), PluralForm::Other);
        assert_eq!(plural(1_000_000), PluralForm::Other);
    }

    proptest::proptest! {
        #[test]
        fn plural_is_one_iff_count_is_one(n in proptest::prelude::any::<usize>()) {
            assert_eq!(plural(n) == PluralForm::One, n == 1);
        }
    }

    #[test]
    fn batch_report_silent_on_full_success_in_both_languages() {
        for lang in [Language::En, Language::Es] {
            for verb in [BatchVerb::Moved, BatchVerb::Deleted] {
                assert_eq!(
                    batch_report(lang, verb, 3, 3, 0, 0, ""),
                    None,
                    "full success is silent"
                );
            }
        }
    }

    #[test]
    fn batch_report_full_sentences_with_skips_and_failures() {
        assert_eq!(
            batch_report(Language::En, BatchVerb::Moved, 1, 3, 1, 1, "c.png"),
            Some("Moved 1 of 3 — 1 skipped (already existed), 1 failed (c.png)".into())
        );
        assert_eq!(
            batch_report(Language::En, BatchVerb::Deleted, 0, 2, 0, 2, "a.png"),
            Some("Deleted 0 of 2 — 2 failed (a.png)".into())
        );
        assert_eq!(
            batch_report(Language::Es, BatchVerb::Moved, 1, 3, 1, 1, "c.png"),
            // Native-speaker review: the reflexive verb agrees with the
            // numeral — 1 takes singular (`Se movió`), 0 and N take plural.
            Some("Se movió 1 de 3: 1 omitido (ya existía), 1 con error (c.png)".into())
        );
        assert_eq!(
            batch_report(Language::Es, BatchVerb::Deleted, 0, 2, 0, 2, "a.png"),
            Some("Se eliminaron 0 de 2: 2 con errores (a.png)".into())
        );
        assert_eq!(
            batch_report(Language::Es, BatchVerb::Moved, 3, 5, 2, 0, ""),
            Some("Se movieron 3 de 5: 2 omitidos (ya existían)".into())
        );
        assert_eq!(
            batch_report(Language::Es, BatchVerb::Deleted, 1, 4, 0, 3, "b.png"),
            Some("Se eliminó 1 de 4: 3 con errores (b.png)".into())
        );
    }

    #[test]
    fn batch_confirm_bars_are_complete_sentences() {
        assert_eq!(
            batch_bar_delete(Language::En, 1),
            "Delete 1 file to recycle bin?"
        );
        assert_eq!(
            batch_bar_delete(Language::En, 3),
            "Delete 3 files to recycle bin?"
        );
        assert_eq!(
            batch_bar_delete(Language::Es, 1),
            // Native-speaker review: `eliminar` does not govern "a la
            // papelera" — the trash semantics take `mover a`.
            "¿Mover 1 archivo a la papelera?"
        );
        assert_eq!(
            batch_bar_delete(Language::Es, 3),
            "¿Mover 3 archivos a la papelera?"
        );
        assert_eq!(
            batch_bar_move(Language::En, 2, "Fotos"),
            "Move 2 files to Fotos?"
        );
        assert_eq!(
            batch_bar_move(Language::Es, 1, "Fotos"),
            "¿Mover 1 archivo a Fotos?"
        );
        assert_eq!(
            batch_bar_move(Language::Es, 2, "Fotos"),
            "¿Mover 2 archivos a Fotos?"
        );
    }

    #[test]
    fn recents_conflict_empty_state_and_suffix_are_complete_sentences() {
        assert_eq!(recents_header(Language::En, 3), "Recent folders (3)");
        assert_eq!(recents_header(Language::Es, 3), "Carpetas recientes (3)");
        assert_eq!(
            conflict_text(Language::En, "Next image"),
            "Already used by Next image"
        );
        assert_eq!(
            conflict_text(Language::Es, "Imagen siguiente"),
            "Ya en uso por Imagen siguiente"
        );
        assert_eq!(
            no_images_in(Language::En, "C:\\Fotos"),
            "No images in C:\\Fotos"
        );
        assert_eq!(
            no_images_in(Language::Es, "C:\\Fotos"),
            "No hay imágenes en C:\\Fotos"
        );
        assert_eq!(selected_suffix(Language::En, 0), "");
        assert_eq!(selected_suffix(Language::En, 2), " (2 selected)");
        assert_eq!(selected_suffix(Language::Es, 0), "");
        assert_eq!(selected_suffix(Language::Es, 1), " (1 seleccionado)");
        assert_eq!(selected_suffix(Language::Es, 2), " (2 seleccionados)");
    }
}

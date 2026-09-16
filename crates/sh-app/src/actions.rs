//! Keyboard actions for the viewer.

use gpui::actions;

actions!(
    sh_images,
    [
        NextImage,
        PrevImage,
        ToggleOverlays,
        ToggleFullscreen,
        OpenFile,
        OpenFolder,
        BackToGrid,
        OpenSelected,
        ToggleCrop,
        CropCopy,
        CropSave,
        CropCancel,
        ToggleSlideshow,
        SelectNext,
        SelectPrev,
        ToggleSelected,
        SelectAll,
        CopySelected,
        DeleteSelected,
        MoveSelected,
        OpenSettings,
    ]
);

/// User-facing descriptor for one keymap-dispatched action.
///
/// `id` is the key in `settings.json`'s keymap object. `context` is the
/// display group in the Shortcuts section ("Viewer" | "Grid" | "Global").
/// The real gpui keymap context for conflict detection + registration is the
/// `KEYMAP_CONTEXT` constant below — every V1 action registers there.
pub struct ActionDescriptor {
    /// Stable id, e.g. `"next-image"` — key in settings.json keymap.
    pub id: &'static str,
    /// Display label, e.g. `"Next image"` (English until i18n).
    pub label: &'static str,
    /// Display group in the Shortcuts section.
    pub context: &'static str,
}

/// The single gpui keymap context every V1 binding registers under.
/// Matches the existing `key_context("image_view")` on the app root div;
/// `validate_binding` callers pass this so conflict detection is scoped by
/// the REAL context rather than the display label.
pub const KEYMAP_CONTEXT: &str = "image_view";

/// Every keymap-dispatched action: descriptor source of truth shared by
/// `resolve_bindings` (startup + live rebind), the Shortcuts UI, and the
/// anti-drift test. `CropCopy`/`CropSave`/`CropCancel` have no key binding
/// registered and are excluded by design.
pub const ACTIONS: &[ActionDescriptor] = &[
    ActionDescriptor {
        id: "next-image",
        label: "Next image",
        context: "Viewer",
    },
    ActionDescriptor {
        id: "prev-image",
        label: "Previous image",
        context: "Viewer",
    },
    ActionDescriptor {
        id: "toggle-overlays",
        label: "Toggle overlays",
        context: "Viewer",
    },
    ActionDescriptor {
        id: "toggle-fullscreen",
        label: "Toggle fullscreen",
        context: "Global",
    },
    ActionDescriptor {
        id: "open-file",
        label: "Open file…",
        context: "Global",
    },
    ActionDescriptor {
        id: "open-folder",
        label: "Open folder…",
        context: "Global",
    },
    ActionDescriptor {
        id: "back-to-grid",
        label: "Back / close",
        context: "Global",
    },
    ActionDescriptor {
        id: "open-selected",
        label: "Open selected",
        context: "Grid",
    },
    ActionDescriptor {
        id: "toggle-crop",
        label: "Toggle crop mode",
        context: "Viewer",
    },
    ActionDescriptor {
        id: "toggle-slideshow",
        label: "Toggle slideshow",
        context: "Viewer",
    },
    ActionDescriptor {
        id: "select-next",
        label: "Extend selection right",
        context: "Grid",
    },
    ActionDescriptor {
        id: "select-prev",
        label: "Extend selection left",
        context: "Grid",
    },
    ActionDescriptor {
        id: "toggle-selected",
        label: "Toggle selection",
        context: "Grid",
    },
    ActionDescriptor {
        id: "select-all",
        label: "Select all",
        context: "Grid",
    },
    ActionDescriptor {
        id: "copy-selected",
        label: "Copy selection paths",
        context: "Grid",
    },
    ActionDescriptor {
        id: "delete-selected",
        label: "Delete selected…",
        context: "Grid",
    },
    ActionDescriptor {
        id: "move-selected",
        label: "Move selected…",
        context: "Grid",
    },
    ActionDescriptor {
        id: "open-settings",
        label: "Open settings",
        context: "Global",
    },
];

/// Build live `gpui::KeyBinding`s from a stored keymap.
///
/// The `match` on `id` is the single dispatch bridge: descriptor ids,
/// bindings, and concrete action types meet here, so no stringly-typed drift
/// is possible between the Shortcuts UI, settings.json, and dispatch.
/// Unknown ids are SKIPPED (forward-compat), never panicked on.
pub fn resolve_bindings(keymap: &sh_core::keymap::Keymap) -> Vec<gpui::KeyBinding> {
    let mut out = Vec::with_capacity(keymap.len());
    for (id, binding) in keymap {
        let keystrokes = binding.to_keymap_string();
        let kb = match id.as_str() {
            "next-image" => gpui::KeyBinding::new(&keystrokes, NextImage, Some(KEYMAP_CONTEXT)),
            "prev-image" => gpui::KeyBinding::new(&keystrokes, PrevImage, Some(KEYMAP_CONTEXT)),
            "toggle-overlays" => {
                gpui::KeyBinding::new(&keystrokes, ToggleOverlays, Some(KEYMAP_CONTEXT))
            }
            "toggle-fullscreen" => {
                gpui::KeyBinding::new(&keystrokes, ToggleFullscreen, Some(KEYMAP_CONTEXT))
            }
            "open-file" => gpui::KeyBinding::new(&keystrokes, OpenFile, Some(KEYMAP_CONTEXT)),
            "open-folder" => gpui::KeyBinding::new(&keystrokes, OpenFolder, Some(KEYMAP_CONTEXT)),
            "back-to-grid" => gpui::KeyBinding::new(&keystrokes, BackToGrid, Some(KEYMAP_CONTEXT)),
            "open-selected" => {
                gpui::KeyBinding::new(&keystrokes, OpenSelected, Some(KEYMAP_CONTEXT))
            }
            "toggle-crop" => gpui::KeyBinding::new(&keystrokes, ToggleCrop, Some(KEYMAP_CONTEXT)),
            "toggle-slideshow" => {
                gpui::KeyBinding::new(&keystrokes, ToggleSlideshow, Some(KEYMAP_CONTEXT))
            }
            "select-next" => gpui::KeyBinding::new(&keystrokes, SelectNext, Some(KEYMAP_CONTEXT)),
            "select-prev" => gpui::KeyBinding::new(&keystrokes, SelectPrev, Some(KEYMAP_CONTEXT)),
            "toggle-selected" => {
                gpui::KeyBinding::new(&keystrokes, ToggleSelected, Some(KEYMAP_CONTEXT))
            }
            "select-all" => gpui::KeyBinding::new(&keystrokes, SelectAll, Some(KEYMAP_CONTEXT)),
            "copy-selected" => {
                gpui::KeyBinding::new(&keystrokes, CopySelected, Some(KEYMAP_CONTEXT))
            }
            "delete-selected" => {
                gpui::KeyBinding::new(&keystrokes, DeleteSelected, Some(KEYMAP_CONTEXT))
            }
            "move-selected" => {
                gpui::KeyBinding::new(&keystrokes, MoveSelected, Some(KEYMAP_CONTEXT))
            }
            "open-settings" => {
                gpui::KeyBinding::new(&keystrokes, OpenSettings, Some(KEYMAP_CONTEXT))
            }
            _ => continue,
        };
        out.push(kb);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-table anti-drift: every id in ACTIONS has a default binding,
    /// and no default is orphaned. Lives in sh-app because only sh-app sees
    /// both tables.
    #[test]
    fn actions_and_defaults_cover_each_other_exactly() {
        let d = sh_core::keymap::defaults();
        assert_eq!(ACTIONS.len(), d.len(), "ACTIONS and defaults() drifted");
        for a in ACTIONS {
            assert!(d.contains_key(a.id), "defaults() missing {}", a.id);
        }
        for id in d.keys() {
            assert!(
                ACTIONS.iter().any(|a| a.id == id.as_str()),
                "ACTIONS missing {id}"
            );
        }
        let resolved = resolve_bindings(&d);
        assert_eq!(resolved.len(), ACTIONS.len());
        let mut with_unknown = d.clone();
        with_unknown.insert(
            "future-action".into(),
            sh_core::keymap::KeyBinding {
                ctrl: true,
                shift: false,
                alt: false,
                platform: false,
                key: "z".into(),
            },
        );
        assert_eq!(resolve_bindings(&with_unknown).len(), ACTIONS.len());
    }
}

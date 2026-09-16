//! Rebindable keyboard shortcuts: bindings, defaults, and conflict validation.
//!
//! Pure data + validation with no GPUI dependency. `defaults()` is the single
//! source of truth for every keymap-dispatched action id.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A single rebindable key combination.
///
/// `key` holds the gpui keystroke token: a printable char (`"o"`, `","`),
/// or a named key (`"left"`, `"escape"`, `"space"`, `"f11"`, `"delete"`,
/// `"enter"`, `"tab"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBinding {
    /// `ctrl` modifier held.
    pub ctrl: bool,
    /// `shift` modifier held.
    pub shift: bool,
    /// `alt` modifier held.
    pub alt: bool,
    /// `cmd` on macOS / `super` on Linux.
    pub platform: bool,
    /// gpui keystroke key token, e.g. `"o"`, `"left"`, `","`.
    pub key: String,
}

/// Action id → binding. `BTreeMap` for stable render order in the panel.
pub type Keymap = BTreeMap<String, KeyBinding>;

impl KeyBinding {
    /// Emit gpui keymap syntax (`"ctrl-shift-o"`, `"right"`, `"ctrl-,"`).
    /// Emission order is fixed: ctrl → shift → alt → cmd → key.
    /// Output always parses via gpui's `Keystroke::parse` by construction:
    /// modifiers are a fixed vocabulary and `key` originates from either
    /// `defaults()` literals or a real key event.
    pub fn to_keymap_string(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("ctrl-");
        }
        if self.shift {
            s.push_str("shift-");
        }
        if self.alt {
            s.push_str("alt-");
        }
        if self.platform {
            s.push_str("cmd-");
        }
        s.push_str(&self.key);
        s
    }
}

fn binding(ctrl: bool, shift: bool, alt: bool, platform: bool, key: &str) -> KeyBinding {
    KeyBinding {
        ctrl,
        shift,
        alt,
        platform,
        key: key.into(),
    }
}

fn no_mod(key: &str) -> KeyBinding {
    binding(false, false, false, false, key)
}

/// Default keymap for all keymap-dispatched actions.
///
/// Single source of truth consumed by `main.rs` (startup registration), the
/// `gpui::test` harness (no duplicated literal list), and the Settings panel
/// reset path. Covers every id listed in sh-app's `ACTIONS` table — the
/// anti-drift test lives in sh-app because only it can see both tables.
/// `CropCopy`/`CropSave`/`CropCancel` are deliberately absent: they have no
/// key binding registered today, so there is nothing to rebind.
pub fn defaults() -> Keymap {
    let mut m = Keymap::new();
    m.insert("next-image".into(), no_mod("right"));
    m.insert("prev-image".into(), no_mod("left"));
    m.insert("toggle-overlays".into(), no_mod("tab"));
    m.insert("toggle-fullscreen".into(), no_mod("f11"));
    m.insert("open-file".into(), binding(true, false, false, false, "o"));
    m.insert("open-folder".into(), binding(true, true, false, false, "o"));
    m.insert("back-to-grid".into(), no_mod("escape"));
    m.insert("open-selected".into(), no_mod("enter"));
    m.insert("toggle-crop".into(), no_mod("c"));
    m.insert("toggle-slideshow".into(), no_mod("space"));
    m.insert(
        "select-next".into(),
        binding(false, true, false, false, "right"),
    );
    m.insert(
        "select-prev".into(),
        binding(false, true, false, false, "left"),
    );
    m.insert(
        "toggle-selected".into(),
        binding(true, false, false, false, "space"),
    );
    m.insert("select-all".into(), binding(true, false, false, false, "a"));
    m.insert(
        "copy-selected".into(),
        binding(true, false, false, false, "c"),
    );
    m.insert("delete-selected".into(), no_mod("delete"));
    m.insert("move-selected".into(), no_mod("m"));
    m.insert(
        "open-settings".into(),
        binding(true, false, false, false, ","),
    );
    m
}

/// The incumbent action that already owns a combination in the same context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Action id of the existing owner (display label resolved in sh-app).
    pub existing_action: String,
}

/// Validate a candidate binding for `action` inside gpui keymap `context`.
///
/// Same combo owned by a DIFFERENT action in the SAME context → `Err(Conflict)`.
/// Same combo in a different context is NOT a conflict (gpui scopes bindings
/// by context; V1 registers everything under `"image_view"`).
/// The action's own current binding is never a conflict (re-pressing the same
/// combo onto the same action is a no-op save).
pub fn validate_binding(
    keymap: &Keymap,
    context: &str,
    action: &str,
    binding: &KeyBinding,
) -> Result<(), Conflict> {
    let _ = context; // Single-context V1: caller passes the real gpui context
                     // ("image_view") for API stability; conflict key today is
                     // (combo) since all bindings share that context. The param
                     // stays so per-view contexts need no signature change.
    for (id, owned) in keymap {
        if id != action && owned == binding {
            return Err(Conflict {
                existing_action: id.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_keymap_string_emits_canonical_order() {
        let b = KeyBinding {
            ctrl: true,
            shift: true,
            alt: false,
            platform: false,
            key: "o".into(),
        };
        assert_eq!(b.to_keymap_string(), "ctrl-shift-o");
        let bare = KeyBinding {
            ctrl: false,
            shift: false,
            alt: false,
            platform: false,
            key: "m".into(),
        };
        assert_eq!(bare.to_keymap_string(), "m");
        let comma = KeyBinding {
            ctrl: true,
            shift: false,
            alt: false,
            platform: false,
            key: ",".into(),
        };
        assert_eq!(comma.to_keymap_string(), "ctrl-,");
        let named = KeyBinding {
            ctrl: false,
            shift: true,
            alt: false,
            platform: false,
            key: "right".into(),
        };
        assert_eq!(named.to_keymap_string(), "shift-right");
    }

    #[test]
    fn defaults_covers_all_eighteen_actions() {
        let d = defaults();
        assert_eq!(d.len(), 18);
        for id in [
            "next-image",
            "prev-image",
            "toggle-overlays",
            "toggle-fullscreen",
            "open-file",
            "open-folder",
            "back-to-grid",
            "open-selected",
            "toggle-crop",
            "toggle-slideshow",
            "select-next",
            "select-prev",
            "toggle-selected",
            "select-all",
            "copy-selected",
            "delete-selected",
            "move-selected",
            "open-settings",
        ] {
            assert!(d.contains_key(id), "defaults missing {id}");
        }
        let mut seen = std::collections::HashSet::new();
        for b in d.values() {
            assert!(
                seen.insert(b.to_keymap_string()),
                "duplicate default combo {}",
                b.to_keymap_string()
            );
        }
    }

    #[test]
    fn validate_binding_rejects_same_context_conflict() {
        let d = defaults();
        let ctrl_o = d.get("open-file").unwrap().clone();
        let err = validate_binding(&d, "image_view", "open-folder", &ctrl_o).unwrap_err();
        assert_eq!(err.existing_action, "open-file");
    }

    #[test]
    fn validate_binding_allows_free_combo_and_self_rebind() {
        let d = defaults();
        let free = KeyBinding {
            ctrl: true,
            shift: false,
            alt: false,
            platform: false,
            key: "k".into(),
        };
        assert!(validate_binding(&d, "image_view", "open-folder", &free).is_ok());
        let own = d.get("open-folder").unwrap().clone();
        assert!(validate_binding(&d, "image_view", "open-folder", &own).is_ok());
    }

    #[test]
    fn keymap_serde_roundtrips_exact_settings_json_shape() {
        let d = defaults();
        let json = serde_json::to_string(&d).unwrap();
        assert!(
            json.contains(
                r#""open-file":{"ctrl":true,"shift":false,"alt":false,"platform":false,"key":"o"}"#
            ),
            "got: {json}"
        );
        let back: Keymap = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }
}

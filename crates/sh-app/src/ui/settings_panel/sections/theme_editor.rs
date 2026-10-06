//! Theme Editor section: the ten color slots plus the two structural fields,
//! as an ordered list with stable element ids.
//!
//! Pure and GPUI-free, like the other section modules — the render lives in
//! `app.rs` and reads everything it needs from here. That split is deliberate
//! for this section more than for the others: [`FIELDS`] is the single ordered
//! description of the column, and the row count the scroll geometry is derived
//! from, the keyboard order, and the tests all read that one list rather than
//! three restatements of "ten slots plus two".

use gpui::Hsla;
use sh_core::theme_draft::Slot;

/// One editable field in the Theme Editor column.
///
/// A `struct`-shaped enum rather than a `Vec<(label, value)>` so the two
/// structural fields cannot be confused with a color slot: `ThemeDraft::get`
/// and `set` are typed on [`Slot`], and a field that is not a slot has to be
/// matched separately rather than reaching them with a string key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// `Theme.name`. Required by `validate`, so it is editable here.
    Name,
    /// One of the ten color slots.
    Color(Slot),
    /// `Theme.typography.family`. Required by `validate`, so editable here.
    Family,
}

/// Every field the column renders, in render order.
///
/// The order is deliberate: the two required structural fields bracket the ten
/// optional colors, so the values that must be valid for a theme to load at
/// all are at the top and bottom of the list, and the middle is entirely
/// free-form. A user who has never opened this section can tell what is
/// mandatory from position alone.
pub const FIELDS: [Field; 12] = [
    Field::Name,
    Field::Color(Slot::Background),
    Field::Color(Slot::Surface),
    Field::Color(Slot::Text),
    Field::Color(Slot::Accent),
    Field::Color(Slot::MutedText),
    Field::Color(Slot::Border),
    Field::Color(Slot::Danger),
    Field::Color(Slot::OnAccent),
    Field::Color(Slot::Elevated),
    Field::Color(Slot::Ring),
    Field::Family,
];

/// Stable element id for the section column itself.
pub const CONTAINER_ID: &str = "theme-editor-controls";
/// Stable element id for the theme-name row.
pub const NAME_ROW_ID: &str = "theme-editor-row-name";
/// Stable element id for the typography-family row.
pub const FAMILY_ROW_ID: &str = "theme-editor-row-family";
/// Height of the hex text field inside a row.
///
/// The value is enforced by a WRAPPER `div`, never by `Input::h`: the kit's
/// inherent `Input::h` is applied only for multi-line inputs, so on a
/// single-line hex field it compiles and does nothing. The wrapper is also
/// strictly smaller than the 40px row on purpose — the row height is what the
/// scroll arithmetic is built from, so the field is pinned well below it and
/// the arithmetic stays correct whatever the kit's default input height is.
/// `input_fits_inside_every_row` asserts the relationship.
pub const FIELD_H_PX: f32 = 28.0;
/// Width of the hex text field inside a row.
///
/// Sized so the widest legal value — `#RRGGBBAA`, nine characters — fits
/// without the input scrolling its own text, and so label + field + swatch
/// still fit the 228px of row interior the 480px minimum window leaves.
pub const FIELD_W_PX: f32 = 120.0;
/// Side length of the color swatch shown beside each slot field.
pub const SWATCH_PX: f32 = 18.0;

/// Stable element id for a field's row.
pub fn row_id(field: Field) -> String {
    match field {
        Field::Name => NAME_ROW_ID.to_string(),
        Field::Family => FAMILY_ROW_ID.to_string(),
        Field::Color(slot) => format!("theme-editor-row-{}", slot.name()),
    }
}

/// Stable element id for a field's text input.
pub fn field_id(field: Field) -> String {
    format!("{}-input", row_id(field))
}

/// Stable element id for a color slot's swatch.
///
/// Non-color fields get one too, painted with the page background: a swatch is
/// how a row says "this is a color", and leaving it out on two of twelve rows
/// makes the difference between "structural field" and "broken slot" a shape
/// the user has to learn instead of something the row states.
pub fn swatch_id(field: Field) -> String {
    format!("{}-swatch", row_id(field))
}

/// The row's visible label.
///
/// JSON field names, not display names, on purpose: these are the keys the
/// same values live under in the theme file, so the row points at the place
/// the value would be edited by hand. `Slot::name` already makes that choice
/// for the colors; `name` and `family` extend it to the two structural fields.
pub fn label(field: Field) -> &'static str {
    match field {
        Field::Name => "name",
        Field::Family => "family",
        Field::Color(slot) => slot.name(),
    }
}

/// The row a draft error message is about, or `None` when it names no row.
///
/// Matched TOKEN BY TOKEN, never as a substring of the whole line. `text` is
/// a substring of `muted_text` and `accent` of `on_accent`, so a `contains`
/// test would point a `muted_text` error at two rows and an `on_accent` one at
/// the wrong pair.
///
/// Tokens rather than only the first one, because the message the caller
/// receives is `ShImagesError`'s `Display`: `"theme error: muted_text: …"` and
/// `"theme error: name must not be empty"`. Scanning every token finds the row
/// and still cannot confuse one slot for another, because `muted_text` is a
/// single token and `text` is not part of it.
///
/// Earliest matching token wins, so a message about two fields points at the
/// first — the one `resolve()` would have complained about.
///
/// `typography.family must not be empty` ends in a label the editor DOES
/// render, so a dotted path still resolves to its row; a message about a field
/// the editor does not expose (`interaction.hover_ratio`) resolves to `None`,
/// and the editor reports it in the header without marking anything.
pub fn field_named_by(message: &str) -> Option<Field> {
    message
        .split(|c: char| c == ':' || c.is_whitespace())
        .filter(|token| !token.is_empty())
        .find_map(|token| {
            FIELDS.iter().copied().find(|field| {
                let label = label(*field);
                token == label || token.ends_with(&format!(".{label}"))
            })
        })
}

/// Stable element id for the Save action in the column header.
pub const SAVE_ID: &str = "theme-editor-save";
/// Stable element id for the Cancel action in the column header.
pub const CANCEL_ID: &str = "theme-editor-cancel";
/// Height of a header action.
///
/// The header row is [`crate::ui::settings_panel::scroll::SETTINGS_HEADER_H_PX`]
/// tall and is already counted by `theme_editor_content_h`, so the actions
/// live INSIDE it rather than as a column child of their own: a new fixed-height
/// child would make the declared content height wrong, and the clamp would then
/// strand the last row past the scroll limit at the minimum window. That also
/// keeps Save on screen at every scroll position.
pub const ACTION_H_PX: f32 = 24.0;

/// The color a row's swatch should paint, or `None` when its text is not a
/// hex value the app can draw.
///
/// `None` is the normal state of a half-typed hex and of the six optional slots
/// a user has cleared to reset them, so it is a value the render has to handle,
/// not an error. It resolves through [`crate::app::parse_hex`] rather than
/// `sh_core::theme::parse_hex_color`, which is private and shared with the
/// file loader.
pub fn swatch_color(text: &str) -> Option<Hsla> {
    crate::app::parse_hex(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sh_core::theme_draft::SLOTS;

    #[test]
    fn every_slot_appears_exactly_once_and_in_render_order() {
        // The count is derived from the schema, so a new color added to
        // `ThemeColors` without a row fails here rather than being silently
        // uneditable — the same contract `Slot::field` enforces at compile
        // time for the draft itself.
        let colored: Vec<Slot> = FIELDS
            .iter()
            .filter_map(|f| match f {
                Field::Color(slot) => Some(*slot),
                _ => None,
            })
            .collect();
        assert_eq!(colored.len(), SLOTS.len());
        assert_eq!(colored.as_slice(), SLOTS.as_slice());
    }

    #[test]
    fn the_two_required_fields_bracket_the_optional_colors() {
        assert_eq!(FIELDS.first(), Some(&Field::Name));
        assert_eq!(FIELDS.last(), Some(&Field::Family));
    }

    #[test]
    fn field_count_is_the_row_count_the_scroll_geometry_uses() {
        assert_eq!(
            FIELDS.len(),
            crate::ui::settings_panel::scroll::THEME_EDITOR_ROW_COUNT
        );
    }

    #[test]
    fn row_and_field_ids_are_unique_and_prefixed() {
        let mut seen: Vec<String> = Vec::new();
        for field in FIELDS {
            let row = row_id(field);
            assert!(row.starts_with("theme-editor-"), "unprefixed id: {row}");
            assert!(!seen.contains(&row), "duplicate row id: {row}");
            assert_ne!(field_id(field), row);
            assert_ne!(swatch_id(field), row);
            seen.push(row);
        }
        assert_eq!(seen.len(), FIELDS.len());
    }

    /// Two rows differing only by a case difference would be indistinguishable
    /// in `debug_bounds` output and in the accessibility tree.
    #[test]
    fn labels_match_the_json_key_the_value_lives_under() {
        assert_eq!(label(Field::Name), "name");
        assert_eq!(label(Field::Family), "family");
        assert_eq!(label(Field::Color(Slot::MutedText)), "muted_text");
        assert_eq!(label(Field::Color(Slot::OnAccent)), "on_accent");
    }

    /// The swatch must tolerate every state the text can be in, including the
    /// ones that are not errors at all: an empty optional slot is how the user
    /// says "reset this", so it must not read as a malformed value.
    #[test]
    fn swatch_accepts_every_hex_form_and_rejects_only_junk() {
        assert!(swatch_color("#0d0d0f").is_some(), "6-digit");
        assert!(swatch_color("#fff").is_some(), "3-digit");
        assert!(swatch_color("#00ffff8c").is_some(), "8-digit keeps alpha");
        assert!(swatch_color("00ffff").is_some(), "leading # optional");
        assert!(swatch_color("").is_none(), "an emptied optional slot");
        assert!(swatch_color("#12").is_none(), "half-typed");
        assert!(swatch_color("zzz").is_none(), "not hex at all");
    }

    /// The field is sized by hand precisely so the kit's own input height
    /// cannot push past the row the scroll arithmetic counts.
    #[test]
    fn input_fits_inside_every_row() {
        let row_h = crate::ui::settings_panel::scroll::SETTINGS_ROW_H_PX;
        assert!(
            FIELD_H_PX < row_h,
            "the field is {FIELD_H_PX} tall inside a {row_h} row: it overflows"
        );
        assert!(
            SWATCH_PX < row_h,
            "the swatch is {SWATCH_PX} tall inside a {row_h} row: it overflows"
        );
        // The field's width is what could push the row's other children out at
        // the 480px minimum, where the row interior is only 228px.
        assert!(
            FIELD_W_PX < row_h * 4.0,
            "a {FIELD_W_PX} field is too wide beside a label and a swatch"
        );
    }

    /// The header actions have to FIT the header, or Save grows the column and
    /// the declared content height becomes wrong. Both must sit strictly inside
    /// `SETTINGS_HEADER_H_PX` for the same reason the fields sit inside a row.
    #[test]
    fn header_actions_fit_inside_the_header_row() {
        let header_h = crate::ui::settings_panel::scroll::SETTINGS_HEADER_H_PX;
        assert!(
            ACTION_H_PX < header_h,
            "the action is {ACTION_H_PX} tall inside a {header_h} header: it overflows"
        );
    }

    /// A slot error must mark its OWN row. `text` is a substring of
    /// `muted_text` and `accent` of `on_accent`, so a whole-line substring match
    /// would light up two rows and point at the wrong one.
    #[test]
    fn an_error_names_exactly_the_row_it_is_about() {
        assert_eq!(
            field_named_by("muted_text: must be a hex color"),
            Some(Field::Color(Slot::MutedText))
        );
        assert_eq!(
            field_named_by("on_accent: must be a hex color"),
            Some(Field::Color(Slot::OnAccent))
        );
        assert_eq!(
            field_named_by("accent: must be a hex color"),
            Some(Field::Color(Slot::Accent))
        );
        assert_eq!(field_named_by("name must not be empty"), Some(Field::Name));
        assert_eq!(
            field_named_by("typography.family must not be empty"),
            Some(Field::Family)
        );
    }

    /// The message the caller really receives is `ShImagesError`'s `Display`,
    /// which prefixes every theme error. A matcher that only looked at the
    /// first token would find `theme` and report no row at all, so every one of
    /// the errors would land in the header with nothing marked.
    #[test]
    fn a_prefixed_error_still_names_its_row() {
        assert_eq!(
            field_named_by("theme error: muted_text: must be a hex color"),
            Some(Field::Color(Slot::MutedText))
        );
        assert_eq!(
            field_named_by("theme error: name must not be empty"),
            Some(Field::Name)
        );
        assert_eq!(
            field_named_by("theme error: typography.family must not be empty"),
            Some(Field::Family)
        );
    }

    /// The inverse case matters too: a field the editor does not expose has no
    /// row, so it must resolve to `None` rather than to the nearest label. A
    /// `contains` match on `interaction.hover_ratio` would find nothing, but a
    /// match on the whole line would find nothing for the wrong reason — this
    /// pins the honest one.
    #[test]
    fn a_message_with_no_row_resolves_to_none() {
        assert_eq!(
            field_named_by("interaction.hover_ratio must be between"),
            None
        );
        assert_eq!(field_named_by(""), None);
    }

    /// The ids the tests and the render share must be stable and distinct: a
    /// Save and a Cancel that resolved to one string would make
    /// `debug_bounds` silently assert the wrong element.
    #[test]
    fn action_ids_are_distinct() {
        assert_ne!(SAVE_ID, CANCEL_ID);
        assert_ne!(SAVE_ID, CONTAINER_ID);
        assert_ne!(CANCEL_ID, CONTAINER_ID);
    }
}

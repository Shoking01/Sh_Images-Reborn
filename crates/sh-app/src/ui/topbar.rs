//! Persistent top bar: folder/image context + open-folder affordance.

use gpui::prelude::*;
use gpui::*;

/// Top bar height in px. Single source of truth: layout offset for the
/// overlay chip AND the viewer fit-area reduction both derive from this.
pub const TOPBAR_H_PX: f32 = 40.0;

/// Data needed to render the top bar.
#[derive(Debug, Clone)]
pub struct TopbarData {
    /// Left slot text (grid: folder name; viewer: unused, back button sits).
    pub left: String,
    /// Center slot text (viewer: "name — 3/12"; grid: empty).
    pub center: String,
    /// Text color from the active theme.
    pub theme_text: Hsla,
    /// Surface color from the active theme (bar background).
    pub theme_surface: Hsla,
}

/// Render the persistent top bar (~40px). `back` is the optional pre-built
/// "Back" button (viewer arm only); `open` is the pre-built "Open folder"
/// button; `sort` is the pre-built sort-criterion chip (V3 sort dropdown
/// trigger); `settings` is the pre-built gear button; `crop` is the optional
/// pre-built scissors button (viewer arm only). Call-site builds buttons with
/// `cx.listener`, same as arrows.
///
/// Returns the concrete `Stateful<Div>` (not `impl IntoElement`) so the
/// caller can apply `.hidden()` for the Viewer-idle dissolve gate.
pub fn topbar(
    data: &TopbarData,
    back: Option<AnyElement>,
    open: AnyElement,
    sort: AnyElement,
    settings: AnyElement,
    crop: Option<AnyElement>,
) -> Stateful<Div> {
    let mut left = div().flex().items_center();
    if let Some(b) = back {
        left = left.child(b);
    } else {
        left = left.child(div().child(data.left.clone()));
    }
    let mut row = div()
        .id("topbar")
        .w_full()
        .h(px(TOPBAR_H_PX))
        .flex()
        .items_center()
        .justify_between()
        .bg(data.theme_surface)
        .text_color(data.theme_text)
        .px(px(14.0))
        .child(left);
    if !data.center.is_empty() {
        row = row.child(div().child(data.center.clone()));
    }
    row.child(
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(open)
            .child(sort)
            .child(settings)
            .children(crop),
    )
}

#[cfg(test)]
mod tests {
    // NOTE: explicit imports instead of `use super::*` — gpui's glob re-exports
    // the `test` proc macro, which blows the recursion limit under `use super::*`.
    use super::{TopbarData, TOPBAR_H_PX};
    use gpui::rgb;

    #[test]
    fn topbar_constant_and_data_smoke() {
        // Locks the layout contract: overlay offset + fit-area math derive here.
        assert_eq!(TOPBAR_H_PX, 40.0);
        let d = TopbarData {
            left: "C:\\Fotos".into(),
            center: String::new(),
            theme_text: rgb(0xe8e8ee).into(),
            theme_surface: rgb(0x121218).into(),
        };
        assert_eq!(d.left, "C:\\Fotos");
        assert!(d.center.is_empty());
    }
}

//! Settings section navigation column layout.

use gpui::prelude::*;
use gpui::*;

/// Sidebar column (~200px). Rows are caller-built (active row wears the
/// accent bar — applied at the call site, same as the topbar chips);
/// this function only lays them out.
pub fn sidebar(rows: Vec<AnyElement>) -> impl IntoElement {
    let mut col = div()
        .id("settings-sidebar")
        .w(px(200.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .p(px(8.0));
    for row in rows {
        col = col.child(row);
    }
    col
}

//! Root application component: session, theme, key dispatch, root render.

use crate::state::session::Session;
use crate::state::theme_store::ThemeStore;
use crate::viewer::{render_viewer, ViewerParams};
use gpui::prelude::*;
use gpui::*;
use std::path::PathBuf;

/// The root application entity.
pub struct App {
    /// Mutable session state.
    pub session: Session,
    /// Active theme store.
    pub theme_store: ThemeStore,
    /// Current viewport size in pixels.
    pub viewport: Size<Pixels>,
}

impl App {
    /// Create a new app with the given session and theme.
    pub fn new(session: Session, theme_store: ThemeStore) -> Self {
        Self {
            session,
            theme_store,
            viewport: size(px(0.), px(0.)),
        }
    }

    /// Open a set of images from a resolved list (Task 7 wires navigation).
    pub fn set_images(&mut self, paths: Vec<PathBuf>, current: usize) {
        let items = paths
            .into_iter()
            .map(|path| crate::state::session::ImageItem {
                name: path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image")
                    .to_string(),
                path,
                decoded: None,
            })
            .collect();
        self.session.images = items;
        self.session.current = current;
    }
}

impl Render for App {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg: Hsla =
            parse_hex(&self.theme_store.theme.colors.background).unwrap_or(rgb(0x0d0d0f).into());
        let params = ViewerParams {
            path: self.session.current_item().map(|i| i.path.clone()),
            position: self.session.position_label(),
            error: self.session.error.clone(),
            zoom_text: format!("{:.0}%", self.session.zoom.scale * 100.0),
            show_overlay_top: self.session.show_overlay_top,
            show_overlay_bottom: self.session.show_overlay_bottom,
        };
        div()
            .id("app-root")
            .size_full()
            .bg(bg)
            .on_drop(cx.listener(|_this, _paths: &ExternalPaths, _window, _cx| {
                // Task 10 wires full open flow
            }))
            .child(render_viewer(&params))
    }
}

/// Parse a hex color string like `"#0d0d0f"` into an [`Hsla`].
/// Returns `None` on error. Supports 3, 6, and 8-digit hex with optional `#`.
pub fn parse_hex(hex: &str) -> Option<Hsla> {
    let hex = hex.trim_start_matches('#');
    let n: u32 = match hex.len() {
        3 => {
            let mut it = hex.chars();
            let r = it.next()?;
            let g = it.next()?;
            let b = it.next()?;
            let v = |c: char| -> Option<u32> { u32::from_str_radix(&format!("{c}{c}"), 16).ok() };
            (v(r)? << 16) | (v(g)? << 8) | v(b)?
        }
        6 => u32::from_str_radix(hex, 16).ok()?,
        8 => u32::from_str_radix(hex, 16).ok()?,
        _ => return None,
    };
    Some(rgb(n).into())
}

#[cfg(test)]
mod tests {
    use super::parse_hex;

    #[test]
    fn parses_six_digit_hex() {
        let c = parse_hex("#0d0d0f");
        assert!(c.is_some());
    }

    #[test]
    fn parses_three_digit_hex() {
        let c = parse_hex("#abc");
        assert!(c.is_some());
    }

    #[test]
    fn parses_eight_digit_hex() {
        let c = parse_hex("#aabbccdd");
        assert!(c.is_some());
    }

    #[test]
    fn rejects_invalid_hex() {
        assert!(parse_hex("nope").is_none());
        assert!(parse_hex("#12").is_none());
        assert!(parse_hex("#xyz").is_none());
    }

    #[test]
    fn accepts_without_hash_prefix() {
        let c = parse_hex("0d0d0f");
        assert!(c.is_some());
    }
}

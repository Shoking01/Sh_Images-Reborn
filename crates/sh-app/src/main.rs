//! Sh_Images desktop application entry point.

use gpui::AppContext as _;
use sh_app::actions::{NextImage, PrevImage, ToggleOverlays};
use sh_app::app::App;
use sh_app::state::session::{build_image_items, Session};
use sh_app::state::theme_store::ThemeStore;
use sh_core::theme;
use tracing::info;

fn main() {
    // Task 0: tracing bootstrap — kept as-is.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    info!("sh_images starting");

    // CLI arg: optional image path to open.
    let arg_path = std::env::args().nth(1).map(std::path::PathBuf::from);

    // Load settings + default theme.
    let settings_path = config_dir().join("settings.json");
    let settings = sh_core::settings::load(&settings_path);

    // Built-in theme: proven by `parses_all_builtin_themes` test in sh-core.
    let theme_json = include_str!("../../../themes/deep-neutral.json");
    let default_theme = theme::parse(theme_json).expect("built-in theme must parse");

    let theme_path = settings_path
        .parent()
        .expect("config_dir always has parent")
        .join("themes")
        .join(&settings.theme);
    let theme_store = ThemeStore::new(default_theme, theme_path);

    let mut session = Session::default();
    if let Some(path) = arg_path {
        if let Ok(list) = sh_core::navigation::resolve(&path) {
            session.images = build_image_items(list.paths);
            session.current = list.current;
            info!(
                "opened {} images, current={}",
                session.images.len(),
                session.current
            );
        }
    }

    gpui::Application::new().run(move |cx: &mut gpui::App| {
        let bounds = gpui::Bounds::centered(None, gpui::size(gpui::px(1000.), gpui::px(720.)), cx);
        cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Sh_Images".into()),
                    ..Default::default()
                }),
                window_min_size: Some(gpui::size(gpui::px(480.), gpui::px(320.))),
                ..Default::default()
            },
            move |_, cx| cx.new(|_| App::new(session, theme_store)),
        )
        .expect("failed to open window");

        // Task 7: register global key bindings for navigation and overlays.
        // Bindings are scoped to the "image_view" key context set on the root div.
        // NOTE: `cx` here is `&mut gpui::App`, not `Context<App>`.
        cx.bind_keys([
            gpui::KeyBinding::new("right", NextImage, Some("image_view")),
            gpui::KeyBinding::new("left", PrevImage, Some("image_view")),
            gpui::KeyBinding::new("tab", ToggleOverlays, Some("image_view")),
        ]);

        cx.activate(true);
        info!("window opened");
    });
}

/// Windows config dir: `%APPDATA%\sh_images`
pub fn config_dir() -> std::path::PathBuf {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("sh_images")
}

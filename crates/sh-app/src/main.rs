//! Sh_Images desktop application entry point.

use gpui::AppContext as _;
use sh_app::actions::{
    BackToGrid, NextImage, OpenFile, OpenFolder, OpenSelected, PrevImage, ToggleFullscreen,
    ToggleOverlays,
};
use sh_app::app::App;
use sh_app::state::session::{build_image_items, Session};
use sh_app::state::theme_store::{theme_startup, ThemeStartup, ThemeStore};
use sh_app::theme_builtins::builtin_theme_json;
use sh_core::theme;
use tracing::{info, warn};

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

    // Load settings + resolve the active theme file.
    let settings_path = config_dir().join("settings.json");
    let settings = sh_core::settings::load(&settings_path);

    let theme_path = settings_path
        .parent()
        .expect("config_dir always has parent")
        .join("themes")
        .join(&settings.theme);

    // Bootstrap or load: a fresh install gets the built-in theme WRITTEN to
    // its config file so the hot-reload flow has something to edit; an
    // existing file is loaded (fall back to built-in if it no longer parses).
    // Invariant: every `expect("built-in theme must parse")` below holds —
    // proven by sh-core's `theme::tests::parses_all_builtin_themes` plus
    // sh-app's `startup_missing_file_bootstraps_builtin`.
    let (default_theme, theme_text) = match theme_startup(&settings.theme, theme_path.exists()) {
        ThemeStartup::UseExisting => match std::fs::read_to_string(&theme_path) {
            Ok(text) => match theme::parse(&text) {
                Ok(t) => (t, text),
                Err(e) => {
                    warn!(
                        "theme {0} failed to parse: {e}; using built-in",
                        settings.theme
                    );
                    let text = builtin_theme_json(&settings.theme).to_string();
                    (
                        theme::parse(&text).expect("built-in theme must parse"),
                        text,
                    )
                }
            },
            Err(e) => {
                warn!("theme {0} unreadable: {e}; using built-in", settings.theme);
                let text = builtin_theme_json(&settings.theme).to_string();
                (
                    theme::parse(&text).expect("built-in theme must parse"),
                    text,
                )
            }
        },
        ThemeStartup::Bootstrap { builtin_json } => {
            let text = builtin_json.to_string();
            if let Err(e) = std::fs::create_dir_all(
                theme_path
                    .parent()
                    .expect("configured theme path always has a parent"),
            )
            .and_then(|()| std::fs::write(&theme_path, &text))
            {
                warn!(
                    "could not bootstrap theme file {}: {e}",
                    theme_path.display()
                );
            }
            (
                theme::parse(&text).expect("built-in theme must parse"),
                text,
            )
        }
    };

    let theme_store = ThemeStore::new(default_theme, settings.theme.clone(), theme_path);

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
        let window = cx
            .open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Sh_Images".into()),
                        ..Default::default()
                    }),
                    window_min_size: Some(gpui::size(gpui::px(480.), gpui::px(320.))),
                    ..Default::default()
                },
                move |_, cx| {
                    cx.new(|cx| {
                        App::new(
                            session,
                            theme_store,
                            settings_path,
                            settings,
                            theme_text,
                            cx,
                        )
                    })
                },
            )
            .expect("failed to open window");

        // Initial probe: the CLI image's dimensions must be read and its fit
        // computed, or the first render shows nothing (scale 0.0 → 0×0 image).
        // navigate(0) targets the current slot; the seq-guard then commits the
        // fit once the header probe lands.
        window
            .update(cx, |app, window, cx| {
                // Keyboard focus must land inside the "image_view" subtree
                // BEFORE the first keystroke: key bindings only match against
                // the focused element's dispatch path. Focusing the tracked
                // root div here makes ←/→/Tab/F11/Ctrl+O work immediately on
                // cold start, with no prior mouse interaction required.
                window.focus(&app.focus_handle);
                app.navigate(0, cx);
                // Task 9: idle watcher — wakes to auto-hide the overlays
                // after OVERLAY_IDLE of no mouse activity.
                App::spawn_idle_watcher(cx);
                // Task 10: theme hot-reload watcher — polls the active
                // theme file and re-applies it on valid edits.
                App::spawn_theme_watcher(cx);
            })
            .expect("window must be open to trigger initial probe");

        // Task 7: register global key bindings for navigation and overlays.
        // Bindings are scoped to the "image_view" key context set on the root div.
        // NOTE: `cx` here is `&mut gpui::App`, not `Context<App>`.
        cx.bind_keys([
            gpui::KeyBinding::new("right", NextImage, Some("image_view")),
            gpui::KeyBinding::new("left", PrevImage, Some("image_view")),
            gpui::KeyBinding::new("tab", ToggleOverlays, Some("image_view")),
            gpui::KeyBinding::new("f11", ToggleFullscreen, Some("image_view")),
            gpui::KeyBinding::new("ctrl-o", OpenFile, Some("image_view")),
            gpui::KeyBinding::new("ctrl-shift-o", OpenFolder, Some("image_view")),
            gpui::KeyBinding::new("escape", BackToGrid, Some("image_view")),
            gpui::KeyBinding::new("enter", OpenSelected, Some("image_view")),
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

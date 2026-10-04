//! Sh_Images desktop application entry point.

// Windows GUI subsystem: without this the linker builds a console subsystem
// binary and every launch flashes a black terminal window behind the viewer.
// Attribute is inert on other platforms. CLI argument handling below is
// unaffected — an attached console (or a parent terminal) still forwards
// argv, and diagnostics go through `tracing` as before.
#![windows_subsystem = "windows"]

use gpui::AppContext as _;
use sh_app::app::App;
use sh_app::state::session::{build_image_items, Session};
use sh_app::state::theme_store::{theme_startup, ThemeStartup, ThemeStore};
use sh_app::state::view::{startup_view, CliKind, StartupTarget, View};
use sh_app::theme_builtins::builtin_theme_json;
use sh_core::theme;
use tracing::{info, warn};

/// Stack size for the thread that runs the GPUI event loop.
///
/// 16 MB is deliberately generous rather than tuned. The requirement is that one
/// unoptimized `App::render` frame fits with room to spare, and the cost of
/// over-provisioning a thread stack is address space that is reserved, never
/// committed. Tuning it down to the measured minimum would trade a crash-free
/// `cargo run` for a fragile one that breaks again the moment a view grows.
const MAIN_THREAD_STACK_BYTES: usize = 16 * 1024 * 1024;

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
    let mut settings = sh_core::settings::load(&settings_path);

    // A pre-v10 file loads current defaults in memory; write it back so
    // settings.json carries the schema version used by every settings writer.
    if settings.version < sh_core::settings::CURRENT_SETTINGS_VERSION {
        let mut upgraded = settings.clone();
        upgraded.version = sh_core::settings::CURRENT_SETTINGS_VERSION;
        if let Err(e) = sh_core::settings::save(&settings_path, &upgraded) {
            warn!("could not persist upgraded settings: {e}");
        }
        settings = upgraded;
    }

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

    // Values the gpui-component bridge needs at startup, taken before
    // `theme_store` is moved into the App. `default_theme` is Clone, and the
    // mode is derived from the theme FILE NAME for the same reason
    // `App::kit_theme_mode` does it: the JSON carries no mode field.
    let startup_theme_for_bridge = theme_store.theme.clone();
    let startup_mode = if settings.theme.to_ascii_lowercase().contains("light") {
        gpui_component::theme::ThemeMode::Light
    } else {
        gpui_component::theme::ThemeMode::Dark
    };

    let mut session = Session::default();
    // V2 Task 8: classify the CLI arg BEFORE App exists (startup_view is
    // pure): file → Viewer with the resolved list (v1 behavior), dir → Grid
    // with the scanned folder (possibly empty → grid empty state), none →
    // Welcome. recent_dirs_available seeds from settings when they exist.
    let cli_kind = arg_path.as_ref().map(|p| {
        if p.is_dir() {
            CliKind::Dir
        } else {
            CliKind::File
        }
    });
    let mut initial_view = match startup_view(cli_kind) {
        StartupTarget::Grid => View::Grid,
        StartupTarget::Viewer => View::Viewer,
        StartupTarget::Welcome => View::Welcome,
    };
    if let Some(path) = &arg_path {
        if path.is_dir() {
            session.sort_by = settings.sort_by;
            session.sort_dir = settings.sort_dir;
            // A folder argument is a folder LISTING, so it obeys the same
            // rule as opening one from the UI — the toggle means something
            // here, and a user who hides dotfiles does not want `.thumbs/`
            // filling their grid at startup.
            let entries = sh_core::navigation::scan_entries(
                path,
                sh_core::navigation::ScanOptions {
                    show_hidden: settings.show_hidden_files,
                },
            );
            session.images = build_image_items(entries);
            // Startup sort: entries scan as Name/Asc; apply the persisted
            // criterion. current anchors on the first image of the new
            // order (resort re-anchors by path first).
            session.resort();
            session.current = 0;
            info!("opened folder with {} images", session.images.len());
        } else if let Ok(list) = sh_core::navigation::resolve(path) {
            // Entry-bridge for the file CLI arg: resolve validated membership
            // (path exists in its parent's list); re-scan the parent with
            // full metadata and anchor by path, then apply the persisted
            // sort on top — metadata sorts need real entries, not bare paths.
            session.sort_by = settings.sort_by;
            session.sort_dir = settings.sort_dir;
            let anchor = list.paths.get(list.current).cloned();
            let parent = path.parent().map(std::path::Path::to_path_buf);
            // `show_hidden: true` on purpose, and it MUST match what `resolve`
            // just did: the CLI argument is an explicit request for THIS
            // file, so a dotfile is opened rather than rejected as
            // `NotAFile`. Filtering here instead would anchor on nothing
            // and quietly drop the argument the user typed.
            let entries = parent
                .map(|p| {
                    sh_core::navigation::scan_entries(
                        &p,
                        sh_core::navigation::ScanOptions { show_hidden: true },
                    )
                })
                .unwrap_or_default();
            session.images = build_image_items(entries);
            session.current = anchor
                .and_then(|a| session.images.iter().position(|i| i.path == a))
                .unwrap_or(0);
            session.resort();
            info!(
                "opened {} images, current={}",
                session.images.len(),
                session.current
            );
        } else {
            // Unresolvable CLI path: fall back to Welcome, never an empty Viewer.
            initial_view = View::Welcome;
        }
    }
    let recent_dirs_available = settings
        .recent_dirs
        .iter()
        .filter(|d| d.is_dir())
        .cloned()
        .collect::<Vec<_>>();
    // The folder a hidden-files toggle must re-scan at startup. Taken from the
    // CLI argument rather than from the loaded list, because a folder whose
    // only images are hidden lands an EMPTY session — and an empty session
    // cannot name the folder it came from. Gated on the app actually having
    // entered Grid: an argument that fell back to Welcome loaded nothing, so
    // scanning a folder for it would put a list in front of a Welcome view.
    let startup_dir = match (initial_view, &arg_path) {
        (View::Grid, Some(path)) => Some(path.clone()),
        _ => None,
    };

    // Snapshot the startup keymap BEFORE the window closure below moves
    // `settings`: `bind_keys` runs after `open_window` in the same scope.
    let startup_keymap = settings.keymap.clone();

    // SPIKE: gpui 0.3.x removed the no-arg `Application::new()` — the platform is
    // now an explicit constructor argument, because the same framework also
    // serves wasm/wgpu/web and a no-arg constructor would have to guess.
    //    `current_platform(false)` is precisely what `new()` used to call
    // internally, so this is the same platform, resolved explicitly.
    // The application runs on an explicitly-sized thread instead of on `main`.
    //
    // The main thread's stack on Windows is 1 MB, and `App::render` is a single
    // method of roughly 12,900 lines that builds the whole element tree inline.
    // An unoptimized build materializes every temporary into the stack frame
    // instead of eliding it, so the first frame overflows and the process dies
    // with `STATUS_STACK_OVERFLOW` (0xc00000fd) before the window ever opens —
    // reproducibly, and only in debug. Release elides enough of it to fit,
    // which is why this reads as "works on my machine": the defect is in the
    // build profile, not in the code path.
    //
    // Raising the stack fixes the cause rather than documenting the workaround,
    // so `cargo run` behaves like `cargo run --release`. The previous behaviour
    // was recorded in the spike notes as "debug builds may overflow the stack",
    // which described the symptom and left it unresolved.
    std::thread::Builder::new()
        .name("sh-images-gpui".to_string())
        .stack_size(MAIN_THREAD_STACK_BYTES)
        .spawn(move || {
            gpui::Application::with_platform(gpui_pre_platform::current_platform(false))
                // ONE asset source. `Application::with_assets` REPLACES the source it
                // was given rather than composing with a previous one, so registering
                // a second source silently disables the first — which blanked every
                // icon the app owns (the crop button drew its layout box and nothing
                // else). `AppAssets` owns the app's SVGs and falls back to
                // gpui-component's bundle, so both sets resolve from this one call.
                // The kit's bytes are compiled in, not read from disk, so nothing
                // renders unless its `AssetSource` answers for the path.
                .with_assets(sh_app::assets::AppAssets)
                .run(move |cx: &mut gpui::App| {
                    // gpui-component must be initialized before any of its components are
                    // built, and its global `Theme` must already carry the app's colors —
                    // `Button::new` reads `cx.theme()` at construction. The two theme
                    // values are moved into the closure below, so this runs on a copy
                    // taken before they are consumed.
                    sh_app::kit_theme::ensure_kit_initialized(cx);
                    {
                        let startup_theme = startup_theme_for_bridge.clone();
                        sh_app::kit_theme::sync_kit_theme(cx, &startup_theme, startup_mode);
                    }
                    let bounds = gpui::Bounds::centered(
                        None,
                        gpui::size(gpui::px(1000.), gpui::px(720.)),
                        cx,
                    );
                    let window = cx
                        .open_window(
                            gpui::WindowOptions {
                                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                                titlebar: Some(gpui::TitlebarOptions {
                                    title: Some("Sh_Images".into()),
                                    ..Default::default()
                                }),
                                window_min_size: Some(gpui::size(gpui::px(480.), gpui::px(320.))),
                                // Left `Opaque` on purpose.
                                //
                                // `Blurred` (Win32 ACCENT_ENABLE_ACRYLICBLURBEHIND) and
                                // `MicaBackdrop` (DWMSBT_MAINWINDOW) are both wired up in
                                // gpui-pre-windows, so this field is not a no-op at the
                                // platform layer — but measured against a saturated
                                // window placed directly behind the app, neither one
                                // composites: the top bar sampled `15151A` with a
                                // magenta window behind it and `101014` in the opaque
                                // grid below. The backdrop simply does not reach the
                                // wgpu swapchain, so enabling it buys a slower
                                // composite path and no visible transparency.
                                //
                                // The bar's translucency is therefore kept deliberately
                                // subtle: it reads against the app's own background,
                                // which is what actually composites.
                                window_background: gpui::WindowBackgroundAppearance::Opaque,
                                ..Default::default()
                            },
                            move |_, cx| {
                                cx.new(|cx| {
                                    let mut app = App::new(
                                        session,
                                        theme_store,
                                        settings_path,
                                        settings,
                                        theme_text,
                                        // A `Context`, not an `App`: `App::new` arms the
                                        // thumbnail batch for the session it is handed,
                                        // and this session comes from a synchronous CLI
                                        // scan, so no async commit handler ever arms it.
                                        cx,
                                    );
                                    app.view = initial_view;
                                    app.recent_dirs_available = recent_dirs_available;
                                    // Overrides the session-derived seed: a folder
                                    // argument may have produced an empty list, and
                                    // the toggle still has to know the folder.
                                    app.current_dir = startup_dir.clone();
                                    app
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
                            // SPIKE: gpui 0.3.x's `Window::focus` takes `&mut App` as a second
                            // argument; 0.2.2 took only the handle.
                            window.focus(&app.focus_handle, cx);
                            // Probe the current image only when there is one: Welcome /
                            // empty Grid have no current slot (navigate would no-op, but
                            // skipping avoids a pointless error-slot write).
                            if !app.session.images.is_empty() {
                                app.navigate(0, cx);
                            }
                            // Task 9: idle watcher — wakes to auto-hide the overlays
                            // after OVERLAY_IDLE of no mouse activity.
                            App::spawn_idle_watcher(cx);
                            // Dynamic slideshow timer: armed when playback starts and
                            // re-armed when its persisted interval changes.
                            app.rearm_slideshow_timer(cx);
                            // Task 10: theme hot-reload watcher — polls the active
                            // theme file and re-applies it on valid edits.
                            App::spawn_theme_watcher(cx);
                        })
                        .expect("window must be open to trigger initial probe");

                    // Task 7 (settings slice): bindings come from the ActionDescriptor table
                    // (sh-app actions.rs) merged with the persisted keymap — one source of
                    // truth shared with the test harness and the live-rebind path. The
                    // startup save above persists the current schema version on first run so
                    // settings.json always carries the bindings the panel edits.
                    cx.bind_keys(sh_app::actions::resolve_bindings(&startup_keymap));

                    cx.activate(true);
                    info!("window opened");
                });
        })
        .expect("the GPUI thread must spawn")
        .join()
        .ok();
}

/// Windows config dir: `%APPDATA%\sh_images`
pub fn config_dir() -> std::path::PathBuf {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("sh_images")
}

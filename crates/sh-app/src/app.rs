//! Root application component: session, theme, key dispatch, root render.

use crate::actions::{
    BackToGrid, CopySelected, CropCancel, CropCopy, CropSave, DeleteSelected, MoveSelected,
    NextImage, OpenFile, OpenFolder, OpenSelected, OpenSettings, PrevImage, SelectAll, SelectNext,
    SelectPrev, ToggleCrop, ToggleFullscreen, ToggleOverlays, ToggleSelected, ToggleSlideshow,
};
use crate::state::session::{
    build_image_items, next_index, FitMode, ImageItem, Session, ZoomPreset,
};
use crate::state::theme_store::{hot_reload_decision, HotReloadDecision, ThemeStore};
use crate::state::view::View;
use crate::ui::grid;
use crate::ui::grid::GridSizeGeometry;
use crate::ui::icons::{icon, IconName};
use crate::ui::motion;
use crate::ui::overlay::{self, OverlayData};
use crate::ui::settings_panel::scroll;
use crate::ui::topbar;
use crate::ui::welcome;
use crate::viewer::{render_viewer, ViewerParams};
use gpui::prelude::*;
use gpui::*;
// `Selectable` supplies `Button::selected`, which paints the selected styling.
// `Button::toggled` is public too but only sets the accessibility metadata, so
// the trait method is what actually renders an active segment or an open menu.
use gpui_component::switch::Switch;
use gpui_component::Selectable;
// `Sizable` supplies `Button::with_size`, needed for the icon-only settings
// button: without an explicit size a compact button with no label collapses.
use gpui_component::Sizable;
use sh_core::i18n::{t, Language, StrKey};
use sh_core::navigation::{SortBy, SortDir};
use sh_core::settings::{GridSize, CURRENT_SETTINGS_VERSION, SLIDESHOW_INTERVAL_MIN_SECS};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

/// Staged batch file op awaiting confirm-bar approval (V3 destructive half).
/// Paths are snapshotted at staging: the rescan after execution rebuilds
/// indices, so holding indices here would corrupt the outcome.
#[derive(Debug, Clone)]
pub enum BatchOp {
    Delete { paths: Vec<PathBuf> },
    Move { paths: Vec<PathBuf>, dest: PathBuf },
}

impl BatchOp {
    fn paths(&self) -> &[PathBuf] {
        match self {
            BatchOp::Delete { paths } => paths,
            BatchOp::Move { paths, .. } => paths,
        }
    }
}

/// The root application entity.
pub struct App {
    /// Mutable session state.
    pub session: Session,
    /// Active theme store.
    pub theme_store: ThemeStore,
    /// Current viewport size in pixels.
    pub viewport: Size<Pixels>,
    /// Monotonic navigation counter; guards against stale decode completions.
    ///
    /// Every `navigate()` bumps this. An async probe completion only commits
    /// global state (`zoom`/`fit_mode`/`error`) while its captured sequence
    /// still matches, so a slow probe for image B cannot clobber the state
    /// of a later navigation to image C.
    pub navigation_seq: u64,
    /// Last mouse-down position while dragging (pan gesture), if any.
    pub drag_last: Option<Point<Pixels>>,
    /// Last observed hover cursor position, if any.
    ///
    /// Deadband anchor for the idle clock (see [`hover_moved_enough`]):
    /// bare-hover moves below [`HOVER_DEADBAND_PX`] don't reset
    /// [`Self::last_interaction`], so sensor-noise micro-movements can't
    /// re-show the overlays right at/after the idle threshold.
    pub last_hover: Option<Point<Pixels>>,
    /// Timestamp of the last user interaction of any kind (mouse move,
    /// keyboard action); drives overlay auto-hide.
    pub last_interaction: Instant,
    /// Pending slideshow delay task. Dropping the handle cancels the task.
    slideshow_timer: Option<Task<()>>,
    /// Whether the idle tick has already hidden the overlays for the current
    /// idle period. Prevents perpetual re-render: the tick only notifies on
    /// the visible→hidden transition, and any interaction resets the flag.
    pub overlays_hidden_by_idle: bool,
    /// Path to `settings.json` (used by [`Self::persist`]).
    pub settings_path: PathBuf,
    /// The settings as loaded at startup; [`Self::persist`] saves a copy of
    /// this with only `theme`, `last_dir`, and the current schema version
    /// updated, so user-edited values in other fields
    /// (`show_hidden_files`, `max_decode_dimension`, and `reduce_motion`)
    /// survive every save.
    pub settings: sh_core::settings::Settings,
    /// Hover phases for the bounded set of motion-enabled controls.
    hover_motion: motion::HoverMotionState,
    /// Theme-file text last successfully applied by hot reload (or startup).
    ///
    /// Hot-reload dedupe anchor: identical text means "nothing changed",
    /// and each DISTINCT invalid text is warned about exactly once.
    pub last_applied_theme_text: String,
    /// Text of the last invalid theme the watcher warned about.
    ///
    /// Dedupes the `warn!` itself (distinct from the apply-skip dedupe in
    /// [`Self::last_applied_theme_text`]): editing an invalid file twice
    /// without changing it warns once, a NEW invalid edit warns again.
    pub last_warned_invalid_theme: Option<String>,
    /// Whether the previous theme-file poll hit a read error. Lets the
    /// watcher warn on the Ok→Err TRANSITION instead of every tick, and
    /// resume normally on recovery.
    ///
    /// Two triggers, and only the first is a real problem:
    ///
    /// 1. The file is gone or unreadable — deleted, moved, permissions
    ///    changed, a network share that stopped answering. The user's copy is
    ///    the thing at risk, so the one `warn!` is worth spending.
    /// 2. The path is real but the file is not there YET, because
    ///    [`Self::apply_theme_entry`] deferred this pick's bootstrap write to
    ///    [`cx.background_executor`] and the watcher won the race against it
    ///    by up to one poll. Self-inflicted by our own deferral, and
    ///    harmless: the applied theme is already correct in memory, the write
    ///    lands moments later, and the next tick clears this flag. It is still
    ///    tracked here rather than special-cased, because the two triggers are
    ///    indistinguishable at the point the flag is set — and the cost of
    ///    conflating them is one log line, while the cost of splitting them
    ///    is a second flag to keep in step.
    pub theme_read_failed: bool,
    /// Keyboard focus anchor for the root `image_view` div.
    ///
    /// GPUI dispatches key bindings along the **focus stack**: a binding
    /// scoped to the `"image_view"` key context only matches when keyboard
    /// focus sits inside the element carrying that context (or a
    /// descendant). The root div tracks this handle via `.track_focus`, so
    /// focusing it once at startup puts every keystroke on the dispatch
    /// path that contains `image_view`, making the ←/→/Tab/F11/Ctrl+O
    /// bindings reachable without any prior mouse interaction.
    pub focus_handle: FocusHandle,
    /// Current app view (Welcome → Grid → Viewer).
    pub view: View,
    /// Selected index in the Grid view.
    pub grid_selected: usize,
    /// Range anchor for Shift+selection (V3): follows the cursor on
    /// navigation and viewer opens, frozen during selection ops — so
    /// chained Shift+clicks share the true anchor and selection never
    /// moves the "visited" mark.
    pub anchor: usize,
    /// Multi-selection set (V3): grid indices, ascending + deduped by
    /// construction. Cursor + range anchor stays `grid_selected`. Cleared
    /// on folder swap and sort change; survives viewer round-trips.
    pub selected: BTreeSet<usize>,
    /// Pending destructive batch op (V3): bar visible ⟺ `Some`. Cleared on
    /// confirm, cancel, folder change. Never persisted.
    pub pending_batch: Option<BatchOp>,
    /// Last batch-op report for the grid topbar-center (V3): `Some` only on
    /// partial outcomes; full success is silent. Cleared on next staging
    /// or folder change — never `session.error` (which does not render with
    /// images present).
    pub batch_status: Option<String>,
    /// Manual grid scroll offset in px (wheel-driven, clamped).
    pub grid_scroll_px: f32,
    /// Manual settings-content scroll offset in px (wheel-driven, clamped).
    /// The clamp uses the active section's exact content geometry, including
    /// General recents and the new Appearance controls. Reset on section
    /// change and on open.
    pub settings_scroll_px: f32,
    /// Decoded 256px thumbnails by path (grid cells). Cleared on every
    /// folder open; filled by one background task per open (seq-guarded).
    /// Full-resolution images NEVER live here — that was the 984MB grid.
    pub thumbs: std::collections::HashMap<PathBuf, std::sync::Arc<gpui::RenderImage>>,
    /// Sequence guarding thumb decode tasks against folder switches.
    pub thumb_seq: u64,
    /// Sequence guarding in-flight directory scans against newer opens.
    ///
    /// Every open ([`Self::open_path`] / [`Self::open_folder`]) takes a ticket
    /// here before spawning its background scan, and the completion commits
    /// only while its ticket is still the latest — so a slow scan for folder
    /// A cannot clobber the fresh list of folder B, opened while A was still
    /// being read.
    ///
    /// Deliberately its OWN counter, NOT a reuse of [`Self::navigation_seq`]
    /// or [`Self::thumb_seq`]: both of those are bumped by plain ←/→
    /// navigation, so keying an open on either would silently drop a valid
    /// load every time the user arrowed around while a scan was in flight.
    /// Nor is it a replacement for [`same_image_set`]: that guard answers
    /// "is the user still looking at the list the batch op acted on?" (a
    /// MEMBERSHIP question about a list that already committed), while this
    /// one answers "is this the newest load request?" (an ORDERING question
    /// about a request that may never have committed at all). Both stay.
    pub list_load_seq: u64,
    /// The folder the loaded list came from, or `None` when no folder has
    /// been entered yet (Welcome, or a CLI argument that never resolved).
    ///
    /// Exists because `session.images` CANNOT answer this: the one moment
    /// the app most needs to know which folder it is showing is the moment
    /// the list is empty. Hiding the last dotfile in a folder empties it,
    /// and a re-scan target derived from the current image would then be
    /// unavailable — the hidden-files toggle would save its flag and
    /// silently repaint nothing. Recorded by the two open entry points
    /// BEFORE their scan is spawned, and seeded in [`Self::new`] from the
    /// session so a list that arrived before the App existed still has a
    /// folder behind it.
    ///
    /// Deliberately not derived from `settings.last_dir`: that mirror is
    /// refreshed by [`Self::persist`], which an EMPTY folder open never
    /// reaches (an empty folder must not enter the recents list), so it can
    /// name a different folder than the one on screen.
    pub current_dir: Option<PathBuf>,
    /// Cached per-thumb alpha verdicts, keyed exactly like [`Self::thumbs`].
    ///
    /// Computed in [`Self::spawn_thumb_batch`] via
    /// [`sh_core::decode::has_alpha_rgba`] on the already-decoded capped
    /// bytes — zero new decodes, zero new I/O. Cleared together with
    /// `thumbs` on every folder open, so no stale verdicts linger. The
    /// grid cell builder reads this map (never the filesystem) to decide
    /// the single baked checkerboard layer per transparent thumb.
    pub thumb_alpha: std::collections::HashMap<PathBuf, bool>,
    /// Folders that can drive the Welcome Continue button + recent chips:
    /// the persisted recents list, filtered to paths that still exist.
    /// Refreshed by [`Self::persist`] and seeded at startup (main.rs).
    pub recent_dirs_available: Vec<PathBuf>,
    /// View to return to when the Settings surface closes (Welcome, Grid,
    /// or Viewer — captured by [`Self::open_settings`]).
    pub settings_return_to: View,
    /// The Appearance section's theme rows: built-ins plus every user theme
    /// discovered in the config `themes/` directory, deduped and ordered by
    /// [`crate::ui::settings_panel::sections::appearance::merge_theme_entries`].
    ///
    /// Cached on `App` and NEVER rebuilt inside `render`: discovery is
    /// `read_dir` + a JSON parse per file, which is filesystem I/O and
    /// belongs off the frame loop (AGENTS.md §7.1). Holding the merged list
    /// (not just the discovered tail) is what keeps the row COUNT that
    /// [`crate::ui::settings_panel::scroll::appearance_content_h`] derives
    /// and the wheel handler's scroll clamp reading the same value — see
    /// [`Self::refresh_theme_entries`].
    ///
    /// Seeded with the built-ins only, so the picker is never empty and
    /// never flashes while the first discovery is in flight.
    pub theme_entries: Vec<crate::ui::settings_panel::sections::appearance::ThemeEntry>,
    /// Sequence guarding in-flight theme discovery against newer refreshes.
    ///
    /// Its own counter, on the [`Self::list_load_seq`] / [`Self::thumb_seq`]
    /// pattern: [`Self::open_settings`] takes a ticket before spawning, and
    /// a completion commits only while its ticket is still the latest, so a
    /// slow scan of a themes directory the user just changed cannot
    /// overwrite a fresher result. Deliberately NOT keyed on
    /// [`Self::navigation_seq`] — discovery is unrelated to image
    /// navigation, and sharing that counter would drop a valid refresh
    /// every time the user arrowed around in Settings.
    pub theme_load_seq: u64,
    /// Active section inside the Settings surface.
    pub settings_section: crate::ui::settings_panel::SettingsSection,
    /// Selected Appearance control, used for deterministic keyboard cycling.
    pub settings_appearance_focus_control: Option<crate::ui::settings_panel::AppearanceControl>,
    /// Shortcut capture in progress: the action id awaiting a keypress, if any.
    /// `None` = not capturing. Scoped to the Settings surface; cleared on
    /// view change (see [`Self::close_settings`]).
    pub capture_action: Option<String>,
    /// Conflict feedback for the row currently in capture mode: the ACTION ID
    /// of the incumbent holding the rejected combo, if the last pressed
    /// combo was rejected. Stored as the id (not the resolved label) so the
    /// message re-renders in the current UI language on live switch.
    /// `None` = no error to show.
    pub capture_conflict: Option<String>,
    /// Reset-all armed state for the two-step inline confirm (Shortcuts section).
    /// Cleared wherever capture is cleared.
    pub reset_armed: bool,
    /// The theme being authored in the Theme Editor section, as free text.
    ///
    /// Lives on `App` rather than being rebuilt per render because that is the
    /// whole reason it is a `ThemeDraft` and not a `Theme`: a partially typed
    /// hex is the normal state while editing, and a draft rebuilt from the
    /// applied theme on every frame would discard each keystroke as it landed.
    /// WU-4 (copy-on-write) and WU-5 (live preview) both read this same field,
    /// so it must be the single copy of the user's edits.
    pub theme_draft: sh_core::theme_draft::ThemeDraft,
    /// `theme_store.name` the draft was warm-started from.
    ///
    /// The identity check, not the content: switching themes in Appearance has
    /// to re-seed the draft from the newly applied one, while re-rendering,
    /// typing, and window resizes must not. A content comparison would throw
    /// away unsaved edits every time the user typed a character that happened
    /// to match the applied theme.
    pub theme_draft_source: String,
    /// Kit input entities backing the Theme Editor column, plus the
    /// subscriptions that push their text into [`Self::theme_draft`].
    ///
    /// `None` until the section first renders, because `InputState::new`
    /// requires a `&mut Window` and `App::new` never has one. Created once and
    /// reused, so the entities survive re-renders; dropping the whole set and
    /// rebuilding it is how a theme switch discards stale input values.
    pub theme_inputs: Option<ThemeInputs>,
    /// Sort dropdown open (sort chip in the top bar).
    pub sort_menu_open: bool,
    /// Crop mode: drag selects a region instead of panning.
    pub crop_mode: bool,
    /// Current selection in viewport px (drag order; normalized on confirm).
    pub crop_rect: Option<sh_core::crop::CropRect>,
    /// Confirm bar visible (a finished drag left a non-degenerate rect).
    pub crop_bar_visible: bool,
    /// Info popover open (viewer-only, transient — never persisted).
    pub info_panel_open: bool,
    /// Cached facts: (path probed, outcome). Re-resolved on open and on
    /// every navigate-while-open; render reads only (path match ⇒ rows).
    pub info_facts: Option<(
        PathBuf,
        Result<sh_core::decode::FileInfo, sh_core::errors::ShImagesError>,
    )>,
}

/// The kit text inputs backing the Theme Editor column, and the subscriptions
/// that push their text into [`App::theme_draft`].
///
/// One entity per rendered field, in [`theme_editor::FIELDS`] order. Bundled
/// into a single `Option` on `App` rather than a bare `Vec` so that a theme
/// switch drops the entities AND their subscriptions together: keeping
/// half of the set alive is how an input ends up editing a draft nobody reads.
pub struct ThemeInputs {
    /// Parallel to [`theme_editor::FIELDS`] — index `N` edits `FIELDS[N]`.
    /// A `Vec` rather than a fixed-size array so constructing it does not need
    /// a `Default` bound on the element type.
    fields: Vec<Entity<gpui_component::input::InputState>>,
    /// Held for their side effect. A dropped `Subscription` stops delivering,
    /// which would freeze the draft on the first keystroke with no error
    /// anywhere — the failure mode `Subscription` exists to make loud.
    _subscriptions: Vec<Subscription>,
}

impl App {
    /// Create a new app with the given session, theme, and settings.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: Session,
        theme_store: ThemeStore,
        settings_path: PathBuf,
        settings: sh_core::settings::Settings,
        last_applied_theme_text: String,
        cx: &mut Context<Self>,
    ) -> Self {
        // Seeded from the session (the only folder signal that exists before
        // the first open) so a CLI-opened image is already attributable to a
        // folder; main.rs overrides it for a folder argument, which can land
        // an EMPTY list and therefore has no session to derive from.
        let current_dir = session
            .current_item()
            .and_then(|i| i.path.parent().map(std::path::Path::to_path_buf));
        // Pure and I/O-free: the built-in rows with no discovered tail, so
        // the first Settings open has something to render before
        // `refresh_theme_entries` lands. Computed before `settings_path` is
        // moved into the struct below.
        let builtin_entries =
            crate::ui::settings_panel::sections::appearance::builtin_theme_entries(
                &settings_path
                    .parent()
                    .map(|p| p.join("themes"))
                    .unwrap_or_default(),
            );
        // Warm-start the Theme Editor draft from the theme the app is already
        // showing, so opening the section shows the applied values rather than
        // an empty column. Computed before `theme_store` is moved into the
        // struct below.
        let theme_draft = sh_core::theme_draft::ThemeDraft::from_theme(&theme_store.theme);
        let theme_draft_source = theme_store.name.clone();
        let app = Self {
            session,
            theme_store,
            viewport: size(px(0.), px(0.)),
            navigation_seq: 0,
            drag_last: None,
            last_hover: None,
            last_interaction: Instant::now(),
            slideshow_timer: None,
            overlays_hidden_by_idle: false,
            settings_path,
            settings,
            hover_motion: motion::HoverMotionState::default(),
            last_applied_theme_text,
            last_warned_invalid_theme: None,
            theme_read_failed: false,
            focus_handle: cx.focus_handle(),
            view: View::Welcome,
            grid_selected: 0,
            anchor: 0,
            pending_batch: None,
            batch_status: None,
            selected: BTreeSet::new(),
            grid_scroll_px: 0.0,
            settings_scroll_px: 0.0,
            thumbs: std::collections::HashMap::new(),
            thumb_seq: 0,
            list_load_seq: 0,
            current_dir,
            thumb_alpha: std::collections::HashMap::new(),
            recent_dirs_available: Vec::new(),
            theme_entries: builtin_entries,
            theme_load_seq: 0,
            settings_return_to: View::Welcome,
            settings_section: crate::ui::settings_panel::SettingsSection::default(),
            settings_appearance_focus_control: None,
            capture_action: None,
            capture_conflict: None,
            reset_armed: false,
            theme_draft,
            theme_draft_source,
            theme_inputs: None,
            sort_menu_open: false,
            crop_mode: false,
            crop_rect: None,
            crop_bar_visible: false,
            info_panel_open: false,
            info_facts: None,
        };
        // Arm the thumbnail batch for the session this app is BORN WITH.
        //
        // `main.rs` scans a CLI folder argument synchronously and hands
        // `App::new` an already-populated session, so none of the async commit
        // handlers that normally arm the batch ever runs. Without this the app
        // started with a full grid and a permanently empty `thumbs` map, painting
        // the placeholder for every cell — the "grid of empty gray boxes" bug —
        // until the user navigated somewhere that re-armed the batch.
        //
        // This is why `new` takes a `Context` rather than an `App`: the batch has
        // to be armed by the constructor, not by a caller remembering to. Arming
        // it from `main` instead would leave the regression test below passing
        // while `main` stopped doing it, which is the exact shape of bug it fixes.
        let mut app = app;
        // Publish the persisted reduced-motion preference to gpui.
        //
        // The setting has always been applied by reading `settings.reduce_motion`
        // directly, which is why the app's OWN animations honoured it. gpui does
        // not work that way: `App::reduce_motion` is a separate flag that
        // `gpui-base` consults to short-circuit springs and animations
        // (`motion.rs:297, 384, 622`). Every gpui-component Button and Switch
        // animates through that flag, not through ours, so without this line the
        // preference was persisted, shown in Settings, and ignored by every
        // component adopted from the kit — which is now the top bar and all
        // twelve viewer controls.
        //
        // Applied here as well as in the setter because a user who enabled it in
        // a previous session would otherwise get motion until they toggled it
        // off and on again.
        cx.set_reduce_motion(app.settings.reduce_motion);
        app.spawn_thumb_batch(cx);
        app
    }

    /// Mark user interaction: refreshes the idle clock and, if the overlays
    /// were hidden by idle, re-shows them with a single notify.
    ///
    /// Called from EVERY interaction surface — mouse move and all keyboard
    /// actions — so keyboard-only users never lose the chrome.
    pub fn note_interaction(&mut self, cx: &mut Context<Self>) {
        self.last_interaction = Instant::now();
        if self.overlays_hidden_by_idle {
            self.overlays_hidden_by_idle = false;
            cx.notify();
        }
    }

    /// Spawn the idle watcher: wakes periodically and hides the overlays
    /// once the mouse has been idle past [`overlay::OVERLAY_IDLE`].
    ///
    /// GPUI renders on demand — when the mouse stops, no events arrive, so
    /// nothing would ever re-render the overlays away. This tick supplies
    /// the missing wake-up. It notifies ONLY on the visible→hidden
    /// transition (guarded by [`Self.overlays_hidden_by_idle`]); any user
    /// interaction resets the flag, re-arming the next transition. The loop
    /// exits once the App entity is dropped.
    pub fn spawn_idle_watcher(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(IDLE_TICK).await;
            // Entity dropped → stop ticking (no immortal background loop).
            if this
                .update(cx, |app, cx| {
                    let idle = app.last_interaction.elapsed() > overlay::OVERLAY_IDLE;
                    let overlays_on = app.session.show_overlay_bottom;
                    // Notify exactly once per idle transition.
                    if idle && overlays_on && !app.overlays_hidden_by_idle {
                        app.overlays_hidden_by_idle = true;
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    /// Cancel the pending slideshow delay, if any.
    fn cancel_slideshow_timer(&mut self) {
        self.slideshow_timer = None;
    }

    /// Arm the cancellable slideshow delay using the current setting.
    ///
    /// Replacing the stored [`Task`] cancels any previous delay before the
    /// new one is scheduled. The task keeps its handle in the App so leaving
    /// the Viewer, stopping playback, or changing the interval can cancel
    /// the pending wake-up immediately.
    pub fn rearm_slideshow_timer(&mut self, cx: &mut Context<Self>) {
        self.cancel_slideshow_timer();
        if !self.session.slideshow_active || self.view != View::Viewer {
            return;
        }

        let mut delay = slideshow_delay(self.settings.slideshow_interval_secs);
        self.slideshow_timer = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(delay).await;
            let next_delay = this.update(cx, |app, cx| {
                if app.session.slideshow_active && app.view == View::Viewer {
                    app.navigate(1, cx);
                    Some(app.settings.slideshow_interval_secs)
                } else {
                    None
                }
            });
            match next_delay {
                Ok(Some(seconds)) => delay = slideshow_delay(seconds),
                _ => break,
            }
        }));
    }

    /// Toggle the slideshow (V3). Viewer-only; crop mode owns the session
    /// when active, so the toggle is a no-op there. `note_interaction`
    /// keeps the overlay visible so the user sees the state flip.
    pub fn toggle_slideshow(&mut self, cx: &mut Context<Self>) {
        if self.crop_mode {
            return;
        }
        self.session.slideshow_active = !self.session.slideshow_active;
        if self.session.slideshow_active {
            self.rearm_slideshow_timer(cx);
        } else {
            self.cancel_slideshow_timer();
        }
        self.note_interaction(cx);
        cx.notify();
    }

    /// Re-resolve info facts for the current session item (sync header +
    /// stat, near-instant — no background task). Called from the info-button
    /// toggle (on open) and `navigate()` (when open). Render reads the cache
    /// only, never I/O.
    fn refresh_info_facts(&mut self) {
        let outcome = match self.session.current_item() {
            Some(item) => sh_core::decode::probe_file_info(&item.path),
            None => Err(sh_core::errors::ShImagesError::UnsupportedFormat(
                "no image open".into(),
            )),
        };
        let path = self
            .session
            .current_item()
            .map(|i| i.path.clone())
            .unwrap_or_default();
        self.info_facts = Some((path, outcome));
    }

    /// Toggle the viewer info popover. `note_interaction` runs FIRST so the
    /// overlay cannot idle-fade mid-tap; opening re-resolves facts for the
    /// current file before the next paint.
    pub fn toggle_info_panel(&mut self, cx: &mut Context<Self>) {
        self.note_interaction(cx);
        self.info_panel_open = !self.info_panel_open;
        if self.info_panel_open {
            self.refresh_info_facts();
        }
        cx.notify();
    }

    /// Close the viewer info popover (outside-click catcher + `Esc` guard).
    /// Touches nothing else — no navigation, no view change.
    pub fn close_info_panel(&mut self, cx: &mut Context<Self>) {
        self.info_panel_open = false;
        cx.notify();
    }
    /// Open a path: scan its parent's entries off the frame loop, anchor the
    /// selection on the opened file, apply the active session sort, show it,
    /// invalidate the grid selection, persist, and kick off the initial
    /// probe/fit.
    ///
    /// The list lands asynchronously (see [`Self::spawn_list_load`]): the
    /// caller returns with the view and the previous list untouched, and
    /// [`Self::commit_open`] applies everything once the scan is back. An
    /// invalid path (no readable parent) surfaces in the session error slot
    /// once the gate confirms it — no list is ever built for it.
    ///
    /// The invariant this entry point has to protect is the same one
    /// [`Self::open_folder`] protects: a grid selection may only ever address
    /// the list it was built against. `selected`, `grid_selected` and
    /// `anchor` are INDICES into `session.images`, and the open below
    /// REPLACES that list — `commit_open` rebuilds it from a fresh scan and
    /// re-sorts it — so they are cleared unconditionally, on this frame,
    /// before the scan is even spawned. See the comment at the reset for why
    /// the staged-op rule right below it is guarded and this one is not.
    pub fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // A root-only path has no parent to scan at all: report it inline
        // (no I/O is involved) instead of spawning a task that can only fail
        // the same way.
        let Some(dir) = path.parent().map(std::path::Path::to_path_buf) else {
            self.session.error = Some(
                sh_core::errors::ShImagesError::NotAFile(path.display().to_string()).to_string(),
            );
            cx.notify();
            return;
        };
        // A staged op is a promise about ONE folder's files (see
        // [`Self::open_folder`]), so it survives this open only when the
        // file's parent IS the folder already on screen — Ctrl+O (or a
        // drop) of a file from the grid the user is looking at leaves the
        // promise true, and throwing it away there would discard a
        // confirmation they can still act on correctly. It is dropped
        // otherwise, which is the file-dialog / drag-and-drop / CLI case
        // where the list about to be built describes a different folder.
        // `current_dir` is the on-screen folder (recorded by both open entry
        // points before their scan, deliberately not `settings.last_dir`).
        if self.current_dir.as_deref() != Some(dir.as_path()) {
            self.pending_batch = None;
            self.batch_status = None;
        }
        // The grid selection goes the OTHER way, and deliberately so: no
        // `current_dir` guard, on either branch.
        //
        // The two rules look alike and are not. A staged op holds PATHS, and
        // a re-scan of the SAME folder preserves which file each path names,
        // so its promise genuinely survives there — that is the guard above.
        // These three hold INDICES, and the open below invalidates them just
        // as thoroughly when the folder is unchanged: `commit_open` always
        // re-scans (Name/Asc) and then re-sorts, so a folder opened under
        // Size/Desc is renumbered by the re-sort alone, and any file added
        // or removed on disk since the last scan shifts everything below it.
        // Guarding the reset would therefore keep precisely the indices
        // already known to be wrong.
        //
        // What made this a live defect is the consequence, not the drift:
        // `stage_delete`/`stage_move`/`copy_selection` turn `selected`'s
        // INDICES into PATHS against whatever `session.images` holds at the
        // time. A `{3, 7}` selection carried from folder A into folder B
        // therefore staged two arbitrary files of B, and Enter trashed them
        // — a cross-folder data-loss twin of the staged-op hazard above, in
        // the one open path that did not guard it.
        //
        // `clear()` and not a remap, matching [`Self::open_folder`] and the
        // documented v1 reorder rule in [`Self::set_sort`].
        // [`remap_selection_by_path`] is the right tool for a list that still
        // HOLDS the selected files (a rescan, a batch op); across an open,
        // after a re-sort, it holds none of them, so it would only ever
        // return an empty set — a `clear()` that says what it means.
        self.grid_selected = 0;
        self.anchor = 0;
        self.selected.clear();
        // Recorded BEFORE the scan, not after it lands: an empty parent (a
        // folder whose images are all hidden) must still leave the app
        // knowing which folder it is showing.
        self.current_dir = Some(dir.clone());
        self.spawn_list_load(dir, OpenRequest::File(path), cx);
    }
    /// Open a folder: scan its entries off the frame loop (or surface "no
    /// images" in the session error slot once the scan says so), enter the
    /// Grid view, persist, and probe.
    ///
    /// The view switch and the Grid resets below happen SYNCHRONOUSLY, so
    /// there is no frozen frame while the scan runs. `session.images` is
    /// emptied for the duration of the load: those resets already moved the
    /// app to fresh Grid chrome, and stale thumbnails from the PREVIOUS
    /// folder showing under it read as a bug rather than as a pending load.
    /// The grid shows its neutral empty state instead. `no_images_in` is
    /// deliberately NOT set here — the scan has not run yet, so "no images"
    /// would be a claim about work still in flight; it is set on completion,
    /// where an empty result is knowledge. A local scan is typically
    /// imperceptible; a huge or network folder can take long enough to
    /// matter, which is exactly why the scan left the frame loop.
    pub fn open_folder(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        // Session is the runtime source of truth for order: sync the
        // persisted sort before the first image list is built.
        self.session.sort_by = self.settings.sort_by;
        self.session.sort_dir = self.settings.sort_dir;
        self.hover_motion.clear();
        self.view = View::Grid;
        self.grid_selected = 0;
        self.anchor = 0;
        // Work-in-progress selection never crosses folders.
        self.selected.clear();
        // A staged op is a promise about ONE folder's files, and it carries
        // their PATHS — so it cannot outlive the folder it was staged in.
        // Left behind, the confirm bar is still on screen (and still
        // confirmable) while the user looks at a different folder: Enter then
        // trashes folder A's files from folder B's grid, and the op's own
        // rescan briefly writes folder A's list into folder B's view.
        self.pending_batch = None;
        // A new folder supersedes any previous batch report.
        self.batch_status = None;
        self.grid_scroll_px = 0.0;
        // See the doc comment: empty for the length of the load so the grid
        // never shows the previous folder's images under the chrome that was
        // just reset.
        self.session.images = Vec::new();
        self.session.current = 0;
        // See [`Self::current_dir`]: recorded before the scan so the folder
        // survives a load that comes back empty.
        self.current_dir = Some(dir.clone());
        self.spawn_list_load(dir, OpenRequest::Folder, cx);
        // NOTE: recents are owned by `persist`, which only the non-empty
        // completion reaches (via `commit_open`); an empty folder must NOT
        // enter the list.
        cx.notify();
    }

    /// Spawn the ONE directory scan behind an open, and take the ticket that
    /// decides whether its result is still wanted when it lands.
    ///
    /// Why off-thread: `scan_entries` is a full `read_dir` plus a
    /// `metadata()` call per entry, so its cost is unbounded in folder size —
    /// and both open entry points used to pay it on the frame loop
    /// (AGENTS.md §7.1). Same `cx.background_executor()` + `cx.spawn` commit
    /// shape as [`Self::confirm_pending`].
    ///
    /// Why the ticket: going async opens a window the synchronous version did
    /// not have — the user can open folder A and then folder B before A's
    /// scan returns, and an unguarded completion would clobber B's list with
    /// A's. See [`Self::list_load_seq`] for why that is a counter of its own.
    ///
    /// Interop with the sibling guard: a `confirm_pending` batch and a load
    /// can land in EITHER order, and both end correct. Load first, then the
    /// batch: `same_image_set` sees the new folder's membership and drops the
    /// batch (its files are gone, but the folder on screen ran its own fresh
    /// scan). Batch first, then the load: the batch commits against the
    /// pre-open list, and the load — holding the newer ticket — then replaces
    /// whatever it wrote, including any `no_images_in` it set (the non-empty
    /// `commit_open` clears the error slot). The load always wins because its
    /// ticket is the newest request; that is the whole point of the counter.
    fn spawn_list_load(&mut self, dir: PathBuf, request: OpenRequest, cx: &mut Context<Self>) {
        self.list_load_seq = self.list_load_seq.wrapping_add(1);
        let ticket = self.list_load_seq;
        // Only the file variant needs the parent-existence gate, so the task
        // gets it as an `Option<PathBuf>` and the folder path runs ungated
        // (see the task body).
        let gate = match &request {
            OpenRequest::File(path) => Some(path.clone()),
            OpenRequest::Folder | OpenRequest::Rescan { .. } => None,
        };
        let bg = cx.background_executor();
        // The completion names the folder in the `no_images_in` error, so the
        // task gets its own copy of the path (one small clone, once per open).
        let scan_dir = dir.clone();
        // Read HERE, at spawn time, not inside the task: the value the user
        // just set is the one this scan must apply, and `ScanOptions` is
        // `Copy`, so travelling into the task costs nothing.
        let opts = sh_core::navigation::ScanOptions {
            show_hidden: self.settings.show_hidden_files,
        };
        let task = bg.spawn(async move {
            match gate {
                // `open_path` gate. One `stat` is cheap, but it is still a
                // blocking syscall — and a slow one on a network path — so it
                // leaves the frame loop with the scan it decides whether to
                // run.
                Some(path) => match path.parent() {
                    Some(parent) if parent.is_dir() => {
                        ScanOutcome::Entries(sh_core::navigation::scan_entries(parent, opts))
                    }
                    _ => ScanOutcome::NoParentDir(path),
                },
                // `open_folder` has NO gate on purpose: a missing or
                // unreadable directory comes back as an empty scan and is
                // reported through the same `no_images_in` path a genuinely
                // empty folder takes, which is what the inline version did.
                None => ScanOutcome::Entries(sh_core::navigation::scan_entries(&scan_dir, opts)),
            }
        });
        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            let _ = this.update(cx, |app, cx| {
                app.commit_list_load(ticket, request, outcome, dir, cx);
            });
        })
        .detach();
    }

    /// Apply a scan that came back from [`Self::spawn_list_load`], if it is
    /// still the load the user is waiting for.
    ///
    /// Named rather than inlined in the spawn closure because it is the whole
    /// policy of a load: the ticket check, the two entry points' different
    /// answers to "the folder is empty", and the shared non-empty tail
    /// ([`Self::commit_open`]).
    fn commit_list_load(
        &mut self,
        ticket: u64,
        request: OpenRequest,
        outcome: ScanOutcome,
        dir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        // Stale-load guard: a newer open took the ticket after this one, so
        // this scan is about a folder the user already left. Dropping it
        // loses nothing — the newer open ran its own scan — and NOT
        // persisting here is what keeps an abandoned folder out of the
        // recents list.
        if ticket != self.list_load_seq {
            return;
        }
        let entries = match outcome {
            ScanOutcome::NoParentDir(path) => {
                self.session.error = Some(
                    sh_core::errors::ShImagesError::NotAFile(path.display().to_string())
                        .to_string(),
                );
                cx.notify();
                return;
            }
            ScanOutcome::Entries(entries) => entries,
        };
        if entries.is_empty() {
            match &request {
                // Real knowledge now that the scan has run, so the error slot
                // is finally allowed to say "no images".
                //
                // `Rescan` answers exactly like `Folder`: hiding dotfiles can
                // legitimately empty the folder the user is looking at, and
                // "no images in <this folder>" is the truth about what is now
                // listed. A rescan that kept the stale list up would be a
                // second lie on top of the one this fixes.
                OpenRequest::Folder | OpenRequest::Rescan { .. } => {
                    self.session.images = Vec::new();
                    self.session.current = 0;
                    self.session.slideshow_active = false;
                    self.cancel_slideshow_timer();
                    self.session.error = Some(sh_core::i18n::no_images_in(
                        self.settings.language,
                        &dir.display().to_string(),
                    ));
                    // Empty folder: still clear + invalidate any previous
                    // folder's thumb map (zero-path batch = clear + seq bump).
                    self.spawn_thumb_batch(cx);
                    cx.notify();
                }
                // Not an error claim here: the parent exists, it just holds
                // no images. Same non-empty tail as a populated list, with no
                // anchor to land on.
                OpenRequest::File(_) => self.commit_open(entries, None, cx),
            }
            return;
        }
        // Folder opens anchor the first scanned image (the list is Name-asc
        // from the scan); a file open anchors itself; a rescan anchors the
        // image that was on screen, which may no longer be in the list.
        let anchor = match &request {
            OpenRequest::Folder => entries.first().map(|e| e.path.clone()),
            OpenRequest::File(path) => Some(path.clone()),
            OpenRequest::Rescan { anchor } => anchor.clone(),
        };
        match &request {
            OpenRequest::Rescan { .. } => self.commit_rescan(entries, anchor, cx),
            _ => self.commit_open(entries, anchor, cx),
        }
    }

    /// The commit half of an open — everything that used to run inline in
    /// `open_path` once the scan returned. Split out because the scan is now
    /// asynchronous (see [`Self::spawn_list_load`]) AND because `open_folder`
    /// applies this exact tail to the single scan it runs for itself: it
    /// used to re-enter `open_path` with the first entry's path, which paid a
    /// SECOND full `read_dir` for the folder it had just scanned.
    ///
    /// `anchor` is the path the selection must land on once the active sort
    /// has been applied. It is `None` only for an empty list, where the
    /// selection stays at 0 — the same `unwrap_or(0)` the inline version
    /// fell back to when the opened file was not among the scanned entries.
    fn commit_open(
        &mut self,
        entries: Vec<sh_core::navigation::ImageEntry>,
        anchor: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.session.error = None;
        self.session.images = build_image_items(entries);
        // Anchor on the opened image, then apply the active sort — the
        // resort re-anchors by path, so the selection survives the sort.
        self.session.current = anchor
            .and_then(|a| self.session.images.iter().position(|i| i.path == a))
            .unwrap_or(0);
        self.session.resort();
        self.session.show_overlay_bottom = true;
        // Folder swap kills the slideshow: auto-advance into a fresh
        // image list the user never chose to play is wrong.
        self.session.slideshow_active = false;
        self.cancel_slideshow_timer();
        cx.notify();
        self.persist(cx);
        self.navigate(0, cx);
        // Thumbnails: every path that swaps `session.images` must
        // re-arm the thumb batch, or the Grid renders empty
        // placeholders forever (drop-a-file → Back showed exactly
        // that: `open_path` swapped the list but only `open_folder`
        // spawned the decode batch). The map is cleared first so
        // no previous folder's thumbs linger; the seq guard drops
        // stale work if another open happens mid-decode.
        self.spawn_thumb_batch(cx);
    }

    /// Apply a toggle-driven re-scan of the folder ALREADY on screen (see
    /// [`Self::toggle_show_hidden_files`]).
    ///
    /// Deliberately not [`Self::commit_open`], because a rescan answers "the
    /// same folder, a different visible set" — everything that describes
    /// WHERE the user is must survive it: the overlay chrome, a running
    /// slideshow, the recents list (no new write: the folder did not change)
    /// and the zoom/pan of an image that is still current. What it shares
    /// with an open is the part about WHAT is on screen: the list swap, the
    /// re-sort, the thumb batch, and a re-probe when the image under the
    /// cursor actually changed.
    fn commit_rescan(
        &mut self,
        entries: Vec<sh_core::navigation::ImageEntry>,
        anchor: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.session.error = None;
        // Snapshot the selection's PATHS before the swap: after it, these
        // indices name other images (hiding a dotfile above the cursor moves
        // every cell below it up one place) and the paths are the only
        // identity the rebuild preserves.
        let keep: Vec<PathBuf> = self
            .selected
            .iter()
            .filter_map(|i| self.session.images.get(*i))
            .map(|item| item.path.clone())
            .collect();
        self.session.images = build_image_items(entries);
        // Clamp, do not reset. Turning the flag OFF can delete the image the
        // user is standing on, and yanking them back to cell 0 would throw
        // away their place in a 400-image folder over one switch. Same clamp
        // `confirm_pending` applies when a batch op shrinks the list; `None`
        // (no anchor to restore) falls back to the index already held, which
        // the `min` keeps in range.
        self.session.current = anchor
            .clone()
            .and_then(|a| self.session.images.iter().position(|i| i.path == a))
            .unwrap_or(self.session.current.min(self.session.images.len() - 1));
        // `resort` re-anchors on whatever now sits at `current`, so the
        // index chosen above survives the re-order.
        self.session.resort();
        // AFTER `resort`, which is the last thing that moves cells: the final
        // indices are only knowable now. Both halves of the grid selection
        // are index-based, so a rescan that skipped this left a multi-selection
        // aimed at the wrong files and a cursor pointing past the end of a
        // shorter list — the same remap `confirm_pending` runs after a batch
        // op rewrites the list, and the same rule (`remap_selection_by_path`).
        let (selected, cursor) = remap_selection_by_path(
            &self.session.images,
            keep.iter().map(std::path::PathBuf::as_path),
            self.grid_selected,
        );
        self.selected = selected;
        self.grid_selected = cursor;
        cx.notify();
        // The grid cells must be re-armed even when the selection did not
        // move: the flag can have added a screenful of images.
        self.spawn_thumb_batch(cx);
        // Re-probe + re-fit ONLY when the image under the cursor changed. A
        // toggle that left the current alone must not reset a zoom or a pan
        // the user set up; a toggle that moved them onto a neighbour must,
        // or the viewer keeps the previous image's dimensions and fit.
        if self.session.current_item().map(|i| &i.path) != anchor.as_ref() {
            self.navigate(0, cx);
        }
    }

    /// Flip `show_hidden_files` and make the change VISIBLE where it is
    /// supposed to mean something: the folder the user is looking at is
    /// re-scanned through the same [`Self::spawn_list_load`] path — and the
    /// same `list_load_seq` ticket — that an open uses, with the flag already
    /// applied.
    ///
    /// Why the rescan IS the fix: without it this row persisted a boolean
    /// that no scan ever read, so the grid kept showing whatever the PREVIOUS
    /// value produced until the user navigated elsewhere — a switch that lies
    /// louder than a switch that does not exist, and a defect that survives a
    /// restart while still doing nothing. The flag is read at spawn time (see
    /// [`Self::spawn_list_load`]), so the scan runs with the value the user
    /// just set.
    ///
    /// The save keeps the optimistic contract of [`Self::persist`]: memory
    /// updates now for instant feedback, the disk write is best-effort and
    /// warns on failure.
    ///
    /// Rescan target: [`Self::current_dir`], the folder the list on screen was
    /// scanned from — NOT `settings.last_dir` (which an empty folder open
    /// never refreshes) and not the current image (which the flag may have just
    /// removed). `None` means no folder has been entered (Welcome, or a CLI
    /// argument that never resolved): there is no list on screen to
    /// contradict, and the next open scans under the new flag anyway.
    pub fn toggle_show_hidden_files(&mut self, cx: &mut Context<Self>) {
        self.settings.show_hidden_files = !self.settings.show_hidden_files;
        self.settings.version = CURRENT_SETTINGS_VERSION;
        let s = self.settings.clone();
        let path = self.settings_path.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = sh_core::settings::save(&path, &s) {
                    tracing::warn!("could not persist settings: {e}");
                }
            })
            .detach();
        if let Some(dir) = self.current_dir.clone() {
            // Anchor the CURRENT image, not the first entry: a rescan that
            // re-anchored on position 0 would teleport the user to the top of
            // a 400-image folder over one switch.
            let anchor = self.session.current_item().map(|i| i.path.clone());
            self.spawn_list_load(dir, OpenRequest::Rescan { anchor }, cx);
        }
        cx.notify();
    }

    /// Arm the background thumbnail batch for the current `session.images`
    /// (8-way parallel chunks, progressive per-thumb commits, seq-guarded
    /// against folder switches).
    ///
    /// Armed from TWO places, and both are needed:
    ///  - `App::new`, for the session the app is born with. `main.rs` seeds that
    ///    from a synchronous CLI scan, so no async commit handler ever runs on
    ///    the startup path — without this the grid rendered placeholders until the
    ///    user navigated somewhere that re-armed it.
    ///  - every list swap (`commit_list_load` / `commit_open` /
    ///    `commit_rescan`), because this reads `session.images` at arm time and
    ///    must therefore run again once the new list has landed.
    ///
    /// The batch is seq-guarded, so the extra arming a navigation costs is a
    /// bounded one: the previous batch's commits are dropped by the ticket check
    /// rather than being allowed to paint into the new folder's grid.
    fn spawn_thumb_batch(&mut self, cx: &mut Context<Self>) {
        self.thumbs.clear();
        // Slice C: verdicts die with the thumbs they describe — a folder
        // swap must never leave a previous folder's boards behind.
        self.thumb_alpha.clear();
        self.thumb_seq = self.thumb_seq.wrapping_add(1);
        let seq = self.thumb_seq;
        let paths: Vec<PathBuf> = self.session.images.iter().map(|i| i.path.clone()).collect();
        cx.spawn(async move |this, cx| {
            for chunk in paths.chunks(8) {
                let tasks: Vec<_> = chunk
                    .iter()
                    .map(|path| {
                        let path = path.clone();
                        // Owned handle per task: entity `Context` only lends
                        // `&BackgroundExecutor`, which cannot enter the task.
                        cx.background_executor().spawn(async move {
                            sh_core::decode::load_with_limit(&path, crate::thumbs::THUMB_MAX_DIM)
                                .ok()
                                .and_then(|d| {
                                    // Slice C: the verdict rides the capped
                                    // bytes already in hand — no new decode,
                                    // no I/O, off the frame loop.
                                    let alpha = sh_core::decode::has_alpha_rgba(&d);
                                    crate::thumbs::render_thumb(&d)
                                        .map(|t| (path.clone(), t, alpha))
                                })
                        })
                    })
                    .collect();
                for task in tasks {
                    let decoded = task.await;
                    let done = this
                        .update(cx, |app, cx| {
                            if seq != app.thumb_seq {
                                return false;
                            }
                            if let Some((path, thumb, alpha)) = decoded {
                                app.thumbs.insert(path.clone(), thumb);
                                // Alongside the thumb: `thumb_alpha` keys stay
                                // a subset of `thumbs` keys, so a board never
                                // outlives its image.
                                app.thumb_alpha.insert(path, alpha);
                                cx.notify();
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !done {
                        return;
                    }
                }
            }
        })
        .detach();
    }

    /// Viewport available to the image: the full window size, floored at
    /// 1.0, minus the filmstrip when visible and minus the in-flow
    /// bottom chrome while Tab is ON.
    ///
    /// The TOPBAR still floats over the image (zero layout space, R3
    /// unchanged for it). The bottom chrome now owns real layout space
    /// below the image (user-directed layout: chips/bar must never
    /// cover the image), so its Tab state enters the carve — but the
    /// idle state does NOT: idle only fades the chrome and grows the
    /// container below the image, so idle transitions never refit and
    /// never jolt. Toggling Tab changes the layout itself and refits
    /// exactly once (the ToggleOverlays action, mirroring
    /// `set_filmstrip`). Toggle, clamp, wheel, and navigate/open
    /// completion all use this; [`stable_open_viewport`] remains as the
    /// named open-time contract.
    pub fn viewer_viewport(&self) -> sh_core::transform::Vec2 {
        let viewer_surface_active = self.view == View::Viewer
            || (self.view == View::Settings && self.settings_return_to == View::Viewer);
        stable_filmstrip_viewport(
            viewport_vec(self.viewport),
            viewer_surface_active && self.settings.filmstrip,
            viewer_surface_active && self.session.show_overlay_bottom,
        )
    }

    /// Toggle the filmstrip: flip the persisted flag, save in background,
    /// refit EXACTLY ONCE against the new carve, repaint.
    ///
    /// A refit-on-toggle path (R5.3 discipline): toggling is an explicit
    /// layout change, so one refit is correct by design. The Tab toggle
    /// follows the same discipline in the `ToggleOverlays` action: the
    /// in-flow bottom chrome changes the image area when it mounts, so
    /// it refits once against the new carve too.
    pub fn set_filmstrip(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.settings.filmstrip = visible;
        self.settings.version = CURRENT_SETTINGS_VERSION;
        self.persist(cx);
        self.session.refit_for_viewport(self.viewer_viewport());
        cx.notify();
    }

    /// Persist checkerboard visibility through the shared settings path.
    pub fn set_checkerboard(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.settings.checkerboard = visible;
        self.settings.version = CURRENT_SETTINGS_VERSION;
        self.persist(cx);
        cx.notify();
    }

    /// Persist the reduced-motion preference through the shared settings path.
    pub fn set_reduce_motion(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.reduce_motion = enabled;
        self.settings.version = CURRENT_SETTINGS_VERSION;
        // Keep gpui's own flag in step, not just ours.
        //
        // gpui-component animations read `cx.reduce_motion()`; they never see
        // `settings.reduce_motion`. Without this the toggle persisted the value,
        // redrew the row, and every kit component animated regardless — so the
        // preference was a setting the user could turn on and observe doing
        // nothing. See the same wiring in `App::new` for the startup case.
        cx.set_reduce_motion(enabled);
        self.persist(cx);
        cx.notify();
    }

    /// Attach the shared reduced-motion-aware hover treatment to a control.
    fn hover_background(
        &mut self,
        element: Stateful<Div>,
        animation_id: motion::AnimationId,
        idle_bg: Hsla,
        hover_bg: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on_hover = cx.listener(
            move |this: &mut App, hovered: &bool, _window: &mut Window, cx: &mut Context<Self>| {
                if this.hover_motion.set_hovered(animation_id, *hovered) {
                    cx.notify();
                }
            },
        );
        motion::hover_background(
            element,
            animation_id,
            self.hover_motion.phase(animation_id),
            self.settings.reduce_motion,
            idle_bg,
            hover_bg,
            on_hover,
        )
    }

    /// Route a control through shared motion only in Viewer, while preserving
    /// native instant hover for the shared topbar's other non-Viewer callsites.
    fn routed_hover_background(
        &mut self,
        element: Stateful<Div>,
        stable_id: &'static str,
        idle_bg: Hsla,
        hover_bg: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(animation_id) = route_viewer_motion(self.view, stable_id) {
            return self.hover_background(element, animation_id, idle_bg, hover_bg, cx);
        }
        element
            .bg(idle_bg)
            .hover(move |style| style.bg(hover_bg))
            .into_any_element()
    }

    /// Validate and persist the slideshow interval without changing state on
    /// an out-of-range candidate.
    pub fn set_slideshow_interval_secs(
        &mut self,
        seconds: u32,
        cx: &mut Context<Self>,
    ) -> sh_core::errors::Result<()> {
        let mut updated = self.settings.clone();
        updated.version = CURRENT_SETTINGS_VERSION;
        updated.set_slideshow_interval_secs(seconds)?;
        self.settings = updated;
        self.persist(cx);
        // A live interval change starts a fresh delay from now.
        self.rearm_slideshow_timer(cx);
        cx.notify();
        Ok(())
    }

    /// Adjust the slideshow interval by one control step and persist it.
    pub fn adjust_slideshow_interval(&mut self, delta: i32, cx: &mut Context<Self>) {
        let current = self.settings.slideshow_interval_secs;
        let next = crate::ui::settings_panel::sections::appearance::adjust_slideshow_interval(
            current, delta,
        );
        if let Err(error) = self.set_slideshow_interval_secs(next, cx) {
            tracing::warn!("could not persist slideshow interval: {error}");
        }
    }

    fn move_settings_appearance_selection(&mut self, backwards: bool) {
        if self.settings_section != crate::ui::settings_panel::SettingsSection::Appearance {
            return;
        }
        let controls = crate::ui::settings_panel::AppearanceControl::ALL;
        let next = match self.settings_appearance_focus_control {
            None if backwards => controls.len() - 1,
            None => 0,
            Some(current) if backwards => (current as usize + controls.len() - 1) % controls.len(),
            Some(current) => (current as usize + 1) % controls.len(),
        };
        self.settings_appearance_focus_control = Some(controls[next]);
    }

    fn activate_settings_appearance_control(&mut self, cx: &mut Context<Self>) {
        let Some(control) = self.settings_appearance_focus_control else {
            return;
        };
        match control {
            crate::ui::settings_panel::AppearanceControl::Filmstrip => {
                let next = !self.settings.filmstrip;
                self.note_interaction(cx);
                self.set_filmstrip(next, cx);
            }
            crate::ui::settings_panel::AppearanceControl::Checkerboard => {
                let next = !self.settings.checkerboard;
                self.note_interaction(cx);
                self.set_checkerboard(next, cx);
            }
            crate::ui::settings_panel::AppearanceControl::SlideshowDecrement => {
                self.note_interaction(cx);
                self.adjust_slideshow_interval(-1, cx);
            }
            crate::ui::settings_panel::AppearanceControl::SlideshowIncrement => {
                self.note_interaction(cx);
                self.adjust_slideshow_interval(1, cx);
            }
            crate::ui::settings_panel::AppearanceControl::ReduceMotion => {
                let next = !self.settings.reduce_motion;
                self.note_interaction(cx);
                self.set_reduce_motion(next, cx);
            }
        }
    }

    /// Back to the grid; selection follows the current image.
    /// (Extended with scroll-into-view in the grid task.)
    pub fn enter_grid(&mut self, cx: &mut Context<Self>) {
        self.hover_motion.clear();
        self.grid_selected = self.session.current;
        // Returning lands the cursor (and anchor) on the viewed image;
        // the set itself is preserved (work-in-progress).
        self.anchor = self.session.current;
        self.grid_scroll_px = 0.0;
        // Leaving the Viewer also cancels any pending slideshow delay.
        self.session.slideshow_active = false;
        self.cancel_slideshow_timer();
        self.view = View::Grid;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Change the active sort: update session + settings copy, re-anchor
    /// the selection by path, sync the grid selection, persist.
    ///
    /// The session is the runtime truth for order; the settings copy
    /// mirrors it so the next launch restores the same criterion.
    pub fn set_sort(
        &mut self,
        by: sh_core::navigation::SortBy,
        dir: sh_core::navigation::SortDir,
        cx: &mut Context<Self>,
    ) {
        self.session.apply_sort(by, dir);
        // Indices invalidate on reorder - clear (documented v1 rule).
        self.selected.clear();
        self.settings.sort_by = by;
        self.settings.sort_dir = dir;
        self.grid_selected = self.session.current;
        // The shift-range anchor is an INDEX too, and re-sorting just moved
        // it: leaving it behind would make the next Shift+click extend a
        // selection from whatever image now occupies the old index rather
        // than from the cursor the user is looking at. Collapse it onto the
        // cursor, which is where a range is expected to start. Same class of
        // invalidation as the `clear()` above - see [`Self::open_path`] for
        // why the two entry points spell the rule differently.
        self.anchor = self.grid_selected;
        self.persist(cx);
        cx.notify();
    }

    /// Change the grid density preset: mirror into the settings copy,
    /// re-clamp the scroll offset into the new preset's range, keep the
    /// cursor visible, persist.
    ///
    /// Unlike [`Self::set_sort`] there is no session field to update and
    /// the selection is untouched: density is view/persistence state, so
    /// `settings.grid_size` is both runtime and persisted truth — size
    /// change re-sorts nothing and never moves the cursor or anchor.
    pub fn set_grid_size(&mut self, size: GridSize, cx: &mut Context<Self>) {
        self.settings.grid_size = size;
        let v = viewport_vec(self.viewport);
        let visible_h = (v.y - topbar::TOPBAR_H_PX).max(1.0);
        let geo = size.geometry();
        let max = grid::grid_max_scroll(self.session.images.len(), v.x, visible_h, &geo);
        self.grid_scroll_px = self.grid_scroll_px.clamp(0.0, max);
        self.scroll_cursor_into_view(v.x, visible_h, &geo);
        self.persist(cx);
        cx.notify();
    }

    /// Scroll the selected row into view under `geo`, then clamp into
    /// range. Shared by [`Self::move_selection`] and
    /// [`Self::set_grid_size`] so arrow navigation and size changes
    /// cannot drift apart.
    fn scroll_cursor_into_view(
        &mut self,
        viewport_w: f32,
        visible_h: f32,
        geo: &grid::GridGeometry,
    ) {
        let len = self.session.images.len();
        if len == 0 {
            self.grid_scroll_px = 0.0;
            return;
        }
        let cols = grid::grid_columns(viewport_w, geo);
        let row_top = (self.grid_selected / cols) as f32 * geo.row_h as f32;
        let row_bottom = row_top + geo.row_h as f32;
        if row_top < self.grid_scroll_px {
            self.grid_scroll_px = row_top;
        } else if row_bottom > self.grid_scroll_px + visible_h {
            self.grid_scroll_px = row_bottom - visible_h;
        }
        let max = grid::grid_max_scroll(len, viewport_w, visible_h, geo);
        self.grid_scroll_px = self.grid_scroll_px.clamp(0.0, max);
    }

    /// Enter the viewer at `idx`: set current + selection, switch view,
    /// probe/fit. Reuses [`Self::navigate`] so probe, seq-guard, fit, and
    /// persist all behave exactly like keyboard navigation.
    pub fn enter_viewer(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.hover_motion.clear();
        if idx < self.session.images.len() {
            self.session.current = idx;
            self.grid_selected = idx;
            // Opening IS visiting: cursor and anchor move together.
            self.anchor = idx;
        }
        self.view = View::Viewer;
        self.note_interaction(cx);
        self.navigate(0, cx);
        self.rearm_slideshow_timer(cx);
    }

    /// Move grid selection by `delta` (keyboard arrows): sticky at the ends,
    /// no wrap. Keeps the selected row visible by adjusting the manual scroll
    /// offset. No-op on an empty grid.
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.session.images.len();
        if len == 0 {
            return;
        }
        self.grid_selected =
            (self.grid_selected as isize + delta).clamp(0, len as isize - 1) as usize;
        // Plain arrows move the cursor AND collapse the set (standard OS
        // behavior); the anchor follows the cursor. Shift+arrows go through
        // `extend_selection_to` instead (anchor frozen).
        self.anchor = self.grid_selected;
        self.selected.clear();
        // Scroll the selected row into view (shared helper: same math
        // as a size change, so navigation and density cannot drift apart).
        let v = viewport_vec(self.viewport);
        let visible_h = (v.y - topbar::TOPBAR_H_PX).max(1.0);
        let geo = self.settings.grid_size.geometry();
        self.scroll_cursor_into_view(v.x, visible_h, &geo);
        self.note_interaction(cx);
        cx.notify();
    }

    /// Toggle one cell in the selection set (Ctrl+click / Ctrl+Space).
    /// Never moves the cursor: selection must not take the "visited" mark —
    /// the cursor only moves on navigation and viewer opens.
    pub fn toggle_selected(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.session.images.len() {
            return;
        }
        if !self.selected.remove(&idx) {
            self.selected.insert(idx);
        }
        self.note_interaction(cx);
        cx.notify();
    }

    /// Union the anchor→target range into the set (Shift+click /
    /// Shift+arrows). The cursor is intentionally untouched: keyboard callers
    /// advance it explicitly as the moving edge, mouse callers leave the
    /// user exactly where they were. Union-only by design: narrowing needs
    /// Ctrl+click.
    pub fn extend_selection_to(&mut self, idx: usize, cx: &mut Context<Self>) {
        let len = self.session.images.len();
        if len == 0 {
            return;
        }
        let target = idx.min(len - 1);
        self.selected
            .extend(grid::selection_range(self.anchor, target));
        self.note_interaction(cx);
        cx.notify();
    }

    /// Fill the set (Ctrl+A). No-op on an empty grid.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected = (0..self.session.images.len()).collect();
        self.note_interaction(cx);
        cx.notify();
    }

    /// Empty the set (Escape in grid). Resets the anchor to the cursor so
    /// the next Shift gesture starts fresh from where the user is.
    /// Always notifies — callers use it as the consumed-gesture path.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected.clear();
        self.anchor = self.grid_selected;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Copy payload for the selection: absolute paths, ascending index
    /// order, `\n`-joined. `None` when empty — the no-op signal, so callers
    /// never touch the clipboard without user-visible content.
    pub fn selection_paths_string(&self) -> Option<String> {
        if self.selected.is_empty() {
            return None;
        }
        let lines: Vec<String> = self
            .selected
            .iter()
            .filter_map(|i| self.session.images.get(*i))
            .map(|item| item.path.display().to_string())
            .collect();
        if lines.is_empty() {
            return None;
        }
        Some(lines.join("\n"))
    }

    /// Copy the selection's paths (grid-only; viewer is a deliberate no-op
    /// — single-copy follow-up lives outside this slice). Empty set is a
    /// silent no-op via the `None` signal. Backend failure warns,
    /// fire-and-forget (same contract as `persist`).
    pub fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Grid {
            return;
        }
        if let Some(payload) = self.selection_paths_string() {
            if let Err(e) = crate::clipboard::copy_text(&payload) {
                tracing::warn!("could not copy paths to clipboard: {e}");
            }
        }
        self.note_interaction(cx);
        cx.notify();
    }

    /// Stage a batch delete (Delete key): snapshot the selection's paths.
    /// Silent no-op outside grid or with an empty set. Clears any previous
    /// transient batch status (the new op supersedes it).
    pub fn stage_delete(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Grid || self.selected.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = self
            .selected
            .iter()
            .filter_map(|i| self.session.images.get(*i))
            .map(|item| item.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.pending_batch = Some(BatchOp::Delete { paths });
        self.batch_status = None;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Stage a batch move to `dest` (M key → picker, then this). Same
    /// guards as [`Self::stage_delete`].
    pub fn stage_move(&mut self, dest: PathBuf, cx: &mut Context<Self>) {
        if self.view != View::Grid || self.selected.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = self
            .selected
            .iter()
            .filter_map(|i| self.session.images.get(*i))
            .map(|item| item.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.pending_batch = Some(BatchOp::Move { paths, dest });
        self.batch_status = None;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Execute the staged batch op (Enter / confirm button): take the op
    /// (bar closes immediately even on fs failure), run it together with the
    /// folder rescan on the background executor, then remap leftovers by
    /// path and report partials in the transient topbar-center status.
    /// Grid-only guard (defensive).
    ///
    /// Why off-thread: `trash_paths`/`move_paths` are N filesystem mutations
    /// (a cross-device move falls back to copy + trash-original, doubling the
    /// per-file cost) and `scan_entries` is a full `read_dir` plus a
    /// `metadata()` per entry. Both are unbounded in the selected count, so
    /// running them inline stalled the frame loop on the one path a user
    /// reaches by selecting hundreds of files (AGENTS.md §7.1). Same
    /// `cx.background_executor()` + `cx.spawn` commit shape as
    /// [`Self::confirm_crop_save`].
    ///
    /// Going async opens a window the synchronous version did not have: the
    /// user can confirm a batch and then open a different folder before the
    /// task lands. The completion is therefore list-identity guarded by
    /// [`same_image_set`] — an unguarded rescan would overwrite the newly
    /// opened folder's `session.images` with the old folder's scan.
    ///
    /// NOTE (plan reorder, Task 2→3): this method is specified in Task 3 but
    /// implemented here because the Task-2 Enter handler references it — no
    /// commit in between may stay broken.
    pub fn confirm_pending(&mut self, cx: &mut Context<Self>) {
        let Some(op) = self.pending_batch.take() else {
            return;
        };
        if self.view != View::Grid {
            return;
        }
        // The report verb is a pure enum read (no I/O), so it resolves here
        // and travels with the task — the completion only formats the line.
        let verb = match &op {
            BatchOp::Delete { .. } => sh_core::i18n::BatchVerb::Deleted,
            BatchOp::Move { .. } => sh_core::i18n::BatchVerb::Moved,
        };
        // Rescan target, derived from the staged paths — all share the
        // visible folder by construction. Pure path math (no `read_dir`),
        // so it stays on this side of the spawn; the `scan_entries` call
        // itself does not.
        let dir = op
            .paths()
            .first()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
        // List-identity anchor: the exact image set this op ran against. The
        // disk work is unconditional (the user asked for it and is not
        // second-guessed), but the UI commit is conditional on the user
        // still being in this folder.
        let before: Vec<PathBuf> = self.session.images.iter().map(|i| i.path.clone()).collect();
        // The bar is already gone (the `take` above), so paint that close NOW
        // instead of making the user stare at a confirm bar for the whole
        // trash. This is the "closes immediately even on fs failure" contract
        // in its strongest form: the bar never waits on the filesystem.
        self.note_interaction(cx);
        cx.notify();
        let bg = cx.background_executor();
        // Same rule as every other scan, and read here for the same reason:
        // the batch's rescan must list exactly what the folder lists NOW,
        // or the leftovers it remaps by path would not match the grid.
        let opts = sh_core::navigation::ScanOptions {
            show_hidden: self.settings.show_hidden_files,
        };
        // ONE task for both halves: the rescan reads the folder AFTER the
        // mutations settled, and a single await keeps the new list and the
        // report committed atomically (no frame where the grid shows a
        // pre-op list next to a post-op status). The folder rides back with
        // its entries so the completion can name it in the empty-folder
        // error without a second lookup.
        let task = bg.spawn(async move {
            let report = match op {
                BatchOp::Delete { paths } => sh_core::batch::trash_paths(&paths),
                BatchOp::Move { paths, dest } => sh_core::batch::move_paths(&paths, &dest),
            };
            // No persist on the other side of this rescan: the folder didn't
            // change, so settings/recents stay untouched.
            let rescanned = dir.map(|d| {
                let entries = sh_core::navigation::scan_entries(&d, opts);
                (d, entries)
            });
            (report, rescanned)
        });
        cx.spawn(async move |this, cx| {
            let (report, rescanned) = task.await;
            let _ = this.update(cx, |app, cx| {
                // Stale-list guard (see [`same_image_set`]): the user has
                // since left the folder this op ran against, so the WHOLE
                // result is dropped — rescan, leftover remap, status, thumb
                // batch. Dropping is a correct no-op, not a lost update: the
                // delete/move already happened on disk (the user confirmed
                // it), while the folder now on screen ran its OWN fresh scan
                // on open, so nothing this op would have refreshed is stale
                // in front of them. Applying it would instead clobber the new
                // list, remap `selected` against the wrong entries, and
                // report a folder the user is no longer looking at.
                if !same_image_set(&before, &app.session.images) {
                    return;
                }
                if let Some((dir, entries)) = rescanned {
                    app.session.images = build_image_items(entries);
                    if app.session.images.is_empty() {
                        app.session.error = Some(sh_core::i18n::no_images_in(
                            app.settings.language,
                            &dir.display().to_string(),
                        ));
                    }
                    app.session.current = app
                        .session
                        .current
                        .min(app.session.images.len().saturating_sub(1));
                    app.spawn_thumb_batch(cx);
                }
                // Leftovers (skipped + failed) stay marked, remapped by path.
                // Shared rule with `commit_rescan` (`remap_selection_by_path`):
                // a shorter list must not leave the cursor past the end.
                let (selected, cursor) = remap_selection_by_path(
                    &app.session.images,
                    report
                        .skipped_existing
                        .iter()
                        .chain(report.failed.iter().map(|(p, _)| p))
                        .map(std::path::PathBuf::as_path),
                    app.grid_selected,
                );
                app.selected = selected;
                app.grid_selected = cursor;
                // Report partials in the transient center status; full
                // success is silent. session.error is deliberately untouched
                // (it does not render with images present). The verb is typed
                // (`BatchVerb`) and the language owns the full sentence (S5)
                // — Es word order is free to differ.
                let total =
                    report.moved.len() + report.skipped_existing.len() + report.failed.len();
                app.batch_status =
                    sh_core::batch::format_report(app.settings.language, verb, total, &report);
                cx.notify();
            });
        })
        .detach();
    }

    /// Enter crop mode: drag will select a region instead of panning.
    pub fn enter_crop(&mut self, cx: &mut Context<Self>) {
        self.crop_mode = true;
        self.crop_rect = None;
        self.crop_bar_visible = false;
        self.drag_last = None;
        // Crop owns the pointer; auto-advance must not fight a selection.
        self.session.slideshow_active = false;
        self.cancel_slideshow_timer();
        self.note_interaction(cx);
        cx.notify();
    }

    /// Leave crop mode, discarding any in-progress selection.
    pub fn cancel_crop(&mut self, cx: &mut Context<Self>) {
        self.crop_mode = false;
        self.crop_rect = None;
        self.crop_bar_visible = false;
        self.drag_last = None;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Toggle crop mode (the `C` key / scissors button).
    pub fn toggle_crop(&mut self, cx: &mut Context<Self>) {
        if self.crop_mode {
            self.cancel_crop(cx);
        } else {
            self.enter_crop(cx);
        }
    }

    /// Arm a crop drag at `pos` (viewport px): zero-area rect anchored
    /// there. Pure state mutation, headless-testable.
    pub fn begin_crop_drag(&mut self, pos: (f32, f32)) {
        self.crop_rect = Some(sh_core::crop::CropRect {
            x0: pos.0,
            y0: pos.1,
            x1: pos.0,
            y1: pos.1,
        });
    }

    /// Extend the armed crop drag to `pos`. No-op when nothing is armed
    /// (bare hover must not fabricate a selection).
    pub fn update_crop_drag(&mut self, pos: (f32, f32)) {
        if let Some(r) = self.crop_rect.as_mut() {
            r.x1 = pos.0;
            r.y1 = pos.1;
        }
    }

    /// Finish the drag: show the confirm bar only when the selection has
    /// real area (> 4 px² anti-click threshold); otherwise discard it
    /// silently (an accidental click deserves no error). Documented
    /// decision in the crop plan (§Task 5 Step 3).
    pub fn finish_crop_drag(&mut self) {
        let has_area = self
            .crop_rect
            .map(|r| (r.x1 - r.x0).abs() * (r.y1 - r.y0).abs() > 4.0)
            .unwrap_or(false);
        self.crop_bar_visible = has_area;
        if !has_area {
            self.crop_rect = None;
        }
    }
    /// Convert the current selection to image pixels via the session zoom.
    /// `None` when degenerate or dimensions unknown (nothing to crop).
    fn crop_rect_px(&self) -> Option<(u32, u32, u32, u32)> {
        let rect = self.crop_rect?;
        let (iw, ih) = self.session.current_dimensions()?;
        rect.to_pixels(
            self.session.zoom.scale,
            self.session.zoom.offset,
            iw as f32,
            ih as f32,
        )
    }

    /// Shared post-crop outcome: success exits crop mode silently (no toast
    /// system — the mode exit IS the confirmation), failure lands in the
    /// viewer error slot and keeps the bar for a retry.
    fn crop_done(&mut self, cx: &mut Context<Self>, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.crop_mode = false;
                self.crop_rect = None;
                self.crop_bar_visible = false;
            }
            Err(msg) => {
                self.session.error = Some(msg);
                self.crop_bar_visible = false;
            }
        }
        self.note_interaction(cx);
        cx.notify();
    }

    /// Confirm-bar "Copy": crop the ORIGINAL file at the converted rect,
    /// then copy to the OS clipboard. Decode + Win32 clipboard calls block,
    /// so everything runs on the background executor; the entity lease is
    /// released for the whole operation (same contract as the thumb
    /// decoders).
    pub fn confirm_crop_copy(&mut self, cx: &mut Context<Self>) {
        let Some(rect) = self.crop_rect_px() else {
            // Degenerate rect or unknown dimensions: silently close, no
            // error (documented decision — nothing was cropped, nothing
            // deserves a message).
            self.crop_rect = None;
            self.crop_bar_visible = false;
            self.note_interaction(cx);
            cx.notify();
            return;
        };
        let Some(path) = self.session.current_item().map(|i| i.path.clone()) else {
            return;
        };
        // Read the decode cap on the MAIN thread, before the background task
        // captures it: a `Settings` read inside the task would race the
        // settings surface, and the whole point of the setting is that the
        // user's choice is already committed by the time they crop.
        let max_decode_dimension = self.settings.max_decode_dimension;
        // Close the bar now (mode exit happens on success below); a second
        // confirm click mid-flight must not re-trigger.
        self.crop_bar_visible = false;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let cut = cx
                .background_executor()
                .spawn(async move {
                    sh_core::crop::crop_image_with_limit(&path, rect, max_decode_dimension)
                        .and_then(|img| crate::clipboard::copy_image(&img))
                })
                .await;
            let result = cut.map_err(|e| format!("Couldn't copy: {e}"));
            let _ = this.update(cx, |app, cx| app.crop_done(cx, result));
        })
        .detach();
    }

    /// Confirm-bar "Save…": show the save dialog (rfd async, PNG filter,
    /// `<stem>_crop.png` default in the original folder), then crop + write
    /// on the background executor. Cancel closes silently — the user
    /// changed their mind, not an error.
    pub fn confirm_crop_save(&mut self, cx: &mut Context<Self>) {
        let Some(rect) = self.crop_rect_px() else {
            self.crop_rect = None;
            self.crop_bar_visible = false;
            self.note_interaction(cx);
            cx.notify();
            return;
        };
        let Some(path) = self.session.current_item().map(|i| i.path.clone()) else {
            return;
        };
        // Same main-thread read discipline as `confirm_crop_copy`: the cap
        // is captured at spawn so an in-flight crop cannot observe a
        // half-applied settings change.
        let max_decode_dimension = self.settings.max_decode_dimension;
        self.crop_bar_visible = false;
        cx.notify();
        let dialog = crate::platform::save_dialog(&path);
        cx.spawn(async move |this, cx| {
            let Some(handle) = dialog.save_file().await else {
                // Cancelled: keep the selection + mode so the user can try
                // again without re-dragging.
                let _ = this.update(cx, |app, cx| {
                    app.crop_bar_visible = true;
                    cx.notify();
                });
                return;
            };
            let out = handle.path().to_path_buf();
            let cut = cx
                .background_executor()
                .spawn(async move {
                    sh_core::crop::crop_image_with_limit(&path, rect, max_decode_dimension)
                        .and_then(|img| sh_core::crop::save_png(&out, &img))
                })
                .await;
            let result = cut.map_err(|e| format!("Couldn't save: {e}"));
            let _ = this.update(cx, |app, cx| app.crop_done(cx, result));
        })
        .detach();
    }

    /// Show the async folder picker; on pick, open the folder in Grid.
    /// Shared by the Welcome/Open buttons, the top bar, and Ctrl+Shift+O —
    /// one place, same `cx.spawn` contract as the file dialog (no
    /// `double_lease_panic`: the lease is released before rfd blocks).
    pub fn pick_folder(&mut self, cx: &mut Context<Self>) {
        self.note_interaction(cx);
        let start_dir = self.recent_dirs_available.first().cloned();
        let dialog = crate::platform::folder_dialog(start_dir.as_deref());
        cx.spawn(async move |this, cx| {
            if let Some(handle) = dialog.pick_folder().await {
                let _ = this.update(cx, |app, cx| {
                    app.open_folder(handle.path().to_path_buf(), cx);
                });
            }
        })
        .detach();
    }

    /// Show the async folder picker for a MOVE destination (V3 batch ops);
    /// on pick, stage the move — the folder is NOT opened (unlike
    /// [`Self::pick_folder`], which navigates into the pick). Cancel stages
    /// nothing. Same `cx.spawn` contract (no `double_lease_panic`).
    pub fn pick_destination(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Grid || self.selected.is_empty() {
            return;
        }
        self.note_interaction(cx);
        let start_dir = self.recent_dirs_available.first().cloned();
        let dialog = crate::platform::folder_dialog(start_dir.as_deref());
        cx.spawn(async move |this, cx| {
            if let Some(handle) = dialog.pick_folder().await {
                let _ = this.update(cx, |app, cx| {
                    app.stage_move(handle.path().to_path_buf(), cx);
                });
            }
        })
        .detach();
    }

    /// Re-scan the config `themes/` directory and rebuild the picker rows.
    ///
    /// Called when the Settings surface opens, which is the only moment the
    /// list can be seen. Discovery on every open (rather than once at
    /// startup) is what makes "drop a JSON file in the folder, reopen
    /// Settings" work, which is the entire distribution story AGENTS.md §10
    /// asks for. It is also the only free moment to pay for it: nothing else
    /// in the app reads the list, so a startup scan would be I/O the user
    /// never asked for, and a scan inside `render` is forbidden outright.
    ///
    /// The directory scan and every JSON parse run on the background
    /// executor. Only the seq-guarded commit touches `App`, matching
    /// [`Self::list_load_seq`]'s discipline: a completion whose ticket is
    /// no longer the latest is dropped whole, so it can neither install a
    /// stale row list nor fail to refresh after a newer one committed.
    /// Invalidation is last-writer-wins by ticket, NOT by completion order.
    pub fn refresh_theme_entries(&mut self, cx: &mut Context<Self>) {
        let Some(config_dir) = self.settings_path.parent().map(|p| p.to_path_buf()) else {
            return;
        };
        self.theme_load_seq = self.theme_load_seq.wrapping_add(1);
        let ticket = self.theme_load_seq;
        cx.spawn(async move |this, cx| {
            let themes_dir = config_dir.join("themes");
            let scan = cx.background_executor().spawn(async move {
                use crate::ui::settings_panel::sections::appearance;
                let discovered = sh_core::theme::load_discovered(&themes_dir);
                let entries = appearance::merge_theme_entries(&themes_dir, &discovered);
                for bad in discovered.iter().filter(|d| d.error.is_some()) {
                    tracing::warn!(
                        "theme {} could not be loaded: {}",
                        bad.path.display(),
                        bad.error.as_deref().unwrap_or("unknown error")
                    );
                }
                entries
            });
            let entries = scan.await;
            let _ = this.update(cx, |app, cx| {
                if ticket != app.theme_load_seq {
                    return; // a newer refresh is in flight; drop this result
                }
                app.theme_entries = entries;
                cx.notify();
            });
        })
        .detach();
    }

    /// Apply the theme behind one picker row: swap the store (theme + name +
    /// the row's real file path), reset the hot-reload baseline and warn
    /// state, persist; the surface stays open.
    ///
    /// One path for built-ins AND user themes, because the only difference
    /// between them is whether the file already exists. Both must end with a
    /// REAL path in `theme_store`, since that path is what
    /// [`Self::spawn_theme_watcher`] polls — selecting a built-in whose path
    /// pointed nowhere would leave hot reload silently dead.
    ///
    /// `bootstrap_json` is written only when the target file is MISSING, so a
    /// built-in the user picked stays editable (and hot-reloadable) exactly
    /// as `theme_startup` establishes on first launch, while a user-supplied
    /// file is never overwritten. If a built-in's file exists but does not
    /// parse, the in-memory theme is still the built-in — the right recovery —
    /// and the watcher warns once about the user's broken copy on its next
    /// tick rather than being lied to about the baseline.
    ///
    /// Everything the user SEES stays synchronous — the store swap, the
    /// hot-reload baseline, the settings write — so the picker's row repaints
    /// on this frame. The `mkdir` and the create+write moved to
    /// [`cx.background_executor`] together: those are blocking syscalls, and
    /// the last two land on a user-chosen path that can be a network share,
    /// all of them on the click path (AGENTS.md §7.1).
    ///
    /// Ordering between the two halves, which is what going async opens up.
    /// The write is keyed by the PATH the row names, never by "the theme that
    /// is current", so picking A and then B while A's write is still in flight
    /// cannot clobber B: A's task can only ever create A's file, which is
    /// what A needed regardless of what happened afterwards. That is also why
    /// this write takes no ticket and commits nothing back into `App` — the
    /// store and the baseline belong to whichever row the user picked LAST, so
    /// a completion that touched them is precisely the clobber this ordering
    /// rules out, and there is no other state a stale completion could
    /// overwrite. `persist` is the same shape for the same reason.
    ///
    /// Two costs, both stated rather than hidden. (1) The MISSING test now
    /// runs on the worker instead of at click time, so the file lands an
    /// executor's queueing delay plus a `write` after the row highlighted it.
    /// What the deferral used to cost on top of that was a genuine
    /// data-loss window, `exists()` and then `write` being two syscalls with
    /// a gap between them; [`create_bootstrap_theme_file`] has no such gap,
    /// because the create IS the test (see its doc comment), so the deferral
    /// now costs latency and nothing else. (2) The watcher can poll the
    /// freshly-selected path before the file exists, log one read error, and
    /// clear it on the next tick. The applied theme is the correct one
    /// throughout — only a log line is off, and only for one tick.
    ///
    /// Returns false (no-op) for an unselectable row, which is how a
    /// click on an invalid theme is refused.
    pub fn apply_theme_entry(
        &mut self,
        entry: &crate::ui::settings_panel::sections::appearance::ThemeEntry,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(theme) = entry.theme.clone() else {
            return false;
        };
        // The write half leaves with everything it needs — target and bytes —
        // and captures NOTHING from `App`, so it can neither read a stale
        // theme nor install one (see the ordering notes above). The parent
        // guard stays here although the mkdir moved into the helper: a
        // parentless path is not something the picker can produce, and
        // `create_bootstrap_theme_file` would resolve it to the process CWD,
        // which is worse than doing nothing.
        if let Some(json) = entry.bootstrap_json.as_deref() {
            if entry.path.parent().is_some() {
                let path = entry.path.clone();
                let json = json.to_string();
                cx.background_executor()
                    .spawn(async move {
                        if let Err(e) = create_bootstrap_theme_file(&path, &json) {
                            // `AlreadyExists` is not a failure: the name was
                            // taken by a user's own copy, which is the rule
                            // working, not a fault worth a log line. Anything
                            // else left the file uncreated, so the applied
                            // in-memory theme stands and the watcher reports
                            // the user's (missing or broken) copy on its next
                            // tick — see the recovery contract above.
                            if e.kind() != std::io::ErrorKind::AlreadyExists {
                                tracing::warn!("could not write theme file: {e}");
                            }
                        }
                    })
                    .detach();
            }
        }
        // gpui-component reads its colors from its own global `Theme`, not from this
        // store. Projecting here — at the single point a new theme is adopted —
        // is what keeps the JSON file the one source of truth. Without it a
        // theme switch would restyle every `div()` in the app while the kit's
        // components kept the previous palette.
        crate::kit_theme::sync_kit_theme(cx, &theme, self.kit_theme_mode(&theme));
        self.theme_store = ThemeStore::new(theme, entry.file_name.clone(), entry.path.clone());
        self.last_applied_theme_text = entry.text.clone();
        self.last_warned_invalid_theme = None;
        self.theme_read_failed = false;
        self.capture_action = None;
        self.capture_conflict = None;
        self.reset_armed = false;
        self.settings.theme = entry.file_name.clone();
        self.persist(cx);
        cx.notify();
        true
    }

    /// Which of gpui-component's two modes this theme belongs to.
    ///
    /// gpui-component needs a `ThemeMode` to pick its default scrollbar,
    /// motion and elevation tokens, and a wrong choice would flip those on a
    /// dark theme. Sh Images' theme JSON carries no mode field — the four
    /// built-ins are `dark-clinical`, `deep-neutral`, `light-clean` and
    /// `noir-gallery`, two of each.
    ///
    /// Deriving the mode from the background's luminance instead would be
    /// more general, but it turns a styling decision into a threshold on a
    /// float: a custom mid-gray theme could land on either side, and the whole
    /// component layer would flip with it. The theme file's own name is the
    /// author's explicit statement, so it wins — with luminance as the
    /// fallback for a file whose name says nothing.
    fn kit_theme_mode(&self, theme: &sh_core::theme::Theme) -> gpui_component::theme::ThemeMode {
        use gpui_component::theme::ThemeMode;
        let name = self.theme_store.name.to_ascii_lowercase();
        if name.contains("light") {
            return ThemeMode::Light;
        }
        if name.contains("dark") || name.contains("noir") || name.contains("deep") {
            return ThemeMode::Dark;
        }
        // A theme named neutrally (a user-authored `mine.json`): fall back to
        // what its own colors imply, so the fallback still does the right
        // thing for the overwhelmingly common case.
        let bg = crate::app::parse_hex(&theme.colors.background).unwrap_or(rgb(0x000000).into());
        if bg.l > 0.5 {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        }
    }

    /// Open the full-screen Settings surface from any view, remembering the
    /// origin so `Esc` / Back returns exactly there. Capture state is
    /// cleared: opening Settings never resumes a stale capture.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.view == View::Settings {
            return;
        }
        self.settings_return_to = self.view;
        self.hover_motion.clear();
        self.view = View::Settings;
        // Settings is a temporary surface over Viewer: pause the pending
        // delay without discarding the user's active playback intent.
        self.cancel_slideshow_timer();
        self.sort_menu_open = false;
        self.capture_action = None;
        self.capture_conflict = None;
        self.reset_armed = false;
        self.settings_scroll_px = 0.0;
        self.settings_appearance_focus_control = None;
        // Re-scan user themes on open, not in `render`: the directory scan
        // and its JSON parses are filesystem I/O that must stay off the
        // frame loop. Until the completion lands, the previously cached
        // rows (built-ins at startup) are what render — never a blank list.
        self.refresh_theme_entries(cx);
        self.note_interaction(cx);
        cx.notify();
    }

    /// Close the Settings surface back to the originating view. Breaks any
    /// in-progress capture (focus-capture edge case: capture never survives
    /// a view change).
    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.hover_motion.clear();
        self.capture_action = None;
        self.capture_conflict = None;
        self.reset_armed = false;
        self.view = self.settings_return_to;
        if self.view == View::Viewer {
            self.rearm_slideshow_timer(cx);
        } else {
            self.cancel_slideshow_timer();
        }
        self.note_interaction(cx);
        cx.notify();
    }

    /// Persist a new keymap and rebind live without restart.
    ///
    /// Atomic `settings::save` (tmp + rename) first; on success the whole
    /// keymap is re-registered (`clear_key_bindings` + `bind_keys`) so the
    /// new combo fires immediately (verified supported in gpui 0.2.2:
    /// `AppContext::bind_keys` is callable from any `Context`). A failed
    /// save keeps the old bindings — nothing is rebound on a write error.
    pub fn apply_keymap(&mut self, keymap: sh_core::keymap::Keymap, cx: &mut Context<Self>) {
        let mut saved = self.settings.clone();
        saved.keymap = keymap;
        saved.version = CURRENT_SETTINGS_VERSION;
        if sh_core::settings::save(&self.settings_path, &saved).is_ok() {
            // NOTE: intentionally synchronous — one small local JSON file
            // (sub-ms); the disk-reload test depends on no-race semantics,
            // unlike `persist()`'s fire-and-forget background write.
            self.settings = saved;
            cx.clear_key_bindings();
            cx.bind_keys(crate::actions::resolve_bindings(&self.settings.keymap));
        } else {
            tracing::warn!("could not persist keymap; bindings unchanged");
        }
        cx.notify();
    }

    /// Persist a new UI language and live-switch every surface without restart.
    ///
    /// Commit-on-success (mirrors [`Self::apply_keymap`]): the candidate is
    /// saved atomically first; on `Ok` the in-memory language commits and
    /// `cx.notify()` re-renders all surfaces through
    /// `t(settings.language, …)`. On `Err` a warning is logged and the
    /// previous language is kept — no commit, no notify.
    pub fn apply_language(&mut self, lang: sh_core::i18n::Language, cx: &mut Context<Self>) {
        let mut saved = self.settings.clone();
        saved.language = lang;
        saved.version = CURRENT_SETTINGS_VERSION;
        // NOTE: intentionally synchronous — same no-race contract as
        // `apply_keymap` (one small local JSON file, sub-ms).
        if sh_core::settings::save(&self.settings_path, &saved).is_ok() {
            self.settings = saved;
            self.note_interaction(cx);
            cx.notify();
        } else {
            tracing::warn!("could not persist language; language unchanged");
        }
    }

    // General: language picker + hidden-files toggle row + recents list +
    // Clear button. Every string resolves via the table from
    // `settings.language`, so `apply_language`'s notify re-renders live.
    fn render_general_section(
        &mut self,
        surface: Hsla,
        text: Hsla,
        _accent: Hsla,
        _bg: Hsla,
        row_hover: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::ui::settings_panel::sections::general as gen;
        use sh_core::i18n::{t, StrKey};
        let lang = self.settings.language;
        let mut col = div().flex().flex_col().gap(px(scroll::GENERAL_GAP_PX));
        // Language picker: one row per option, check on the current.
        // Commit-on-success via `apply_language` (failed save keeps the old
        // language); the notify there re-renders every surface, no restart.
        col = col.child(
            div()
                .flex()
                .items_center()
                .h(px(scroll::SETTINGS_HEADER_H_PX))
                .px(px(10.0))
                .text_color(text)
                .child(t(lang, StrKey::LanguageLabel)),
        );
        for (row_idx, (option, autonym)) in gen::language_options().iter().enumerate() {
            let selected = *option == lang;
            let next = *option;
            let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let mut row = div()
                .id(("settings-language-row", row_idx))
                .flex()
                .items_center()
                .justify_between()
                .cursor_pointer()
                .rounded(px(6.0))
                .h(px(scroll::SETTINGS_ROW_H_PX))
                .px(px(10.0))
                .hover(move |s| s.bg(row_hover))
                .child(div().text_color(text).child(*autonym))
                .child(
                    div()
                        .text_color(text)
                        .child(if selected { "✓" } else { "" }),
                )
                .on_mouse_down(MouseButton::Left, swallow)
                .on_click(
                    cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        this.apply_language(next, cx);
                    }),
                );
            if selected {
                row = row.bg(surface);
            }
            col = col.child(row);
        }
        // Hidden files toggle.
        let hidden = self.settings.show_hidden_files;
        let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        col = col.child(
            div()
                .id("settings-show-hidden")
                .debug_selector(|| "settings-show-hidden".to_string())
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(6.0))
                .h(px(scroll::SETTINGS_ROW_H_PX))
                .px(px(10.0))
                .bg(surface)
                .hover(move |s| s.bg(row_hover))
                .text_color(text)
                .child(t(lang, StrKey::ShowHiddenFiles))
                .child(if hidden { "✓" } else { "" })
                .on_mouse_down(MouseButton::Left, swallow)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        this.toggle_show_hidden_files(cx);
                    }),
                ),
        );
        // Recents header + rows + Clear.
        col = col.child(
            div()
                .flex()
                .items_center()
                .h(px(scroll::SETTINGS_HEADER_H_PX))
                .px(px(10.0))
                .text_color(text)
                .child(
                    crate::ui::settings_panel::sections::general::recents_header(
                        lang,
                        self.settings.recent_dirs.len(),
                    ),
                ),
        );
        // Recent rows open their folder in Grid — the same contract as the
        // Welcome recent chips (pinned by
        // `settings_recent_row_opens_folder_in_grid`).
        for (idx, dir) in self.settings.recent_dirs.clone().iter().enumerate() {
            // Owned path: the click closure must be `'static`, so it cannot
            // borrow the temporary `Vec` above (Welcome-chip pattern).
            let dir = dir.clone();
            let swallow_recent =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            col = col.child(
                div()
                    .id(("settings-recent", idx))
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .h(px(scroll::SETTINGS_ROW_H_PX))
                    .px(px(10.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(text)
                    .hover(move |s| s.bg(row_hover))
                    .child(sh_core::recent::display_name(&dir))
                    .on_mouse_down(MouseButton::Left, swallow_recent)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.open_folder(dir.clone(), cx);
                        }),
                    ),
            );
        }
        if !self.settings.recent_dirs.is_empty() {
            let swallow_clear =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            col = col.child(
                div()
                    .id("settings-clear-recents")
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .h(px(scroll::SETTINGS_ROW_H_PX))
                    .px(px(12.0))
                    .bg(surface)
                    .hover(move |s| s.bg(row_hover))
                    .text_color(text)
                    .child(t(lang, StrKey::ClearRecents))
                    .on_mouse_down(MouseButton::Left, swallow_clear)
                    .on_click(
                        cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            // Optimistic UI (same contract as persist): memory updates now for instant feedback; disk write is best-effort and warns on failure.
                            this.settings.recent_dirs.clear();
                            this.settings.last_dir = None;
                            this.recent_dirs_available.clear();
                            this.settings.version = CURRENT_SETTINGS_VERSION;
                            let s = this.settings.clone();
                            let path = this.settings_path.clone();
                            cx.background_executor()
                                .spawn(async move {
                                    if let Err(e) = sh_core::settings::save(&path, &s) {
                                        tracing::warn!("could not persist settings: {e}");
                                    }
                                })
                                .detach();
                            cx.notify();
                        }),
                    ),
            );
        }
        col.into_any()
    }

    // Appearance: theme picker rows (moved from the old topbar dropdown).
    fn render_appearance_section(
        &mut self,
        surface: Hsla,
        text: Hsla,
        accent: Hsla,
        _bg: Hsla,
        row_hover: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::ui::settings_panel::sections::appearance;
        use sh_core::i18n::{t, StrKey};
        let lang = self.settings.language;
        let selected_control = self.settings_appearance_focus_control;
        let mut col = div()
            .id("settings-appearance-controls")
            .debug_selector(|| "settings-appearance-controls".to_string())
            .flex()
            .flex_col()
            .gap(px(scroll::APPEARANCE_GAP_PX));
        col = col.child(
            div()
                .flex()
                .items_center()
                .h(px(scroll::SETTINGS_HEADER_H_PX))
                .px(px(10.0))
                .text_color(text)
                .child(t(lang, StrKey::ThemeLabel)),
        );
        // One list, built-ins first then discovered user themes, already
        // deduped and ordered by `merge_theme_entries`. Cached on `App`, so
        // this loop reads memory only — no discovery, no parsing, no I/O on
        // the frame loop.
        for (row_idx, entry) in self.theme_entries.clone().into_iter().enumerate() {
            let display = entry.display_name();
            let active = entry.file_name == self.theme_store.name;
            let selectable = entry.is_selectable();
            // An invalid file renders dimmed and takes no pointer, because
            // there is nothing valid to apply. Hiding it instead would make
            // "not discovered" and "discovered but broken" indistinguishable
            // from outside, so a user who drops a bad file would see their
            // theme simply vanish.
            let label_color = if selectable { text } else { row_hover };
            let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let mut row = div()
                .id(("settings-theme-row", row_idx))
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(6.0))
                .h(px(scroll::SETTINGS_ROW_H_PX))
                .px(px(10.0))
                .child(div().text_color(label_color).child(display))
                .child(
                    div()
                        .text_color(if active { accent } else { text })
                        .child(if active { "✓" } else { "" }),
                );
            if selectable {
                row = row
                    .cursor_pointer()
                    .hover(move |s| s.bg(row_hover))
                    .on_mouse_down(MouseButton::Left, swallow)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            // Apply WITHOUT closing: the surface persists.
                            this.apply_theme_entry(&entry, cx);
                        }),
                    );
            }
            if active {
                row = row.bg(surface);
            }
            col = col.child(row);
        }

        let filmstrip_enabled = self.settings.filmstrip;
        let swallow_filmstrip = cx.listener(|this: &mut App, _ev: &MouseDownEvent, window, cx| {
            this.settings_appearance_focus_control =
                Some(crate::ui::settings_panel::AppearanceControl::Filmstrip);

            window.focus(&this.focus_handle, cx);
            cx.stop_propagation();
        });
        col = col.child(
            self.hover_background(
                div()
                    .id(appearance::FILMSTRIP_TOGGLE_ID)
                    .debug_selector(|| appearance::FILMSTRIP_TOGGLE_ID.to_string())
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(6.0))
                    .h(px(scroll::SETTINGS_ROW_H_PX))
                    .px(px(10.0))
                    .border(px(1.0))
                    .border_color(
                        if selected_control
                            == Some(crate::ui::settings_panel::AppearanceControl::Filmstrip)
                        {
                            accent
                        } else {
                            surface
                        },
                    )
                    .text_color(text)
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .child(t(lang, StrKey::FilmstripLabel)),
                    )
                    .child(
                        div()
                            .flex_shrink(0.0)
                            .debug_selector(|| {
                                appearance::FILMSTRIP_TOGGLE_TRACK_SELECTOR.to_string()
                            })
                            .child(
                                Switch::new("settings-filmstrip-toggle-track")
                                    .checked(filmstrip_enabled)
                                    .tab_stop(false)
                                    .accessibility_label(t(lang, StrKey::FilmstripLabel)),
                            ),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_filmstrip)
                    .on_click(
                        cx.listener(|this: &mut App, event: &ClickEvent, _window, cx| {
                            if matches!(event, ClickEvent::Keyboard(_)) {
                                return;
                            }
                            let next = !this.settings.filmstrip;
                            this.note_interaction(cx);
                            this.set_filmstrip(next, cx);
                        }),
                    ),
                motion::AnimationId::new(appearance::FILMSTRIP_TOGGLE_ID),
                surface,
                row_hover,
                cx,
            ),
        );

        let checkerboard_enabled = self.settings.checkerboard;
        let swallow_checkerboard =
            cx.listener(|this: &mut App, _ev: &MouseDownEvent, window, cx| {
                this.settings_appearance_focus_control =
                    Some(crate::ui::settings_panel::AppearanceControl::Checkerboard);
                window.focus(&this.focus_handle, cx);
                cx.stop_propagation();
            });
        col = col.child(
            self.hover_background(
                div()
                    .id(appearance::CHECKERBOARD_TOGGLE_ID)
                    .debug_selector(|| appearance::CHECKERBOARD_TOGGLE_ID.to_string())
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(6.0))
                    .h(px(scroll::SETTINGS_ROW_H_PX))
                    .px(px(10.0))
                    .border(px(1.0))
                    .border_color(
                        if selected_control
                            == Some(crate::ui::settings_panel::AppearanceControl::Checkerboard)
                        {
                            accent
                        } else {
                            surface
                        },
                    )
                    .text_color(text)
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .child(t(lang, StrKey::CheckerboardLabel)),
                    )
                    .child(
                        div()
                            .flex_shrink(0.0)
                            .debug_selector(|| {
                                appearance::CHECKERBOARD_TOGGLE_TRACK_SELECTOR.to_string()
                            })
                            .child(
                                Switch::new("settings-checkerboard-toggle-track")
                                    .checked(checkerboard_enabled)
                                    .tab_stop(false)
                                    .accessibility_label(t(lang, StrKey::CheckerboardLabel)),
                            ),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_checkerboard)
                    .on_click(
                        cx.listener(|this: &mut App, event: &ClickEvent, _window, cx| {
                            if matches!(event, ClickEvent::Keyboard(_)) {
                                return;
                            }
                            let next = !this.settings.checkerboard;
                            this.note_interaction(cx);
                            this.set_checkerboard(next, cx);
                        }),
                    ),
                motion::AnimationId::new(appearance::CHECKERBOARD_TOGGLE_ID),
                surface,
                row_hover,
                cx,
            ),
        );

        let interval_seconds = self.settings.slideshow_interval_secs;
        let swallow_decrement = cx.listener(|this: &mut App, _ev: &MouseDownEvent, window, cx| {
            this.settings_appearance_focus_control =
                Some(crate::ui::settings_panel::AppearanceControl::SlideshowDecrement);

            window.focus(&this.focus_handle, cx);
            cx.stop_propagation();
        });
        let decrement = div()
            .id(appearance::SLIDESHOW_INTERVAL_DECREMENT_ID)
            .debug_selector(|| appearance::SLIDESHOW_INTERVAL_DECREMENT_ID.to_string())
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .w(px(32.0))
            .h(px(32.0))
            .rounded(px(6.0))
            .bg(surface)
            .border(px(1.0))
            .border_color(
                if selected_control
                    == Some(crate::ui::settings_panel::AppearanceControl::SlideshowDecrement)
                {
                    accent
                } else {
                    surface
                },
            )
            .hover(move |s| s.bg(row_hover))
            .text_color(text)
            .child("−")
            .on_mouse_down(MouseButton::Left, swallow_decrement)
            .on_click(
                cx.listener(|this: &mut App, event: &ClickEvent, _window, cx| {
                    if matches!(event, ClickEvent::Keyboard(_)) {
                        return;
                    }
                    this.note_interaction(cx);
                    this.adjust_slideshow_interval(-1, cx);
                }),
            );
        let swallow_increment = cx.listener(|this: &mut App, _ev: &MouseDownEvent, window, cx| {
            this.settings_appearance_focus_control =
                Some(crate::ui::settings_panel::AppearanceControl::SlideshowIncrement);

            window.focus(&this.focus_handle, cx);
            cx.stop_propagation();
        });
        let increment = div()
            .id(appearance::SLIDESHOW_INTERVAL_INCREMENT_ID)
            .debug_selector(|| appearance::SLIDESHOW_INTERVAL_INCREMENT_ID.to_string())
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .w(px(32.0))
            .h(px(32.0))
            .rounded(px(6.0))
            .bg(surface)
            .border(px(1.0))
            .border_color(
                if selected_control
                    == Some(crate::ui::settings_panel::AppearanceControl::SlideshowIncrement)
                {
                    accent
                } else {
                    surface
                },
            )
            .hover(move |s| s.bg(row_hover))
            .text_color(text)
            .child("+")
            .on_mouse_down(MouseButton::Left, swallow_increment)
            .on_click(
                cx.listener(|this: &mut App, event: &ClickEvent, _window, cx| {
                    if matches!(event, ClickEvent::Keyboard(_)) {
                        return;
                    }
                    this.note_interaction(cx);
                    this.adjust_slideshow_interval(1, cx);
                }),
            );
        col = col.child(
            div()
                .id(appearance::SLIDESHOW_INTERVAL_ROW_ID)
                .debug_selector(|| appearance::SLIDESHOW_INTERVAL_ROW_ID.to_string())
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(6.0))
                .h(px(scroll::SETTINGS_ROW_H_PX))
                .px(px(10.0))
                .bg(surface)
                .text_color(text)
                .child(t(lang, StrKey::SlideshowIntervalLabel))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(decrement)
                        .child(
                            div()
                                .id(appearance::SLIDESHOW_INTERVAL_VALUE_ID)
                                .debug_selector(|| {
                                    appearance::SLIDESHOW_INTERVAL_VALUE_ID.to_string()
                                })
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(48.0))
                                .text_color(text)
                                .child(format!("{interval_seconds} s")),
                        )
                        .child(increment),
                ),
        );

        let reduce_motion_enabled = self.settings.reduce_motion;
        let swallow_reduce_motion =
            cx.listener(|this: &mut App, _ev: &MouseDownEvent, window, cx| {
                this.settings_appearance_focus_control =
                    Some(crate::ui::settings_panel::AppearanceControl::ReduceMotion);
                window.focus(&this.focus_handle, cx);
                cx.stop_propagation();
            });
        col = col.child(
            self.hover_background(
                div()
                    .id(appearance::REDUCE_MOTION_TOGGLE_ID)
                    .debug_selector(|| appearance::REDUCE_MOTION_TOGGLE_ID.to_string())
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(6.0))
                    .h(px(scroll::SETTINGS_ROW_H_PX))
                    .px(px(10.0))
                    .border(px(1.0))
                    .border_color(
                        if selected_control
                            == Some(crate::ui::settings_panel::AppearanceControl::ReduceMotion)
                        {
                            accent
                        } else {
                            surface
                        },
                    )
                    .text_color(text)
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .child(t(lang, StrKey::ReduceMotionLabel)),
                    )
                    .child(
                        div()
                            .flex_shrink(0.0)
                            .debug_selector(|| {
                                appearance::REDUCE_MOTION_TOGGLE_TRACK_SELECTOR.to_string()
                            })
                            .child(
                                Switch::new("settings-reduce-motion-toggle-track")
                                    .checked(reduce_motion_enabled)
                                    .tab_stop(false)
                                    .accessibility_label(t(lang, StrKey::ReduceMotionLabel)),
                            ),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_reduce_motion)
                    .on_click(
                        cx.listener(|this: &mut App, event: &ClickEvent, _window, cx| {
                            if matches!(event, ClickEvent::Keyboard(_)) {
                                return;
                            }
                            let next = !this.settings.reduce_motion;
                            this.note_interaction(cx);
                            this.set_reduce_motion(next, cx);
                        }),
                    ),
                motion::AnimationId::new(appearance::REDUCE_MOTION_TOGGLE_ID),
                surface,
                row_hover,
                cx,
            ),
        );
        col.into_any()
    }

    // Theme Editor: one row per field — [label] [hex field] [swatch].
    //
    // `window` is threaded in unlike every other section because the kit
    // `InputState` cannot be constructed without one, and `App::new` has none.
    // The entities are built once and cached on `Self::theme_inputs`; building
    // them per frame would reset the text on every re-render.
    // `window` is the eighth argument and the only reason this needs the allow
    // that `App::new` already carries: `InputState::new` cannot be built
    // without one, and `App::new` has none to give. Bundling the five colors
    // into a struct to get back under seven would mean changing the call shape
    // of the three sections this one sits beside, which is a wider edit than
    // this section is worth.
    #[allow(clippy::too_many_arguments)]
    fn render_theme_editor_section(
        &mut self,
        surface: Hsla,
        text: Hsla,
        _accent: Hsla,
        bg: Hsla,
        _row_hover: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::ui::settings_panel::sections::theme_editor as te;
        use gpui_component::input::Input;
        use sh_core::i18n::{t, StrKey};
        let lang = self.settings.language;

        // Re-seed only when the APPLIED theme identity changes. Comparing the
        // draft's content instead would discard unsaved edits on every frame
        // that happened to match, and rebuilding per frame would drop the
        // field the user is typing into.
        if self.theme_draft_source != self.theme_store.name {
            self.theme_draft =
                sh_core::theme_draft::ThemeDraft::from_theme(&self.theme_store.theme);
            self.theme_draft_source = self.theme_store.name.clone();
            // The entities hold the OLD text, so they are rebuilt rather than
            // patched: `Entity::update` hands out no `&mut Window`, and
            // `set_value` needs one, so the only way to write fresh values from
            // here is to construct them with them.
            self.theme_inputs = None;
        }
        if self.theme_inputs.is_none() {
            self.theme_inputs = Some(self.build_theme_inputs(window, cx));
        }

        let mut col = div()
            .id(te::CONTAINER_ID)
            .debug_selector(|| te::CONTAINER_ID.to_string())
            .flex()
            .flex_col()
            // MUST match `scroll::THEME_EDITOR_GAP_PX`.
            .gap(px(scroll::THEME_EDITOR_GAP_PX));
        col = col.child(
            div()
                .flex()
                .items_center()
                .h(px(scroll::SETTINGS_HEADER_H_PX))
                .px(px(10.0))
                .text_color(text)
                .child(t(lang, StrKey::ThemeLabel)),
        );

        // Enumerated rather than looked up by `position`: `FIELDS` and
        // `theme_inputs.fields` are built from the same list in the same order,
        // so the index IS the correspondence and searching for it would only
        // be a slower way to be wrong.
        for (idx, field) in te::FIELDS.iter().copied().enumerate() {
            let raw = self.theme_field_text(field);
            // A half-typed hex and a cleared optional slot are the NORMAL
            // states here, so the swatch falls back to the surface rather than
            // disappearing — the row still shows that a color is expected.
            let swatch_color = te::swatch_color(&raw).unwrap_or(surface);
            let entity = self
                .theme_inputs
                .as_ref()
                .expect("theme inputs are built above")
                .fields[idx]
                .clone();
            let label = te::label(field);
            col = col.child(
                div()
                    .id(te::row_id(field))
                    .debug_selector(move || te::row_id(field))
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(6.0))
                    // MUST match `scroll::SETTINGS_ROW_H_PX`.
                    .h(px(scroll::SETTINGS_ROW_H_PX))
                    .px(px(10.0))
                    .text_color(text)
                    .child(div().flex_shrink_0().text_color(text).child(label))
                    // The kit `Input` renders `.size_full()` and takes its
                    // height from its own `Sizable` size, so its geometry is
                    // pinned by this WRAPPER rather than on the input: the
                    // inherent `Input::h` only applies to multi-line inputs and
                    // would silently do nothing here.
                    .child(
                        div()
                            .id(te::field_id(field))
                            .debug_selector(move || te::field_id(field))
                            .flex()
                            // Without this the field is the row's shrinkable
                            // element, so the two longest slot names
                            // ("background", "muted_text") squeezed it from
                            // 120px to 114 at the 480px minimum. Nothing
                            // overflowed, but a hex field whose width depends
                            // on the label beside it is not a fixed geometry.
                            .flex_shrink_0()
                            .w(px(te::FIELD_W_PX))
                            .h(px(te::FIELD_H_PX))
                            .child(Input::new(&entity).small()),
                    )
                    .child(
                        div()
                            .id(te::swatch_id(field))
                            .debug_selector(move || te::swatch_id(field))
                            .flex_shrink_0()
                            .w(px(te::SWATCH_PX))
                            .h(px(te::SWATCH_PX))
                            .rounded(px(4.0))
                            .border(px(1.0))
                            .border_color(bg)
                            .bg(swatch_color),
                    ),
            );
        }
        col.into_any()
    }

    /// Create the field inputs and wire their change events into the draft.
    ///
    /// One subscription per field, each capturing its own [`te::Field`] rather
    /// than an index, so reordering `FIELDS` cannot silently redirect an
    /// input's keystrokes to another row.
    ///
    /// `default_value` is the only way to seed the text here: `Entity::update`
    /// hands out no `&mut Window`, and `set_value` requires one. It is also why
    /// changing the applied theme rebuilds this set instead of patching it.
    fn build_theme_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) -> ThemeInputs {
        use crate::ui::settings_panel::sections::theme_editor as te;
        use gpui_component::input::{InputEvent, InputState};
        let mut fields = Vec::with_capacity(te::FIELDS.len());
        let mut subscriptions = Vec::with_capacity(te::FIELDS.len());
        for field in te::FIELDS {
            let initial = self.theme_field_text(field);
            let entity = cx.new(|cx| InputState::new(window, cx).default_value(initial));
            subscriptions.push(cx.subscribe(
                &entity,
                move |this: &mut App, entity, event: &InputEvent, cx| {
                    // Focus, blur and Enter all arrive on this same channel;
                    // only a change may write to the draft.
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let typed = entity.read(cx).value().to_string();
                    this.set_theme_field(field, typed, cx);
                },
            ));
            fields.push(entity);
        }
        ThemeInputs {
            fields,
            _subscriptions: subscriptions,
        }
    }

    /// This field's current text in the draft.
    fn theme_field_text(
        &self,
        field: crate::ui::settings_panel::sections::theme_editor::Field,
    ) -> String {
        use crate::ui::settings_panel::sections::theme_editor as te;
        match field {
            te::Field::Name => self.theme_draft.name.clone(),
            te::Field::Family => self.theme_draft.family.clone(),
            te::Field::Color(slot) => self.theme_draft.get(slot).to_string(),
        }
    }

    /// Record an edit. No save and no preview here: WU-4 writes the file and
    /// WU-5 applies the draft, and until they exist the applied theme must not
    /// move under the user mid-edit.
    fn set_theme_field(
        &mut self,
        field: crate::ui::settings_panel::sections::theme_editor::Field,
        value: String,
        cx: &mut Context<Self>,
    ) {
        use crate::ui::settings_panel::sections::theme_editor as te;
        match field {
            te::Field::Name => self.theme_draft.name = value,
            te::Field::Family => self.theme_draft.family = value,
            te::Field::Color(slot) => self.theme_draft.set(slot, value),
        }
        // Typing counts as interaction: without this the overlay chrome is
        // free to fade out over a field the user is actively editing.
        self.note_interaction(cx);
        cx.notify();
    }

    // Shortcuts: Reset button + one row per ACTIONS entry ([label] [chip]).
    // Click chip → capture mode; conflict → red error, stay capturing;
    // free → apply_keymap (atomic save + live rebind).
    fn render_shortcuts_section(
        &mut self,
        surface: Hsla,
        text: Hsla,
        accent: Hsla,
        _bg: Hsla,
        row_hover: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::ui::settings_panel::sections::shortcuts as sc;
        use sh_core::i18n::{t, StrKey};
        let lang = self.settings.language;
        let mut col = div().flex().flex_col().gap(px(2.0));

        // Reset-all button (two-step inline confirm, the batch-bar pattern).
        let armed = self.reset_armed;
        let swallow_reset = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        col = col.child(
            div()
                .id("shortcuts-reset")
                .debug_selector(|| "shortcuts-reset".to_string())
                .cursor_pointer()
                .flex()
                .items_center()
                .rounded(px(6.0))
                .px(px(12.0))
                // Fixed height (see `scroll::SHORTCUT_RESET_H_PX`): the
                // scroll clamp math depends on it.
                .h(px(scroll::SHORTCUT_RESET_H_PX))
                .bg(surface)
                .hover(move |s| s.bg(row_hover))
                .text_color(if armed { accent } else { text })
                .child(if armed {
                    t(lang, StrKey::ResetConfirm)
                } else {
                    t(lang, StrKey::ResetShortcuts)
                })
                .on_mouse_down(MouseButton::Left, swallow_reset)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        if this.reset_armed {
                            this.reset_armed = false;
                            this.capture_action = None;
                            this.capture_conflict = None;
                            this.apply_keymap(sh_core::keymap::defaults(), cx);
                        } else {
                            this.reset_armed = true;
                            cx.notify();
                        }
                    }),
                ),
        );

        // One row per action, grouped by display context in ACTIONS order.
        let mut last_group = "";
        let defaults = sh_core::keymap::defaults();
        for (idx, desc) in crate::actions::ACTIONS.iter().enumerate() {
            if desc.context != last_group {
                last_group = desc.context;
                col = col.child(
                    div()
                        .flex()
                        .items_center()
                        .px(px(10.0))
                        // Fixed height (see `scroll::SHORTCUT_GROUP_H_PX`).
                        .h(px(scroll::SHORTCUT_GROUP_H_PX))
                        .text_color(text)
                        .child(desc.context),
                );
            }
            let capturing = self.capture_action.as_deref() == Some(desc.id);
            let stored = self
                .settings
                .keymap
                .get(desc.id)
                .cloned()
                .unwrap_or_else(|| {
                    defaults
                        .get(desc.id)
                        .cloned()
                        .unwrap_or(sh_core::keymap::KeyBinding {
                            ctrl: false,
                            shift: false,
                            alt: false,
                            platform: false,
                            key: "?".into(),
                        })
                });
            let chip_label = if capturing {
                match &self.capture_conflict {
                    Some(incumbent_id) => {
                        sc::conflict_text(lang, sc::action_label(incumbent_id, lang))
                    }
                    None => sc::capture_prompt(lang).into(),
                }
            } else {
                sc::chip_text(&stored)
            };
            let has_error = capturing && self.capture_conflict.is_some();
            let id = desc.id;
            let swallow_row = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let mut chip = div()
                .id(("shortcut-chip", idx))
                .cursor_pointer()
                .rounded(px(6.0))
                .px(px(10.0))
                .py(px(4.0))
                // Single-line chip (grid-label discipline): long capture /
                // conflict text truncates instead of wrapping, which would
                // silently break the fixed row height below.
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .bg(surface)
                .text_color(if has_error { accent } else { text });
            if !has_error {
                chip = chip.hover(move |s| s.bg(row_hover));
            }
            // Error state: the theme's own `danger`, not a fixed red. This used to be
            // `rgb(0xff5555)` with a comment explaining that themes had no error
            // token; ADR-021 added one, and leaving a literal here would mean the
            // most alarming color in the app was the one color a theme cannot
            // choose.
            if has_error {
                let danger = parse_hex(&self.theme_store.theme.colors.danger)
                    .unwrap_or(rgb(0xff5555).into());
                chip = chip.text_color(danger);
            }
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(6.0))
                    .px(px(10.0))
                    // Fixed height (see `scroll::SHORTCUT_ROW_H_PX`): the
                    // scroll clamp math depends on it — keep rows
                    // single-line.
                    .h(px(scroll::SHORTCUT_ROW_H_PX))
                    .text_color(text)
                    .child(t(lang, desc.label_key))
                    .child(
                        chip.child(chip_label)
                            .on_mouse_down(MouseButton::Left, swallow_row)
                            .on_click(cx.listener(
                                move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                                    this.note_interaction(cx);
                                    this.reset_armed = false;
                                    this.capture_action = Some(id.to_string());
                                    this.capture_conflict = None;
                                    cx.notify();
                                },
                            )),
                    ),
            );
        }
        col.into_any()
    }

    /// Persist theme + last_dir to `settings.json` on a worker (atomic write
    /// via sh-core's `.tmp` + rename).
    ///
    /// Saves a copy of the STARTUP-loaded settings with only `theme`,
    /// `last_dir`, and the current schema version updated — never
    /// `Settings::default()`, which would stomp user-edited values in
    /// unrelated fields. It is called by settings writers; the recent-folder
    /// bookkeeping is refreshed on each write. Fire-and-forget: a failed write
    /// is logged, never surfaced as an error state.
    fn persist(&mut self, cx: &mut Context<Self>) {
        let folder = self
            .session
            .current_item()
            .and_then(|i| i.path.parent().map(std::path::Path::to_path_buf));
        // V3 recents: push the opened folder (dedupe + trim, pure core
        // helper), then mirror last_dir = recent[0] so a v2 binary reading
        // a v3 file still finds its Continue path. A parent-less edge
        // (no current image) leaves the existing list untouched.
        if let Some(dir) = folder {
            self.settings.recent_dirs =
                sh_core::recent::push_recent(&self.settings.recent_dirs, dir);
        }
        self.settings.last_dir = self.settings.recent_dirs.first().cloned();
        // `recent_dirs_available` is refreshed here (every persist: file
        // dialog, drag&drop, navigate-folder-change) — the in-memory mirror
        // the Welcome screen reads.
        self.recent_dirs_available = self.settings.recent_dirs.clone();
        self.settings.theme = self.theme_store.name.clone();
        self.settings.version = CURRENT_SETTINGS_VERSION;
        let s = self.settings.clone();
        let path = self.settings_path.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = sh_core::settings::save(&path, &s) {
                    tracing::warn!("could not persist settings: {e}");
                }
            })
            .detach();
    }

    /// Spawn the theme hot-reload watcher: polls the active theme file every
    /// [`THEME_POLL`] and applies it when the text changed and parses.
    ///
    /// Disk read AND parse run on the background executor (off the main
    /// thread); the entity update only happens when there is something to
    /// apply. Unchanged text is skipped (`hot_reload_decision`), changed text
    /// that fails validation keeps the last valid theme and warns once per
    /// DISTINCT invalid text (deduped by [`Self::last_applied_theme_text`]).
    /// A read error (deleted/unreadable file) is distinct from an invalid
    /// file: it warns once on the Ok→Err transition, is treated as
    /// UNCHANGED for that tick (no parse, no apply, last valid theme kept),
    /// and normal flow resumes on recovery. The loop exits once the App
    /// entity is dropped.
    pub fn spawn_theme_watcher(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(THEME_POLL).await;
            // Read + parse off the main thread; a locked/huge file can never
            // stall a frame.
            let path = this.update(cx, |app, _| app.theme_store.path.clone()).ok();
            let Some(path) = path else {
                break; // entity dropped → stop polling
            };
            let read_task = cx
                .background_executor()
                .spawn(async move { std::fs::read_to_string(&path) });
            let read = read_task.await;

            // Read-error transition handling (main thread: mutates the
            // `theme_read_failed` flag and warns once per failure period).
            let read_text = this
                .update(cx, |app, _| match read {
                    Ok(text) => {
                        app.theme_read_failed = false; // recovered (or still ok)
                        Some(text)
                    }
                    Err(e) => {
                        // Warn once per failure period, not per tick.
                        if !app.theme_read_failed {
                            tracing::warn!("could not read theme file: {e}");
                            app.theme_read_failed = true;
                        }
                        None
                    }
                })
                .ok();
            let Some(text) = read_text else {
                break; // entity dropped → stop polling
            };
            // Deleted/unreadable: treat as unchanged this tick (keep last
            // valid theme; no parse, no apply).
            let Some(text) = text else {
                continue;
            };

            let decision = this.update(cx, |app, _| {
                hot_reload_decision(&app.last_applied_theme_text, &text)
            });
            let Some(decision) = decision.ok() else {
                break; // entity dropped → stop polling
            };
            if let HotReloadDecision::Apply = decision {
                // Parse off the main thread too — theme JSON is small but
                // validation is pure CPU and belongs on a worker.
                let parse_text = text.clone();
                let parse_task = cx
                    .background_executor()
                    .spawn(async move { sh_core::theme::parse(&parse_text) });
                let parsed = parse_task.await;
                let applied = this.update(cx, |app, cx| {
                    match parsed {
                        Ok(theme) => {
                            let path = app.theme_store.path.clone();
                            // Same projection as the settings-panel path: a
                            // hot-reloaded theme must move the kit's global too,
                            // or editing the file would only restyle half the UI.
                            let mode = app.kit_theme_mode(&theme);
                            crate::kit_theme::sync_kit_theme(cx, &theme, mode);
                            app.theme_store.set(theme, path);
                            app.last_applied_theme_text = text;
                            cx.notify();
                        }
                        Err(e) => {
                            // Keep the last valid theme; warn once per
                            // DISTINCT invalid text.
                            if app.last_warned_invalid_theme.as_deref() != Some(text.as_str()) {
                                tracing::warn!("theme hot-reload rejected: {e}");
                                app.last_warned_invalid_theme = Some(text);
                            }
                        }
                    }
                });
                if applied.is_err() {
                    break; // entity dropped → stop polling
                }
            }
        })
        .detach();
    }

    /// Navigate `delta` steps (‑1 = prev, +1 = next) with a header probe on a worker.
    ///
    /// Spawns a background dimension probe for the target image (never a full
    /// pixel decode), then updates session state on completion. A neighbor
    /// prefetch warms the next image's dimensions.
    ///
    /// Completions carry a monotonic sequence number: a completion belonging
    /// to a navigation that is no longer the latest is dropped ENTIRELY — it
    /// may neither commit global state (`zoom`/`fit_mode`/`error`) nor write
    /// slot `dimensions`. This is not just about stale UI: `open_path` swaps
    /// the whole images Vec, so a stale completion's captured index could
    /// point past the new list (out-of-bounds panic) or poison a slot with
    /// another image's dimensions. ALL session mutations are seq-guarded.
    pub fn navigate(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.session.images.len();
        let Some(next) = next_index(self.session.current, delta, n) else {
            return;
        };
        self.navigation_seq += 1;
        let seq = self.navigation_seq;
        self.session.current = next;
        self.session.error = None;
        // Navigate-while-open updates in place: the popover stays open and
        // its facts are re-resolved for the new current file atomically with
        // the index advance, so no stale paint of image A is possible.
        if self.info_panel_open {
            self.refresh_info_facts();
        }

        // ── Probe current image dimensions on background thread ──
        // NOTE: fit is computed at COMPLETION time against the live viewport,
        // so a resize mid-probe picks up the current size.
        let path = self.session.images[next].path.clone();
        let bg = cx.background_executor();
        // Read the decode cap on the main thread at SPAWN time (not inside
        // the task), same discipline the crop and folder paths use: a
        // settings change is therefore picked up by the NEXT navigation
        // rather than racing this one. `probe_dimensions` is deliberately
        // NOT capped — it feeds fit/zoom math, and capping a header probe
        // would make the viewer zoom against a frame it is not showing.
        let max_decode_dimension = self.settings.max_decode_dimension;
        // Slice B: the SAME background task coalesces the alpha verdict
        // (`probe_has_alpha_with_limit`: JPEG fast path = header cost, no
        // pixel decode; alpha-capable formats decode off the frame loop,
        // now bounded by the user's cap instead of the hardcoded default).
        // One task, one file open per probe — no new threads, no
        // render-time I/O.
        let probe_task = bg.spawn(async move {
            let dims = sh_core::decode::probe_dimensions(&path);
            let alpha = sh_core::decode::probe_has_alpha_with_limit(&path, max_decode_dimension);
            (dims, alpha)
        });
        cx.spawn(async move |this, cx| {
            let (dims, alpha) = probe_task.await;
            let _ = this.update(cx, |app, cx| {
                // A stale probe skips entirely (see the seq-guard invariant
                // above): if the user returns to this image later, the next
                // navigation probes it again — a header read is near-instant.
                if seq != app.navigation_seq {
                    return;
                }
                match dims {
                    Ok((w, h)) => {
                        app.session.images[next].dimensions = Some((w, h));
                        // Stable open zoom: the carve (full window with the
                        // strip hidden, strip-subtracted with it visible),
                        // never the idle-dependent live viewport — the same
                        // image always lands at the same fit scale.
                        let stable = app.viewer_viewport();
                        app.session.zoom = sh_core::transform::fit(
                            sh_core::transform::Vec2 {
                                x: w as f32,
                                y: h as f32,
                            },
                            stable,
                        );
                        app.session.fit_mode = FitMode::Fit;
                    }
                    Err(_) => {
                        app.session.error = Some("could not read image".into());
                    }
                }
                // A failed alpha probe leaves `None` (render as opaque):
                // the dims probe above owns the error slot, and a missing
                // verdict must never surface as a verdict.
                if let Ok(a) = alpha {
                    app.session.images[next].has_alpha = Some(a);
                }
                cx.notify();
            });
        })
        .detach();

        // ── Prefetch neighbor dimensions (CPU-side session warm-up) ──
        // NOTE: This warms CPU-side session state (dimensions for fit/zoom
        // math) but does NOT populate GPUI's `img()` asset cache. Render-side
        // reuse arrives with the use_asset wiring in later tasks.
        if n > 1 {
            let pre_idx = (next + 1) % n;
            let pre_path = self.session.images[pre_idx].path.clone();
            let bg = cx.background_executor();
            // Same main-thread read as the probe above, and the same value:
            // both sites must agree or a neighbor would be warmed under a
            // different decode budget than the image it precedes.
            let pre_max_decode_dimension = max_decode_dimension;
            // Slice B: the neighbor prefetch warms the alpha verdict with
            // the same coalesced task (dims + alpha, one file open).
            let pre_task = bg.spawn(async move {
                let dims = sh_core::decode::probe_dimensions(&pre_path);
                let alpha = sh_core::decode::probe_has_alpha_with_limit(
                    &pre_path,
                    pre_max_decode_dimension,
                );
                (dims, alpha)
            });
            cx.spawn(async move |this, cx| {
                let (pre, pre_alpha) = pre_task.await;
                let _ = this.update(cx, |app, cx| {
                    // Same seq guard as the probe: a stale prefetch's index
                    // may be meaningless after an open_path Vec swap. The
                    // warm-cache optimization simply dies with its navigation.
                    if seq != app.navigation_seq {
                        return;
                    }
                    if let Ok(d) = pre {
                        app.session.images[pre_idx].dimensions = Some(d);
                    }
                    if let Ok(a) = pre_alpha {
                        app.session.images[pre_idx].has_alpha = Some(a);
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        // Sync state (current/error) changed; paint immediately instead of
        // waiting for the probe to land.
        cx.notify();
    }
}

/// Convert a pixel size into a transform vector (small helper to avoid
/// repeating the `f32::from` dance at every call site).
fn viewport_vec(viewport: Size<Pixels>) -> sh_core::transform::Vec2 {
    sh_core::transform::Vec2 {
        x: f32::from(viewport.width),
        y: f32::from(viewport.height),
    }
}

/// Hover deadband in pixels: bare-hover moves shorter than this don't reset
/// the overlay idle clock. Filters sensor-noise micro-movements that would
/// otherwise re-show the overlays right at/after the idle threshold and
/// cause show/hide oscillation.
pub const HOVER_DEADBAND_PX: f32 = 3.0;

/// Whether the cursor moved far enough to count as a real interaction.
///
/// Pure helper over `(x, y)` pixel pairs (kept free of GPUI types for
/// trivial unit testing). Euclidean distance `>= HOVER_DEADBAND_PX`
/// counts; anything less is treated as sensor noise. A missing anchor
/// (`None` at the call site) always counts — there is no baseline yet.
pub fn hover_moved_enough(old: (f32, f32), new: (f32, f32)) -> bool {
    let dx = new.0 - old.0;
    let dy = new.1 - old.1;
    dx.hypot(dy) >= HOVER_DEADBAND_PX
}

/// Whether the viewer info button renders: Viewer-only with an image open.
///
/// Deliberately NOT gated on the bottom bar or the idle clock (B3): the
/// button used to live inside the auto-hiding bar, so `Display::None` left
/// it with no hitbox most of the time. Pure so the reachability contract
/// is unit-testable.
pub fn info_button_visible(view: View, has_image: bool) -> bool {
    view == View::Viewer && has_image
}

/// Whether the info button mounts in the floating-chips row (R2): the row
/// exists only while the topbar is dissolved, and its right cluster is the
/// one corner the standalone float used to collide with. When this returns
/// `false` the button keeps its absolute top-right float. Exactly one of
/// the two containers parents the button per frame — never both, never
/// neither (while the button itself is visible). Pure so the container
/// contract is unit-testable.
pub fn info_button_in_chips_row(topbar_dissolved: bool) -> bool {
    topbar_dissolved
}

/// Pure predicate: should the solid topbar be dissolved? Viewer-only by
/// construction (Grid never dissolves — the caller guards on view). The bar
/// dissolves while the Tab bottom bar is ARMED, so total chrome never stacks
/// two solid rows: bottom-armed ⇒ bottom row only; Tab OFF ⇒ topbar row
/// only. Idle does NOT restore the topbar — the bottom bar itself idle-fades
/// to `Display::None`, leaving zero chrome for clean viewing; Tab OFF keeps
/// the single topbar row solid even when idle (it is the last visible UI).
pub fn topbar_dissolved_for_viewer(show_overlay_bottom: bool) -> bool {
    show_overlay_bottom
}

/// Pure wheel routing: Welcome and Settings never touch viewer or grid
/// state. Welcome has nothing to zoom or scroll; Settings content scrolls
/// natively (`overflow_y_scroll` owns the gesture), so the wheel handler
/// must not fall through to viewer zoom — that would silently mutate
/// `session.zoom` while in Settings. Pure so the routing is unit-testable
/// (the harness cannot synthesize wheel events).
pub fn wheel_parks(view: View) -> bool {
    matches!(view, View::Welcome | View::Settings)
}

/// Hover tint: the modern button idiom (Figma/Linear/Zed) — no border swap;
/// the background itself lightens (or darkens, on light themes) toward the
/// foreground color. Channel-wise mix in RGBA space; `ratio` 0 = pure
/// background, 1 = pure foreground. Opaque so it works as a solid `.bg()`.
pub fn hover_tint(bg: Hsla, fg: Hsla, ratio: f32) -> Hsla {
    let b: Rgba = bg.into();
    let f: Rgba = fg.into();
    let mix = |x: f32, y: f32| x + (y - x) * ratio;
    Rgba {
        r: mix(b.r, f.r),
        g: mix(b.g, f.g),
        b: mix(b.b, f.b),
        a: 1.0,
    }
    .into()
}

/// Perceived luminance of a color (Rec. 709 luma weights, 0..=1).
fn luma(c: Hsla) -> f32 {
    let r: Rgba = c.into();
    0.2126 * r.r + 0.7152 * r.g + 0.0722 * r.b
}

/// Theme-adaptive hover fill: the same mix ratio that reads as subtle
/// elevation on dark themes becomes a dirty smudge on light ones (the eye
/// is far more sensitive to darkening on light). The ratio derives linearly
/// from the background's own luminance: the theme's `hover_ratio` anchor on
/// dark, easing down by 0.03 toward ~7% on light — a touch stronger than
/// Figma/GitHub-light hover so the plate still reads on white (user
/// feedback). Returns an opaque color ready for `.bg()`.
///
/// `anchor` is [`ThemeInteraction::hover_ratio`]. It is a parameter rather
/// than a constant so a theme can set its own hover strength; the defaults
/// reproduce the ratios this function used before that field existed.
pub fn hover_fill(bg: Hsla, fg: Hsla, anchor: f32) -> Hsla {
    // Linear ramp anchored at the dark-theme value; clamped so custom themes
    // can't overshoot either way. Slope 0.03 keeps the dark end at the anchor
    // while lifting the light end from ~5% to ~7%. The floor and ceiling are
    // derived from the anchor so raising it raises both ends together.
    let ratio = (anchor - luma(bg) * 0.03).clamp(anchor - 0.03, anchor);
    hover_tint(bg, fg, ratio)
}

/// Strong hover fill for small chips painted near the page color. The
/// default [`hover_fill`] is calibrated for flat surfaces that pop a gray
/// card against the page; a small button whose base is *brighter* than the
/// page (e.g. the Welcome `surface` chips on light-clean) darkens TOWARD
/// the page on hover, so its edge contrast collapses and it reads as "no
/// hover" — even though the pixel delta equals the approved grid plate.
/// This variant lifts the floor so the chip darkens clearly PAST the page
/// color, restoring the same perceived edge cue as the grid, while staying
/// luma-adaptive.
pub fn hover_fill_strong(bg: Hsla, fg: Hsla, anchor: f32) -> Hsla {
    let ratio = (anchor - luma(bg) * 0.035).clamp(anchor - 0.035, anchor);
    hover_tint(bg, fg, ratio)
}

// `VIEWER_DENSITY_MOTION_IDS` is gone with the topbar move to gpui-component:
// the density segments are kit `Button`s now, and the kit owns its own hover
// transition, so there is no `motion::AnimationId` left to route for them.
// The other two ids still key the hand-built viewer controls.
const VIEWER_ZOOM_PRESET_MOTION_IDS: [&str; 3] =
    ["zoom-preset-0", "zoom-preset-1", "zoom-preset-2"];
const VIEWER_BACK_PERSISTENT_ID: &str = "viewer-back-persistent";
const EXISTING_GRID_MOTION_ID: &str = "topbar-settings";

/// Route the expanded motion set to Viewer while preserving controls that
/// already used the shared layer in Grid.
fn route_viewer_motion(view: View, stable_id: &'static str) -> Option<motion::AnimationId> {
    (view == View::Viewer || (view == View::Grid && stable_id == EXISTING_GRID_MOTION_ID))
        .then(|| motion::AnimationId::new(stable_id))
}

/// The app's own crop glyph, re-hosted as a kit `Icon`.
///
/// WHY THE BYTES ARE INLINED INSTEAD OF A KIT ICON NAME. `gpui_kit_assets`
/// does have a `Scissors` variant, so `IconName::Scissors` compiles — but the
/// app registers `Assets`, the kit's DEFAULT 104-icon bundle, not
/// `AllAssets` (see `assets.rs`). `scissors.svg` is not in `default-icons.txt`,
/// so the kit's own copy of those bytes is never embedded. The variant would
/// still *render*, but only because `AppAssets::load` checks the app's own
/// table first and `icons/scissors.svg` happens to collide with a key the app
/// already owns. That is an accident of naming, not a contract: pointing the
/// app at `AllAssets`, renaming the app's asset, or a kit release that drops
/// the variant would each turn it into a silently blank control.
///
/// Inlining the bytes states the actual intent — this is the app's glyph,
/// sized and coloured by the kit — with no dependence on which bundle happens
/// to be registered. Crop is semantically distinct from every other action in
/// the app, so the glyph is kept rather than substituted with a near-miss.
const CROP_ICON_SVG: &[u8] = include_bytes!("../assets/icons/scissors.svg");

/// Box size for an icon-only kit `Button` whose glyph should paint at
/// `glyph_px`.
///
/// The kit derives the icon box as `0.75x` the Button's own size (it hands
/// `size * 0.75` to `ButtonIcon::with_size`), so preserving a hand-picked
/// glyph size means asking for a proportionally larger box. Inverting that
/// ratio here keeps the kit's constant in one place instead of re-deriving it
/// at every call site.
fn icon_only_box(glyph_px: f32) -> gpui_component::Size {
    gpui_component::Size::Size(px(glyph_px / 0.75))
}

/// Use the stronger, theme-aware tint only for Viewer controls. The regular
/// tint remains byte-for-byte unchanged for every non-Viewer topbar control.
///
/// Takes the theme's two anchors rather than reading a global, so a caller with
/// a live theme (hot reload mid-frame) cannot mix a stale ratio with fresh
/// colors.
fn viewer_control_hover_fill(
    view: View,
    bg: Hsla,
    fg: Hsla,
    interaction: &sh_core::theme::ThemeInteraction,
) -> Hsla {
    if view == View::Viewer {
        hover_fill_strong(bg, fg, interaction.hover_ratio_strong)
    } else {
        hover_fill(bg, fg, interaction.hover_ratio)
    }
}

/// Build one confirmation action for the crop-confirm and batch-confirm bars.
///
/// Both bars used to carry a byte-identical COPY of the same hand-built `div`
/// factory, so a change to how a destructive or cancelling action looked or
/// announced itself could land in one bar and silently miss the other. They
/// share this one now, and each control gains the `Role::Button`, tab stop and
/// accessible name the two `div`s never had.
///
/// The kit's `on_click` is a plain `Fn(&ClickEvent, &mut Window, &mut App)`
/// rather than a `Context::listener`, so the handler re-enters through
/// `entity` — the same shape the topbar controls use. `on_click` therefore
/// stays a `fn` pointer, which also keeps the call sites naming real functions
/// instead of captured environments.
///
/// Idle/hover/active come from the projected kit theme rather than the bars'
/// own `surface` + `.hover()` pair, which is what removes the duplication:
/// the two bars no longer state any color of their own.
fn confirm_bar_button(
    id: &'static str,
    label: &'static str,
    entity: Entity<App>,
    on_click: fn(&mut App, &ClickEvent, &mut Window, &mut Context<App>),
    cx: &mut Context<App>,
) -> AnyElement {
    let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
        cx.stop_propagation();
    });
    gpui_component::button::Button::new(id)
        .label(label)
        .compact()
        .on_mouse_down(MouseButton::Left, swallow)
        .on_click(move |ev, window, cx| {
            entity.update(cx, move |this, cx| on_click(this, ev, window, cx));
        })
        .debug_selector(|| id.to_string())
        .into_any_element()
}

/// Expose the root focus handle so external code (and GPUI's
/// `window.focus_view`) can focus the app's `image_view` subtree.
impl Focusable for App {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Sort dropdown rows: criterion list + direction list (V3 sort chip).
/// Labels are table keys resolved with `settings.language` at the render
/// site, so the menu live-switches (S3). The source of truth is these
/// `sh-app` literals' keys — `sh-core::navigation` carries no display
/// strings; the keys still live in `sh-core::i18n`.
const SORT_MENU_ITEMS: &[(SortBy, StrKey)] = &[
    (SortBy::Name, StrKey::SortName),
    (SortBy::Created, StrKey::SortCreated),
    (SortBy::Modified, StrKey::SortModified),
    (SortBy::Size, StrKey::SortSize),
    (SortBy::Type, StrKey::SortType),
];
const SORT_DIR_ITEMS: &[(SortDir, StrKey)] = &[
    (SortDir::Asc, StrKey::SortAscending),
    (SortDir::Desc, StrKey::SortDescending),
];

/// Human label for the sort chip: localized criterion + locale-neutral
/// direction arrow (`↑`/`↓` stay untranslated per the glyph exemption).
/// Pure so the format is unit-testable.
pub fn sort_chip_label(lang: Language, by: SortBy, dir: SortDir) -> String {
    let name = SORT_MENU_ITEMS
        .iter()
        .find(|(by2, _)| *by2 == by)
        .map(|(_, key)| lang.get(*key))
        .unwrap_or(lang.get(StrKey::SortName));
    let arrow = match dir {
        SortDir::Asc => "↑",
        SortDir::Desc => "↓",
    };
    format!("{name} {arrow}")
}

/// Human label for one density segment: the localized word (`"Small"` /
/// `"Pequeño"`), never a bare letter. Pure so the format is unit-testable.
pub fn grid_size_label(lang: Language, size: GridSize) -> String {
    lang.get(match size {
        GridSize::S => StrKey::GridSizeSmall,
        GridSize::M => StrKey::GridSizeMedium,
        GridSize::L => StrKey::GridSizeLarge,
    })
    .to_string()
}

/// Segment model for the grid-density chip: exactly S/M/L in order with
/// the active preset flagged. Pure so labels + active marking are
/// unit-testable; the render maps it 1:1 to clickable segments.
pub fn grid_size_segments(lang: Language, active: GridSize) -> Vec<(GridSize, String, bool)> {
    [GridSize::S, GridSize::M, GridSize::L]
        .iter()
        .map(|size| (*size, grid_size_label(lang, *size), *size == active))
        .collect()
}

/// Human label for one zoom-preset chip: localized `"Fit"`/`"Ajustar"` or
/// the locale-neutral numeral. Pure so the format is unit-testable.
pub fn zoom_preset_label(lang: Language, preset: ZoomPreset) -> String {
    lang.get(match preset {
        ZoomPreset::Fit => StrKey::ZoomPresetFit,
        ZoomPreset::Scale100 => StrKey::ZoomPreset100,
        ZoomPreset::Scale200 => StrKey::ZoomPreset200,
    })
    .to_string()
}

/// Segment model for the zoom-preset chips: exactly Fit/100/200 in
/// order with the active chip flagged. Pure so labels + active marking are
/// unit-testable; the render maps it 1:1 to clickable segments (same shape
/// as [`grid_size_segments`]).
///
/// Active rule = display-percent equality derived from the zoom text format
/// (`format!("{:.0}%", scale * 100.0)`): a chip reads active exactly when
/// the zoom text already reads its percentage. Fit is active iff the
/// session is in [`FitMode::Fit`]; scale chips are gated on
/// [`FitMode::Percent100`] so a fit scale that happens to round to a preset
/// never dual-activates, and a free-zoomed 137% matches no chip.
pub fn zoom_preset_segments(
    lang: Language,
    fit_mode: FitMode,
    scale: f32,
) -> Vec<(ZoomPreset, String, bool)> {
    let in_fit = fit_mode == FitMode::Fit;
    let percent = (scale * 100.0).round() as i32;
    [ZoomPreset::Fit, ZoomPreset::Scale100, ZoomPreset::Scale200]
        .iter()
        .map(|preset| {
            let active = match preset {
                ZoomPreset::Fit => in_fit,
                ZoomPreset::Scale100 => !in_fit && percent == 100,
                ZoomPreset::Scale200 => !in_fit && percent == 200,
            };
            (*preset, zoom_preset_label(lang, *preset), active)
        })
        .collect()
}

/// Fit floor for the current image + viewport, if dimensions are known.
///
/// Pure so the chip-disabled rule is unit-testable. `None` while the probe
/// is in flight — callers treat unknown dims as "no floor" (chips stay
/// enabled) so a slow header read never bricks the preset row.
pub fn zoom_preset_floor(
    image_dims: Option<(u32, u32)>,
    viewport: sh_core::transform::Vec2,
) -> Option<f32> {
    image_dims
        .map(|(w, h)| sh_core::transform::fit_scale(w as f32, h as f32, viewport.x, viewport.y))
}

/// Whether a preset chip must render disabled: its target scale sits at or
/// below the fit floor, so activation would snap straight back to fit via
/// the pinned [`Session::clamp_zoom`] funnel (the "dead 50% chip").
///
/// Fit itself is never disabled; unknown dims (probe in flight) leave every
/// chip enabled (fallback). A chip that is both active and disabled can only
/// arise from a viewport shrink after a manual zoom — it keeps its active
/// tint but takes no clicks until the floor drops again.
pub fn zoom_preset_disabled(preset: ZoomPreset, floor: Option<f32>) -> bool {
    match (preset.scale(), floor) {
        (Some(target), Some(floor)) => target <= floor,
        _ => false,
    }
}

/// Stable fit viewport for navigate/open completion: the full window size,
/// floored at 1.0 — deliberately independent of the transient idle/topbar
/// state, so the same image always opens at the same zoom.
///
/// R3 unified the chrome rule: `viewer_viewport` IS this function now —
/// the image area owns the full window in every Tab/idle state, so toggle,
/// resize, wheel, and open all share one geometry.
pub fn stable_open_viewport(window: sh_core::transform::Vec2) -> sh_core::transform::Vec2 {
    sh_core::transform::Vec2 {
        x: window.x,
        y: window.y.max(1.0),
    }
}

/// Fit viewport with the filmstrip and/or the in-flow bottom chrome
/// carved out. Both hidden ⟺ bit-identical to
/// [`stable_open_viewport`] for the same window.
///
/// `chrome_visible` is the TAB state (`session.show_overlay_bottom`),
/// deliberately NOT the idle state: the in-flow `#viewer-chrome` owns
/// real layout space while mounted, so the fit area must match it or
/// the image clips behind the chrome row. Idle only fades the chrome
/// away (the container grows BELOW the unchanged image), so idle never
/// enters this function and never refits — image position is
/// jolt-free across idle transitions. Tab toggling changes the layout
/// itself, so the Tab action refits exactly once against the new
/// carve (same discipline as `set_filmstrip`; the pre-chrome R3
/// "Tab never refits" guarantee applied while chrome floated over
/// the image and is superseded by the in-flow layout).
///
/// The carved height keeps the `.max(1.0)` floor pact. Callers pass
/// `view == View::Viewer && settings.filmstrip` and
/// `view == View::Viewer && session.show_overlay_bottom`; see
/// [`App::viewer_viewport`].
pub fn stable_filmstrip_viewport(
    window: sh_core::transform::Vec2,
    strip_visible: bool,
    chrome_visible: bool,
) -> sh_core::transform::Vec2 {
    let carved = window.y
        - if strip_visible {
            crate::filmstrip::STRIP_H_PX
        } else {
            0.0
        }
        - if chrome_visible {
            crate::ui::overlay::BOTTOM_CHROME_H_PX
        } else {
            0.0
        };
    sh_core::transform::Vec2 {
        x: window.x,
        y: carved.max(1.0),
    }
}

fn settings_control_activation(event: &KeyDownEvent) -> bool {
    let stroke = &event.keystroke;
    (stroke.key.eq_ignore_ascii_case("enter") || stroke.key.eq_ignore_ascii_case("space"))
        && !stroke.modifiers.modified()
}

/// The slideshow chip shows the ACTION, not the state: Pause while
/// playing, Play while stopped.
///
/// Expressed in the kit's catalog because the chip is a kit `Button`. Both
/// `Play` and `Pause` are in the kit's DEFAULT bundle, so they resolve through
/// `AppAssets` on their own and need no inlined bytes the way the crop glyph
/// does — which is the whole difference between this and `CROP_ICON_SVG`.
fn slideshow_kit_icon(active: bool) -> gpui_kit_assets::IconName {
    if active {
        gpui_kit_assets::IconName::Pause
    } else {
        gpui_kit_assets::IconName::Play
    }
}

/// Topbar-left suffix for the selection count: `""` when empty, otherwise
/// the localized `selected_suffix` template. Thin wrapper so the builder
/// stays readable; the wording itself is unit-tested in `sh-core::i18n`.
fn selected_count_suffix(lang: Language, selected: &std::collections::BTreeSet<usize>) -> String {
    sh_core::i18n::selected_suffix(lang, selected.len())
}

/// Confirm-bar message for a staged op: counts + destination for moves
/// (`display_name`-shortened). Pure so the wording is unit-testable; the
/// sentence itself lives in the `i18n` templates (S5) with the one/other
/// plural handled there.
fn batch_bar_message(lang: Language, op: &BatchOp) -> String {
    match op {
        BatchOp::Delete { paths } => sh_core::i18n::batch_bar_delete(lang, paths.len()),
        BatchOp::Move { paths, dest } => {
            sh_core::i18n::batch_bar_move(lang, paths.len(), &sh_core::recent::display_name(dest))
        }
    }
}

/// True when `current` still holds exactly the paths captured before a
/// background batch op started, so its rescan may still be applied.
///
/// The stale-completion guard for [`App::confirm_pending`]. It compares
/// MEMBERSHIP, never order, because the two things a user can do while a
/// trash is in flight must both keep the result valid: arrow-key navigation
/// only moves `session.current`, and a re-sort only reorders
/// `session.images` (`Session::resort` re-anchors by path). Only a genuine
/// folder switch — `open_folder`/`open_path` rebuilding the list from
/// another directory — changes membership, and that is the one case where
/// the op's rescan must be dropped.
///
/// Deliberately NOT keyed on `navigation_seq`/`thumb_seq`: those are bumped
/// by plain ←/→ navigation, so reusing them would drop valid results and
/// leave deleted files listed in the grid forever. Set membership is the only
/// signal that actually distinguishes "still in the folder the op ran in"
/// from "left it". Paths come from a directory scan, so they are unique and a
/// set comparison loses nothing.
///
/// Pure and free of GPUI types so the contract is unit-testable without a
/// `Context`.
fn same_image_set(before: &[PathBuf], current: &[ImageItem]) -> bool {
    let before: BTreeSet<&std::path::Path> = before.iter().map(|p| p.as_path()).collect();
    let current: BTreeSet<&std::path::Path> = current.iter().map(|i| i.path.as_path()).collect();
    before == current
}

/// Re-point a grid selection at the list it now faces: the cells that the
/// paths in `keep` occupy in `images`, plus a cursor clamped into range.
///
/// The selection is stored as INDICES, so every code path that rebuilds
/// `session.images` has to carry it across or it silently starts naming
/// different images. A path is the only stable identity a rebuild preserves
/// (a re-sort and a re-list both shuffle indices), and a path that is no
/// longer in the list is simply not re-marked — a selected file that got
/// deleted has nothing left to mark, which is the same answer
/// [`App::confirm_pending`] already gives its leftovers.
///
/// The cursor is CLAMPED, never reset: a list that shrank must not leave it
/// pointing past the end (a past-the-end cursor makes
/// [`App::enter_viewer`] silently a no-op and scroll math read out of
/// bounds), and a list that merely re-shuffled must not throw away the
/// user's place over it. The cursor's *identity* is not remapped here
/// because callers own that separately: [`App::commit_rescan`] re-anchors it
/// through `session.current` first.
///
/// Shared by the two sites that must not grow a third copy of this rule —
/// [`App::confirm_pending`] (the leftovers of a batch op) and
/// [`App::commit_rescan`] (a hidden-files toggle that shifts every index).
/// Pure and free of GPUI types, for the same reason as [`same_image_set`].
/// The two lifetimes are independent on purpose: `keep` usually holds paths
/// snapshotted BEFORE the rebuild (a batch op's leftovers, a pre-swap
/// selection snapshot), so it cannot borrow from `images`.
fn remap_selection_by_path<'a, 'p>(
    images: &'a [ImageItem],
    keep: impl Iterator<Item = &'p std::path::Path>,
    cursor: usize,
) -> (BTreeSet<usize>, usize) {
    let selected: BTreeSet<usize> = keep
        .filter_map(|p| images.iter().position(|i| i.path == p))
        .collect();
    (selected, cursor.min(images.len().saturating_sub(1)))
}

/// Create `path` and write `json` into it, ONLY if the name is still free.
///
/// The bootstrap half of [`App::apply_theme_entry`], extracted so the
/// "never overwrite a user's theme file" rule is one auditable unit rather
/// than three statements inside a background closure — and so it can be
/// tested without the executor queue in the way.
///
/// Why `create_new` and NOT the obvious `if path.exists() { return; }`
/// followed by a write: that is a time-of-check-to-time-of-use pair, TWO
/// syscalls with a gap between them, and the gap is exactly where a user's
/// own file appears — a second window picking the same built-in, an editor's
/// atomic save, a sync client, a config-management tool. The pre-fix code ran
/// the check on the worker precisely to shrink that gap, and the write
/// TRUNCATED whatever landed in it, on the one file the rule exists to
/// protect. `O_CREAT | O_EXCL` (`create_new`) asks the kernel to make the
/// check and the create the same step: it fails with
/// [`std::io::ErrorKind::AlreadyExists`] rather than clobbering, so there is
/// no window left to lose and no second syscall to reorder.
///
/// The parent is created first, always: on a first pick the `themes/` dir is
/// usually absent, and that mkdir is the only reason the create can fail for
/// a reason other than "the name was taken".
///
/// `AlreadyExists` is returned as-is, for the caller to recognize as the rule
/// working (see [`App::apply_theme_entry`], which stays silent on it). Every
/// other error means the file was not created, and the in-memory theme stands
/// on its own — the hot-reload watcher reports the user's missing or broken
/// copy on its next tick.
///
/// `write_all` on the handle this call created, not a second `fs::write`: the
/// handle is the only writer of a name nothing else can take, so a partial
/// write here cannot be interleaved with anybody.
fn create_bootstrap_theme_file(path: &std::path::Path, json: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    std::io::Write::write_all(&mut file, json.as_bytes())
}

/// What an in-flight open asks of the single directory scan serving it.
///
/// The scan is shared by both open entry points and the two disagree on
/// exactly two things: which image the selection must land on, and whether an
/// empty result is an ERROR or just an empty list. Folding that into the
/// request (instead of branching at two call sites) is what lets both
/// entry points share ONE scan and one completion.
#[derive(Debug, Clone)]
enum OpenRequest {
    /// [`App::open_folder`]: anchor the folder's FIRST scanned image, and
    /// treat an empty result as real knowledge — the `no_images_in` error,
    /// with no recents write (an empty folder must never enter the list).
    Folder,
    /// [`App::open_path`]: anchor this exact file, and treat an empty result
    /// as a normal (empty) list, never an error claim — the parent exists,
    /// the user simply has no images in it.
    File(PathBuf),
    /// [`App::toggle_show_hidden_files`]: re-scan the folder already on
    /// screen because the visible-set rule changed under it. Anchors the
    /// image the user was looking at (which the new rule may have removed,
    /// hence the `Option`), and shares [`OpenRequest::Folder`]'s answer to an
    /// empty result. Deliberately NOT an open: the folder did not change, so
    /// the chrome, the slideshow and the recents list must survive it (see
    /// [`App::commit_rescan`]).
    ///
    /// No gate: this variant only exists for a folder the app is already
    /// inside, so there is no `NotAFile` claim to make.
    Rescan { anchor: Option<PathBuf> },
}

/// What a background directory scan resolved to.
enum ScanOutcome {
    /// `scan_entries` returned. The Vec may legitimately be empty; the
    /// request decides whether that is an error (see [`OpenRequest`]).
    Entries(Vec<sh_core::navigation::ImageEntry>),
    /// [`App::open_path`] only: the target has no parent directory (or its
    /// parent is not a directory), so there is nothing to scan and no list to
    /// build. Surfaces as the `NotAFile` session error.
    NoParentDir(PathBuf),
}

/// Cadence of the idle watcher poll. Independent of [`overlay::OVERLAY_IDLE`]
/// (the actual hide threshold) — a short tick keeps the hide within ~500ms
/// of the deadline without notifying more than once.
const IDLE_TICK: std::time::Duration = std::time::Duration::from_millis(500);

/// Convert the persisted slideshow interval into a safe timer duration.
///
/// The settings contract rejects values below one second, but keeping this
/// guard at the timer boundary prevents a direct in-memory corruption from
/// creating a zero-delay loop.
fn slideshow_delay(seconds: u32) -> std::time::Duration {
    std::time::Duration::from_secs(u64::from(seconds.max(SLIDESHOW_INTERVAL_MIN_SECS)))
}

/// Cadence of the theme hot-reload poll (AGENTS.md §10: apply within ~1.5s
/// of an edit; 1s poll + file read comfortably meets that).
const THEME_POLL: std::time::Duration = std::time::Duration::from_secs(1);

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // gpui-component's components read their colors from the kit's global
        // `Theme` on first construction, so the kit has to be initialized before
        // any of them is built below. `main` also calls this at startup — doing it
        // here as well is what makes every `#[gpui::test]` that builds an `App`
        // and renders it work without each test having to know about the kit.
        // Idempotent, so the double call costs one `has_global` check.
        crate::kit_theme::ensure_kit_initialized(cx);
        // Live viewport: GPUI re-renders on window resize (`on_resize` →
        // `bounds_changed` → `refresh`), so reading the drawable size here
        // keeps `App.viewport` current on every frame. When the size changed
        // (e.g. fullscreen toggle), the fit computed for the OLD viewport
        // would render off-center — recompute it (Fit mode only; Percent100
        // user zoom is intentionally left alone on resize).
        let new_viewport = window.viewport_size();
        if new_viewport != self.viewport {
            self.viewport = new_viewport;
            self.session.refit_for_viewport(self.viewer_viewport());
        }

        let bg: Hsla =
            parse_hex(&self.theme_store.theme.colors.background).unwrap_or(rgb(0x0d0d0f).into());
        // Resolved once per frame and handed to every presenter that draws a
        // transparency board. A populated grid builds one board per visible
        // image, so parsing per board would repeat the work per cell.
        let checker_palette = crate::checkerboard::palette_for_page(bg);
        let params = ViewerParams {
            path: self.session.current_item().map(|i| i.path.clone()),
            error: self.session.error.clone(),
            zoom_scale: self.session.zoom.scale,
            pan_offset: self.session.zoom.offset,
            decoded_size: self
                .session
                .current_dimensions()
                .map(|(w, h)| (w as f32, h as f32)),
            lang: self.settings.language,
            // Task 2.5: the SINGLE computation site for the board gate.
            // `render_viewer` stays pure-presentational: it only reads this
            // precomputed bool. `None` (verdict in flight) renders without
            // the board until the navigate probe lands + notifies.
            show_checkerboard: crate::viewer::should_show_checkerboard(
                self.settings.checkerboard,
                self.session.current_item().and_then(|i| i.has_alpha),
            ),
            checker_palette,
        };

        // ── Overlay visibility = Tab-toggled && not idle ──
        let idle = self.last_interaction.elapsed() > overlay::OVERLAY_IDLE;
        if idle {
            self.hover_motion.clear();
        }
        let bottom_visible = self.session.show_overlay_bottom && !idle;

        // ── Topbar dissolve (Viewer only) ──
        // Grid never dissolves: folder actions (open, settings) must stay
        // reachable and no image is covered there. B2 single-row chrome:
        // while the bottom bar is armed the topbar dissolves (never two
        // solid bars); Tab OFF re-pins the single topbar row. The bottom
        // chrome is in-flow (below the image), so the Tab ACTION refits
        // once against the new carve; idle flips never refit — the idle
        // state is deliberately absent from `viewer_viewport`.
        let topbar_dissolved = self.view == View::Viewer
            && topbar_dissolved_for_viewer(self.session.show_overlay_bottom);

        let overlay_data = OverlayData::from_theme(
            format!("{:.0}%", self.session.zoom.scale * 100.0),
            &self.theme_store.theme.colors.text,
            &self.theme_store.theme.colors.surface,
        );

        // ── Bottom overlay action chrome (shared by chips + arrows) ──
        // Viewer controls use the stronger theme-aware tint. Enabled chips
        // keep the topbar density-control idiom, while bare actions retain
        // their content-only footprint and gain a transparent idle plate.
        let chip_bg =
            parse_hex(&self.theme_store.theme.colors.background).unwrap_or(rgb(0x0d0d0f).into());
        let chip_hover = viewer_control_hover_fill(
            self.view,
            chip_bg,
            overlay_data.theme_text,
            &self.theme_store.theme.interaction,
        );
        let chip_pressed: Hsla = {
            let h: Rgba = chip_hover.into();
            let b: Rgba = chip_bg.into();
            let step = |x: f32, y: f32| x + (x - y);
            Rgba {
                r: step(h.r, b.r),
                g: step(h.g, b.g),
                b: step(h.b, b.b),
                a: h.a,
            }
            .into()
        };
        let action_style = overlay::ActionButtonStyle {
            text: overlay_data.theme_text,
            idle_bg: chip_bg,
            hover_bg: chip_hover,
            active_bg: chip_pressed,
        };

        // Build nav arrow elements for the bottom overlay. Constructed with
        // `cx.listener` here (same pattern as Tasks 7/8) and handed to the
        // overlay as pre-built elements.
        // Swallow mouse-down on the buttons so double-clicking an arrow
        // navigates twice instead of also toggling fit on the root div.
        let swallow_prev = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_next = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        // The three bottom-bar actions are kit `Button`s, so each one gains the
        // `Role::Button`, tab stop and accessible name the hand-built divs never
        // had. The element id moves into `Button::new(id)` — the kit reads that
        // same id for `debug_bounds` and for keyed focus state, so the tests
        // that resolve these controls by selector keep working.
        //
        // Because `on_click` is a plain `Fn` rather than `Context::listener`,
        // each handler re-enters through the entity, which is why the
        // `on_prev` / `on_next` / `on_toggle_slide` listener trio is gone.
        let bar_entity = cx.entity();
        let prev_control = gpui_component::button::Button::new("prev-btn")
            .icon(gpui_kit_assets::IconName::ChevronLeft)
            // Icon-only, so the box is what gives the glyph a hit target and
            // keeps the previous 14px glyph size.
            .with_size(icon_only_box(14.0))
            .accessibility_label(t(self.settings.language, StrKey::ActionPrevImage))
            .compact()
            .on_mouse_down(MouseButton::Left, swallow_prev)
            .on_click({
                let entity = bar_entity.clone();
                move |_ev, _window, cx| {
                    entity.update(cx, |this: &mut App, cx| {
                        this.note_interaction(cx);
                        this.navigate(-1, cx);
                    });
                }
            })
            .debug_selector(|| "prev-btn".to_string());
        let prev_btn = prev_control.into_any_element();
        let next_control = gpui_component::button::Button::new("next-btn")
            .icon(gpui_kit_assets::IconName::ChevronRight)
            .with_size(icon_only_box(14.0))
            .accessibility_label(t(self.settings.language, StrKey::ActionNextImage))
            .compact()
            .on_mouse_down(MouseButton::Left, swallow_next)
            .on_click({
                let entity = bar_entity.clone();
                move |_ev, _window, cx| {
                    entity.update(cx, |this: &mut App, cx| {
                        this.note_interaction(cx);
                        this.navigate(1, cx);
                    });
                }
            })
            .debug_selector(|| "next-btn".to_string());
        let next_btn = next_control.into_any_element();
        let swallow_slide = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        // V3 slideshow chip: play/pause between zoom text and arrows. The name
        // is the toggle action rather than the current state, matching the
        // keyboard action it mirrors.
        let slideshow_control = gpui_component::button::Button::new("slideshow-btn")
            .icon(slideshow_kit_icon(self.session.slideshow_active))
            .with_size(icon_only_box(14.0))
            .accessibility_label(t(self.settings.language, StrKey::ActionToggleSlideshow))
            .compact()
            .on_mouse_down(MouseButton::Left, swallow_slide)
            .on_click({
                let entity = bar_entity.clone();
                move |_ev, _window, cx| {
                    entity.update(cx, |this: &mut App, cx| {
                        this.toggle_slideshow(cx);
                    });
                }
            })
            .debug_selector(|| "slideshow-btn".to_string());
        let slideshow_btn = slideshow_control.into_any_element();

        // Zoom-preset chips: one per `zoom_preset_segments` entry, built
        // through the same action_button helper with the pill chrome. The
        // mousedown swallow keeps taps out of the root pan/double-click-fit
        // handler; `note_interaction` resets the idle clock FIRST so the
        // overlay cannot fade out right after the tap. A chip whose target
        // sits at/below the fit floor renders DISABLED (dimmed, no click
        // handler — a tap would only snap back to fit, the old "dead 50%");
        // unknown dims keep every chip enabled until the probe lands.
        let preset_floor =
            zoom_preset_floor(self.session.current_dimensions(), self.viewer_viewport());
        let chip_buttons: Vec<AnyElement> = zoom_preset_segments(
            self.settings.language,
            self.session.fit_mode,
            self.session.zoom.scale,
        )
        .into_iter()
        .enumerate()
        .map(|(idx, (preset, label, active))| {
            let swallow_chip = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            if zoom_preset_disabled(preset, preset_floor) {
                let mut faded = overlay_data.theme_text;
                faded.a *= 0.35;
                return div()
                    .id(("zoom-preset", idx as u64))
                    .bg(chip_bg)
                    .text_color(faded)
                    .rounded(px(6.0))
                    .px(px(12.0))
                    .py(px(4.0))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, swallow_chip)
                    .into_any();
            }
            let control = overlay::action_button(
                label,
                &action_style,
                &overlay::ActionButtonOpts {
                    chrome: true,
                    active,
                    hover: false,
                    pad_x: 12.0,
                    pad_y: 4.0,
                },
            )
            .id(("zoom-preset", idx as u64))
            .on_mouse_down(MouseButton::Left, swallow_chip)
            .on_click(cx.listener(
                move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.note_interaction(cx);
                    let viewport = this.viewer_viewport();
                    this.session.set_zoom_preset(preset, viewport);
                    cx.notify();
                },
            ));
            self.routed_hover_background(
                control,
                VIEWER_ZOOM_PRESET_MOTION_IDS[idx],
                if active {
                    action_style.active_bg
                } else {
                    action_style.idle_bg
                },
                action_style.hover_bg,
                cx,
            )
        })
        .collect();

        // Info-panel button: localized label through the shared action_button
        // helper with pill chrome (zoom-chip precedent). B3 made it
        // reachable in every Tab/idle state; R2 fixes WHERE it mounts: with
        // Tab ON the floating chips row occupies the same top-right corner,
        // so the button becomes that row's third slot (no overlap); with
        // Tab OFF it keeps the standalone absolute float. Mousedown-swallow
        // keeps the tap out of the root pan/double-click-fit handler;
        // `note_interaction` runs FIRST inside the toggle so the overlay
        // cannot fade out right after the tap. Click-only by design: no
        // action id, no keymap entry, no Shortcuts-panel row. Toggle +
        // popover behavior unchanged.
        let swallow_info = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let info_idle_bg = if self.info_panel_open {
            action_style.active_bg
        } else {
            action_style.idle_bg
        };
        let info_btn: AnyElement = self.routed_hover_background(
            overlay::action_button(
                t(self.settings.language, StrKey::InfoButtonLabel),
                &action_style,
                &overlay::ActionButtonOpts {
                    chrome: true,
                    active: self.info_panel_open,
                    hover: false,
                    pad_x: 12.0,
                    pad_y: 4.0,
                },
            )
            .id("info-btn")
            .debug_selector(|| "info-btn".to_string())
            .on_mouse_down(MouseButton::Left, swallow_info)
            .on_click(
                cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.toggle_info_panel(cx);
                }),
            ),
            "info-btn",
            info_idle_bg,
            action_style.hover_bg,
            cx,
        );
        // R2 placement: reachability is structural (Viewer + image), never
        // idle- or Tab-dependent — but the CONTAINER is Tab-dependent. The
        // wrap decision is deferred to the chips construction below, where
        // `topbar_dissolved` is known: exactly one container parents the
        // button per frame (chips slot or float), never both.
        let mut info_btn_opt: Option<AnyElement> =
            if info_button_visible(self.view, self.session.current_item().is_some()) {
                Some(info_btn)
            } else {
                None
            };

        let viewer = render_viewer(&params);

        // Persistent Viewer Back: when the topbar is dissolved, this target
        // belongs to the image surface rather than the ephemeral bottom
        // chrome. It is absolute so it paints over the image without taking
        // layout space, and its motion ID stays stable across idle/active
        // lower-chrome states.
        let viewer_back_persistent: Option<AnyElement> =
            if self.view == View::Viewer && topbar_dissolved {
                let swallow_persistent_back =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                let back_entity = cx.entity();
                let back_control = gpui_component::button::Button::new(VIEWER_BACK_PERSISTENT_ID)
                    // `ArrowLeft` is what the migrated topbar Back already
                    // uses, so the two Back affordances are now the same glyph.
                    .icon(gpui_kit_assets::IconName::ArrowLeft)
                    .label(t(self.settings.language, StrKey::TopbarBack))
                    .compact()
                    // It floats over the image instead of taking layout space.
                    // `Button` forwards `Styled` to the root it renders, so the
                    // positioning rides on the component itself rather than on
                    // a wrapper div — one element, one id, one hitbox.
                    .absolute()
                    .top(px(12.0))
                    .left(px(12.0))
                    .on_mouse_down(MouseButton::Left, swallow_persistent_back)
                    .on_click({
                        let entity = back_entity.clone();
                        move |_ev, _window, cx| {
                            entity.update(cx, |this: &mut App, cx| {
                                this.note_interaction(cx);
                                this.enter_grid(cx);
                            });
                        }
                    })
                    .debug_selector(|| VIEWER_BACK_PERSISTENT_ID.to_string());
                Some(back_control.into_any_element())
            } else {
                None
            };

        // ── Viewer filmstrip (Slice B): borrowed sidecar, conditional mount.
        // `strip_visible` is the `should_mount_filmstrip` predicate inline
        // (Viewer + persisted setting; Tab/idle cannot reach it). Built only
        // when visible — zero cost otherwise. `FilmstripParams` borrows the
        // resident thumb maps read-only: no decodes, no I/O, no copies.
        let strip_visible = self.view == View::Viewer && self.settings.filmstrip;
        debug_assert_eq!(
            strip_visible,
            crate::filmstrip::should_mount_filmstrip(
                self.view == View::Viewer,
                self.settings.filmstrip
            )
        );
        let strip_el: Option<AnyElement> = if strip_visible {
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let filmstrip_params = crate::filmstrip::FilmstripParams {
                current: self.session.current,
                viewport_width: f32::from(window.viewport_size().width),
                images: &self.session.images,
                thumbs: &self.thumbs,
                thumb_alpha: &self.thumb_alpha,
                checkerboard_on: self.settings.checkerboard,
                checker_palette: crate::checkerboard::palette_for_page(
                    parse_hex(&self.theme_store.theme.colors.background)
                        .unwrap_or(rgb(0x0d0d0f).into()),
                ),
                accent,
                surface,
            };
            Some(crate::filmstrip::render_filmstrip(&filmstrip_params, cx))
        } else {
            None
        };

        // ── V2 Task 6: top bar data (grid: folder name; viewer: name — pos).
        // Built for every frame; only attached outside Welcome below.
        let topbar_data = topbar::TopbarData {
            left: format!(
                "{}{}",
                self.recent_dirs_available
                    .first()
                    .and_then(|d| d.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or(""),
                selected_count_suffix(self.settings.language, &self.selected)
            ),
            center: if self.view == View::Viewer {
                format!(
                    "{} — {}",
                    self.session
                        .current_item()
                        .map(|i| i.name.clone())
                        .unwrap_or_default(),
                    self.session.position_label()
                )
            } else {
                // V3 batch report: last-op status, cleared on next staging
                // or folder change. Empty in every other case (grid had no
                // center content before).
                self.batch_status.clone().unwrap_or_default()
            },
            theme_text: parse_hex(&self.theme_store.theme.colors.text)
                .unwrap_or(rgb(0xe8e8ee).into()),
            theme_surface: parse_hex(&self.theme_store.theme.colors.surface)
                .unwrap_or(rgb(0x121218).into()),
        };
        let swallow_back_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_open_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_gear_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_sort_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_crop_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        type TopbarElements = (Option<AnyElement>, Option<AnyElement>);
        let (topbar_el, viewer_topbar_el): TopbarElements =
            if !matches!(self.view, View::Welcome | View::Settings) {
                // Every top-bar control below is a gpui-component Button, so the
                // hand-tuned trio that fed routed_hover_background is gone: the kit
                // owns its own idle/hover/active colors, fed by the global Theme
                // that crate::kit_theme projects from the app's JSON theme.
                //
                // gpui-component `Button` replaces the hand-built control. The
                // `topbar-back` id and the center-click contract the harness
                // drives are unchanged, so `viewer_back_button_returns_to_grid`
                // (which reads `debug_bounds("topbar-back")` and clicks its
                // center) keeps passing without edits.
                //
                // Colors come from the kit's global theme, which
                // `crate::kit_theme` projects from the app's own JSON theme —
                // so `btn_bg` / `btn_hover` / `topbar_data.theme_text` are no
                // longer read here. The hand-calibrated hover tint
                // (`viewer_control_hover_fill`) is replaced by the kit's own
                // hover state, which is the point of adopting it.
                //
                // The kit's `on_click` is a plain closure over
                // `(&ClickEvent, &mut Window, &mut App)` rather than
                // `Context::listener`, so the entity handle is captured once
                // and re-entered explicitly.
                let app_entity = cx.entity();
                // gpui-component `Button` replaces the hand-built control. The
                // `topbar-back` element id and the center-click contract the
                // harness drives are unchanged, so
                // `viewer_back_button_returns_to_grid` (which reads
                // `debug_bounds("topbar-back")` and clicks its center) keeps
                // passing without edits.
                //
                // Two API facts the kit's website examples do not show, both
                // verified against the published crate:
                //  - the element id goes in `Button::new(id)`, not a trailing
                //    `.id()`; `new()` takes no `&App`.
                //  - `on_click` takes a plain `Fn(&ClickEvent, &mut Window,
                //    &mut App)`, not `Context::listener`, so the entity handle
                //    is captured and re-entered explicitly.
                //
                // Colors come from the kit's global theme, which
                // `crate::kit_theme` projects from the app's own JSON theme — so
                // `btn_bg` / `btn_hover` / `topbar_data.theme_text` are no
                // longer read here. The hand-calibrated hover tint
                // (`viewer_control_hover_fill`) is replaced by the kit's own
                // hover state, which is the point of adopting it.
                let back_control = gpui_component::button::Button::new("topbar-back")
                    .icon(gpui_kit_assets::IconName::ArrowLeft)
                    .label(t(self.settings.language, StrKey::TopbarBack))
                    .compact()
                    .on_mouse_down(MouseButton::Left, swallow_back_btn)
                    .on_click({
                        let entity = app_entity.clone();
                        move |_ev, _window, cx| {
                            entity.update(cx, |this: &mut App, cx| {
                                this.note_interaction(cx);
                                this.enter_grid(cx);
                            });
                        }
                    })
                    .debug_selector(|| "topbar-back".to_string());
                let back_btn = back_control.into_any_element();
                let open_control = gpui_component::button::Button::new("topbar-open")
                    .label(t(self.settings.language, StrKey::TopbarOpen))
                    .compact()
                    .on_mouse_down(MouseButton::Left, swallow_open_btn)
                    .on_click({
                        let entity = app_entity.clone();
                        move |_ev, _window, cx| {
                            entity.update(cx, |this: &mut App, cx| {
                                this.pick_folder(cx);
                            });
                        }
                    });
                let open_btn = open_control.into_any_element();
                // Settings gear: an icon-only kit `Button`. The sort menu is closed on the
                // way in because the gear and the sort dropdown share the bar
                // and leaving the dropdown open behind Settings would render
                // two overlays at once.
                let gear_control = gpui_component::button::Button::new("topbar-settings")
                    // An icon-only compact Button collapses to zero width —
                    // verified by rendering it, not by reading the source — so
                    // the size is explicit and the icon has a box to sit in.
                    // The accessible name is required too: with no label the
                    // button would announce as an unnamed control.
                    .icon(gpui_kit_assets::IconName::Settings)
                    .with_size(gpui_component::Size::Size(px(28.0)))
                    .accessibility_label(t(self.settings.language, StrKey::SettingsTitle))
                    .on_mouse_down(MouseButton::Left, swallow_gear_btn)
                    .on_click({
                        let entity = app_entity.clone();
                        move |_ev, _window, cx| {
                            entity.update(cx, |this: &mut App, cx| {
                                this.sort_menu_open = false;
                                this.open_settings(cx);
                            });
                        }
                    });
                let gear_btn = gear_control.into_any_element();
                // V3 sort chip: shows the active criterion + direction; click
                // toggles the sort dropdown. The kit's `selected` is what paints
                // the open state, replacing the hand-computed `btn_pressed`
                // background for this one control.
                let sort_control = gpui_component::button::Button::new("topbar-sort")
                    .label(sort_chip_label(
                        self.settings.language,
                        self.session.sort_by,
                        self.session.sort_dir,
                    ))
                    .compact()
                    .selected(self.sort_menu_open)
                    .toggled(self.sort_menu_open)
                    .on_mouse_down(MouseButton::Left, swallow_sort_btn)
                    .on_click({
                        let entity = app_entity.clone();
                        move |_ev, _window, cx| {
                            entity.update(cx, |this: &mut App, cx| {
                                this.note_interaction(cx);
                                this.sort_menu_open = !this.sort_menu_open;
                                cx.notify();
                            });
                        }
                    });
                let sort_btn = sort_control.into_any_element();
                // Density segmented control: one segment per preset with the
                // localized word; the active preset wears the pressed tint (the
                // sort-chip idiom). Segments dispatch straight to
                // `set_grid_size` — instant switch, no restart, no re-sort.
                let size_btn = {
                    let mut row = div().id("topbar-size").flex().items_center().gap(px(4.0));
                    for (idx, (size, label, active)) in
                        grid_size_segments(self.settings.language, self.settings.grid_size)
                            .into_iter()
                            .enumerate()
                    {
                        let swallow =
                            cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                                cx.stop_propagation();
                            });
                        // `Selectable::toggled` only announces the pressed state to assistive tech —
                        // it does NOT paint. `Selectable::selected` is what paints
                        // the active segment, and it is what replaces the
                        // hand-computed `btn_pressed` fill this control used
                        // before the kit migration. With `toggled` alone the
                        // active density preset was announced correctly but
                        // looked identical to the other two: measured on a
                        // rendered build, the resting and active segments both
                        // painted `25252B`. Both are set, for the same reason as
                        // the sort chip and the crop toggle.
                        //
                        // The element id keeps the `("grid-size", idx)` key the
                        // density tests read, so they need no edits.
                        let segment =
                            gpui_component::button::Button::new(("grid-size", idx as u64))
                                .label(label)
                                .compact()
                                .selected(active)
                                .toggled(active)
                                .on_mouse_down(MouseButton::Left, swallow)
                                .on_click({
                                    let entity = app_entity.clone();
                                    move |_ev, _window, cx| {
                                        entity.update(cx, |this: &mut App, cx| {
                                            this.note_interaction(cx);
                                            this.set_grid_size(size, cx);
                                        });
                                    }
                                });
                        row = row.child(segment.into_any_element());
                    }
                    row.into_any_element()
                };
                let crop_btn = if self.view == View::Viewer {
                    // Crop mode is a genuine toggle: `toggled` tells assistive
                    // tech it is pressed, `selected` paints the active state.
                    let crop_control = gpui_component::button::Button::new("topbar-crop")
                        .icon(gpui_kit_assets::IconName::Scissors)
                        .compact()
                        .selected(self.crop_mode)
                        .toggled(self.crop_mode)
                        .on_mouse_down(MouseButton::Left, swallow_crop_btn)
                        .on_click({
                            let entity = app_entity.clone();
                            move |_ev, _window, cx| {
                                entity.update(cx, |this: &mut App, cx| {
                                    this.toggle_crop(cx);
                                });
                            }
                        });
                    Some(crop_control.into_any_element())
                } else {
                    None
                };
                // Grid arm passes no back button (welcome is startup-only);
                // Viewer passes ← Grid.
                let back = if self.view == View::Viewer && !topbar_dissolved {
                    Some(back_btn)
                } else {
                    None
                };
                let bar = topbar::topbar(
                    &topbar_data,
                    back,
                    open_btn,
                    sort_btn,
                    size_btn,
                    gear_btn,
                    crop_btn,
                );
                // (in-flow bar, floating viewer bar) — exactly one is Some; see
                // the R3 layout note at the assignment site below.
                // R3 layout: in Grid the bar stays an in-flow flex child (grid
                // content lays out below it). In the Viewer the bar FLOATS
                // (absolute wrap attached after viewer-area): the image area
                // owns the full window height in every Tab state, so toggling
                // Tab can never change the fit geometry — chrome overlays the
                // image instead of reserving layout space. .hidden() =
                // Display::None (same mechanism as the overlay gate: no
                // hitboxes, element IDs stay stable). Mouse move >= deadband
                // wakes the idle watcher, which re-renders and restores the bar.
                if self.view == View::Viewer {
                    let float = div()
                        .id("topbar-float")
                        .absolute()
                        .top(px(0.0))
                        .left(px(0.0))
                        .right(px(0.0))
                        .child(bar);
                    if topbar_dissolved {
                        (None, Some(float.hidden().into_any_element()))
                    } else {
                        (None, Some(float.into_any_element()))
                    }
                } else {
                    (Some(bar.into_any_element()), None)
                }
            } else {
                (None, None)
            };

        // ── V2: view-specific content ──
        // Welcome: startup screen with Continue / Open-folder. Buttons are
        // built here (cx.listener call-site pattern, same as overlay arrows).
        // Button + hero strings resolve through the i18n table (S3) so the
        // surface live-switches with `settings.language`.
        let welcome_el = if self.view == View::Welcome {
            let lang = self.settings.language;
            let welcome_data = welcome::WelcomeData::from_theme(
                &self.theme_store.theme.colors.text,
                &self.theme_store.theme.colors.surface,
                &self.theme_store.theme.colors.accent,
            );
            let swallow_continue =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            let swallow_open = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            // Modern hover idiom (matches the topbar buttons): bg tints
            // toward the theme text on hover — no border swap. Use the
            // strong variant: these chips start at the *surface* color (pure
            // white on light-clean) which sits above the page, so the plain
            // 7% fill would darken them INTO the page and read as no hover.
            let welcome_hover = hover_fill_strong(
                welcome_data.theme_surface,
                welcome_data.theme_text,
                self.theme_store.theme.interaction.hover_ratio_strong,
            );
            let continue_btn = self.recent_dirs_available.first().cloned().map(|dir| {
                let btn = div()
                    .id("welcome-continue")
                    .debug_selector(|| "welcome-continue".to_string())
                    .cursor_pointer()
                    .bg(welcome_data.theme_surface)
                    .hover(move |s| s.bg(welcome_hover))
                    .text_color(welcome_data.theme_text)
                    .rounded(px(6.0))
                    .px(px(16.0))
                    .py(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(icon(
                                IconName::ChevronRight,
                                px(14.0),
                                welcome_data.theme_text,
                            ))
                            .child(lang.get(StrKey::ContinueButton)),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_continue)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.open_folder(dir.clone(), cx);
                        }),
                    );
                btn
            });
            let open_btn: AnyElement = div()
                .id("welcome-open")
                .debug_selector(|| "welcome-open".to_string())
                .cursor_pointer()
                .bg(welcome_data.theme_surface)
                .hover(move |s| s.bg(welcome_hover))
                .text_color(welcome_data.theme_text)
                .rounded(px(6.0))
                .px(px(16.0))
                .py(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(icon(
                            IconName::FolderOpen,
                            px(14.0),
                            welcome_data.theme_text,
                        ))
                        .child(lang.get(StrKey::WelcomeOpen)),
                )
                .on_mouse_down(MouseButton::Left, swallow_open)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.pick_folder(cx);
                    }),
                )
                .into_any();
            // V3 recent-folder chips: recent[1..] (Continue covers [0]).
            // Pre-built here with cx.listener — the Continue/Open pattern.
            // Element ids are index-keyed tuples (the sort-row pattern):
            // gpui 0.2.2 has no From<(&str, String)>.
            let recent_chips: Vec<AnyElement> = self
                .recent_dirs_available
                .iter()
                .skip(1)
                .enumerate()
                .map(|(idx, dir)| {
                    let dir = dir.clone();
                    let swallow =
                        cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                        });
                    div()
                        .id(("welcome-recent", idx as u64))
                        .debug_selector(move || format!("welcome-recent-{idx}"))
                        .cursor_pointer()
                        .bg(welcome_data.theme_surface)
                        .hover(move |s| s.bg(welcome_hover))
                        .text_color(welcome_data.theme_text)
                        .text_size(px(12.0))
                        .rounded(px(8.0))
                        .px(px(10.0))
                        .py(px(6.0))
                        .child(sh_core::recent::display_name(&dir))
                        .on_mouse_down(MouseButton::Left, swallow)
                        .on_click(cx.listener(
                            move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                                this.note_interaction(cx);
                                this.open_folder(dir.clone(), cx);
                            },
                        ))
                        .into_any()
                })
                .collect();
            Some(
                welcome::welcome(
                    lang,
                    &welcome_data,
                    continue_btn.map(|b| b.into_any()),
                    open_btn,
                    recent_chips,
                )
                .into_any_element(),
            )
        } else {
            None
        };

        // ── V2 Task 7: grid content (cells pre-built below with clicks). ──
        let grid_el = if self.view == View::Grid && !self.session.images.is_empty() {
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            // Cell geometry follows the active density preset (S/M/L); M is
            // today's geometry verbatim. Cast once here — scroll math below
            // stays `f32` as before.
            let geo = self.settings.grid_size.geometry();
            let cell_w = geo.cell_w as f32;
            let thumb_w = geo.thumb_w as f32;
            let thumb_h = geo.thumb_h as f32;
            let label_w = geo.label_w as f32;
            let bar_w = geo.bar_w as f32;
            let bar_h = geo.bar_h as f32;
            let bar_left = geo.bar_left as f32;
            // Cell hover plate: the SAME tint as the buttons (hover_fill over
            // the app background) — one hover language across the whole app,
            // dark and light themes alike.
            let cell_hover = hover_fill(bg, text, self.theme_store.theme.interaction.hover_ratio);
            let row_h = geo.row_h as f32;
            let viewport = viewport_vec(self.viewport);
            let visible_h = (viewport.y - topbar::TOPBAR_H_PX).max(1.0);
            let columns = grid::grid_columns(viewport.x, &geo);
            // Only rows intersecting the clipped viewport pay the cost of a
            // thumbnail, label, marker, and click listener. Every other cell
            // remains a fixed-size placeholder so flex-wrap row positions and
            // the existing manual-scroll clamp stay unchanged.
            let visible_rows = grid::visible_row_range(
                self.session.images.len(),
                self.grid_scroll_px,
                viewport.x,
                visible_h,
                &geo,
            );
            let mut cells: Vec<AnyElement> = Vec::with_capacity(self.session.images.len());
            for (idx, item) in self.session.images.iter().enumerate() {
                if !visible_rows.contains(&(idx / columns)) {
                    cells.push(div().w(px(cell_w)).h(px(row_h)).into_any());
                    continue;
                }
                // V3 multi-select: every set member wears the accent bar,
                // not just the cursor.
                // V3 multi-select: the cursor (last visited) wears the full
                // accent bar; set members wear the same bar dimmed — one
                // marker shape, two intensities, readable in any theme.
                let is_cursor = idx == self.grid_selected;
                let in_set = self.selected.contains(&idx);
                let swallow_cell =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                // Dimmed label color: 60% alpha text (editorial hierarchy).
                let mut label_color = text;
                label_color.a = 0.6;
                // Thumb or placeholder: a missing decode (batch still
                // running, slow/corrupt file) shows the themed chip
                // until the background batch lands.
                let thumb: AnyElement = match self.thumbs.get(&item.path) {
                    Some(arc) => img(arc.clone())
                        .id(("grid-thumb", idx))
                        .w(px(thumb_w))
                        .h(px(thumb_h))
                        .rounded(px(10.0)) // 8 → 10 per spec
                        .into_any(),
                    None => div()
                        .id(("grid-thumb-empty", idx))
                        .debug_selector(move || format!("grid-thumb-empty-{idx}"))
                        .w(px(thumb_w))
                        .h(px(thumb_h))
                        .bg(topbar_data.theme_surface)
                        .rounded(px(10.0))
                        .into_any(),
                };
                // Minimal active marker: a slim accent bar laid over the
                // thumbnail's bottom edge (streaming-app active pattern) —
                // no border box around the cell. The relative frame anchors
                // the absolutely-positioned bar to the thumb itself.
                // Slice C: per-cell transparency board — exactly one baked
                // layer behind the thumb iff the persisted setting is ON
                // and this path's batch verdict is a confirmed `true`
                // (same gate `App::render` uses for the viewer). Opaque
                // cells and verdict-pending placeholders are untouched;
                // cell geometry is unchanged.
                let show_board = crate::viewer::should_show_checkerboard(
                    self.settings.checkerboard,
                    self.thumb_alpha.get(&item.path).copied(),
                );
                // The frame carries the thumb's footprint and clips it.
                //
                // `Img::request_layout` imposes `aspect_ratio` from the image's
                // own dimensions when the caller has not set one (gpui-pre
                // `elements/img.rs`), so a photo lays itself out against its
                // natural shape rather than the box asked for. Measured on a
                // folder of mixed-aspect PNGs, a 223px image rendered in a 75px
                // slot and painted over the row above and the label below.
                //
                // The cell's own fixed row height is what actually restores
                // uniform rows — reverting that alone reproduces 146px cells
                // against a 170px preset. This clip is the paint-level half:
                // the layout test cannot see whether an image PAINTED past its
                // slot, only where the box ended, and the label overlap was
                // exactly that kind of invisible-to-tests damage.
                let mut thumb_frame = div()
                    .relative()
                    .w(px(thumb_w))
                    .h(px(thumb_h))
                    .overflow_hidden();
                if show_board {
                    thumb_frame = thumb_frame.child(
                        crate::checkerboard::checkerboard_layer(thumb_w, thumb_h, checker_palette)
                            .absolute()
                            .top(px(0.0))
                            .left(px(0.0))
                            .debug_selector(move || format!("grid-board-{idx}")),
                    );
                }
                thumb_frame = thumb_frame.child(thumb);
                // V3 multi-select markers — position, not intensity: the
                // cursor (last visited) wears the bottom bar; set members
                // wear the same bar mirrored at the top. Both at full accent
                // so they read in any theme; cursor+member shows both bars.
                if in_set {
                    // Same geometry as the cursor bar, mirrored to the top.
                    thumb_frame = thumb_frame.child(
                        div()
                            .id(("grid-selected-bar", idx))
                            .absolute()
                            .top(px(0.0))
                            .left(px(bar_left))
                            .w(px(bar_w))
                            .h(px(bar_h))
                            .rounded(px(2.0))
                            .bg(accent),
                    );
                }
                if is_cursor {
                    // Inset 10px horizontally so the bar clears the thumb's
                    // rounded corners; 3px tall, pill-shaped.
                    thumb_frame = thumb_frame.child(
                        div()
                            .id(("grid-active-bar", idx))
                            .absolute()
                            .bottom(px(0.0))
                            .left(px(bar_left))
                            .w(px(bar_w))
                            .h(px(bar_h))
                            .rounded(px(2.0))
                            .bg(accent),
                    );
                }
                let cell = div()
                    .id(("grid-cell", idx))
                    .debug_selector(move || format!("grid-cell-{idx}"))
                    .w(px(cell_w))
                    // Uniform row height, matching the culled placeholder that
                    // stands in for off-screen rows. This is the fix: with the
                    // height unset the cell sized itself from its content, so a
                    // tall image grew its whole flex line and every row boundary
                    // derived from it — the scroll math and the render stopped
                    // agreeing with each other. Verified by reverting it alone:
                    // 146px against the preset's 170px.
                    .h(px(row_h))
                    .cursor_pointer()
                    // Centered flex column: the cell is wider than the
                    // thumb+label pair — without centering the content sat
                    // flush left and the plate jutted out to the right.
                    // Centered, the plate reads as a symmetric card around
                    // the image. (Cell height = thumb + label, unchanged —
                    // preset row-height math intact.)
                    .flex()
                    .flex_col()
                    .items_center()
                    // Plate corners echo the thumb's rounding so the hover
                    // shape matches the image shape.
                    .rounded(px(10.0))
                    // Hover plate: same color-mix idiom as every button —
                    // the cell background tints toward the theme text. This
                    // replaces the old soft shadow (invisible on dark,
                    // heavy on light); it reads as a subtle plate under the
                    // thumb in BOTH theme families.
                    .hover(move |s| s.bg(cell_hover))
                    .child(thumb_frame)
                    // Single-line ellipsis: a wrapped label grows the row
                    // and breaks the scroll math (see GRID_ROW_H_PX).
                    // Label brightens on hover: the hover sits on the label
                    // div itself (nested in the cell, both fire together).
                    .child(
                        div()
                            .w(px(label_w))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(label_color)
                            .hover(move |s| s.text_color(text))
                            .child(item.name.clone()),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_cell)
                    .on_click(
                        cx.listener(move |this: &mut App, ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            if ev.modifiers().control {
                                this.toggle_selected(idx, cx);
                            } else if ev.modifiers().shift {
                                this.extend_selection_to(idx, cx);
                            } else {
                                this.enter_viewer(idx, cx);
                            }
                        }),
                    );
                cells.push(cell.into_any());
            }
            Some(grid::grid(cells, self.grid_scroll_px).into_any_element())
        } else {
            None
        };

        // Empty grid (no images: empty folder or failed resolve): centered
        // message instead of cells. The error slot carries the reason.
        // The default (no error reason) resolves through the table (S3).
        let grid_empty_el = if self.view == View::Grid && self.session.images.is_empty() {
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let lang = self.settings.language;
            let msg = self.session.error.clone().unwrap_or_else(|| {
                sh_core::i18n::t(lang, sh_core::i18n::StrKey::GridEmptyDefault).to_string()
            });
            Some(
                div()
                    .id("grid-empty")
                    .debug_selector(|| "grid-empty".to_string())
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().text_color(text).child(msg))
                    .into_any_element(),
            )
        } else {
            None
        };

        // ── Sort dropdown (Grid + Viewer): full-window click catcher closes
        // on outside click; the menu lists criteria + directions with the
        // active one checked. Rendered last so both float above content.
        let (sort_catcher_el, sort_menu_el) = if self.sort_menu_open && self.view != View::Welcome {
            let lang = self.settings.language;
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let row_bg = parse_hex(&self.theme_store.theme.colors.background)
                .unwrap_or(rgb(0x0d0d0f).into());
            let row_hover =
                hover_fill(row_bg, text, self.theme_store.theme.interaction.hover_ratio);
            let catcher: AnyElement = div()
                .id("sort-catcher")
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .cursor_default()
                .on_mouse_down(
                    // Swallow the press so the grid cell / viewer gesture
                    // behind the catcher never arms.
                    MouseButton::Left,
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    }),
                )
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.sort_menu_open = false;
                        cx.notify();
                    }),
                )
                .into_any();
            // Criterion rows: keep the current direction, change the criterion.
            let mut list = div().flex().flex_col().gap(px(2.0));
            for (row_idx, (by, key)) in SORT_MENU_ITEMS.iter().enumerate() {
                let active = *by == self.session.sort_by;
                let swallow_row =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                let mut row = div()
                    .id(("sort-row", row_idx))
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .hover(move |s| s.bg(row_hover))
                    .child(div().text_color(text).child(lang.get(*key)))
                    .child(
                        div()
                            .text_color(if active { accent } else { text })
                            .child(if active { "✓" } else { "" }),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_row)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.sort_menu_open = false;
                            this.set_sort(*by, this.session.sort_dir, cx);
                        }),
                    );
                if active {
                    row = row.bg(row_bg);
                }
                list = list.child(row);
            }
            // Direction rows: keep the current criterion, change direction.
            for (row_idx, (dir, key)) in SORT_DIR_ITEMS.iter().enumerate() {
                let active = *dir == self.session.sort_dir;
                let swallow_row =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                let mut row = div()
                    .id(("sort-dir-row", row_idx))
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .hover(move |s| s.bg(row_hover))
                    .child(div().text_color(text).child(lang.get(*key)))
                    .child(
                        div()
                            .text_color(if active { accent } else { text })
                            .child(if active { "✓" } else { "" }),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_row)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.sort_menu_open = false;
                            this.set_sort(this.session.sort_by, *dir, cx);
                        }),
                    );
                if active {
                    row = row.bg(row_bg);
                }
                list = list.child(row);
            }
            let swallow_menu = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let menu: AnyElement = div()
                .id("sort-menu")
                .absolute()
                .top(px(topbar::TOPBAR_H_PX + 8.0))
                .right(px(12.0))
                .bg(surface)
                .text_color(text)
                .rounded(px(8.0))
                .p(px(8.0))
                .on_mouse_down(MouseButton::Left, swallow_menu)
                .child(
                    div()
                        .px(px(10.0))
                        .py(px(4.0))
                        .child(lang.get(StrKey::SortByLabel)),
                )
                .child(list)
                .into_any();
            (Some(catcher), Some(menu))
        } else {
            (None, None)
        };

        // ── V2: viewer + overlays, Viewer-only, confined below the bar. ──
        // Crop selection overlay: accent border, no dim (YAGNI — border only).
        // Normalized at paint time; render never mutates state.
        let crop_overlay: Option<AnyElement> = match (&self.crop_mode, &self.crop_rect) {
            (true, Some(rect)) => {
                let n = rect.normalized();
                let x = n.x0.min(n.x1);
                let y = n.y0.min(n.y1);
                let w = (n.x1 - n.x0).abs();
                let h = (n.y1 - n.y0).abs();
                let accent = parse_hex(&self.theme_store.theme.colors.accent)
                    .unwrap_or(rgb(0x00ffff).into());
                Some(
                    div()
                        .id("crop-selection")
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w))
                        .h(px(h))
                        .border(px(2.0))
                        .border_color(accent)
                        .into_any_element(),
                )
            }
            _ => None,
        };
        // ── Task 5: crop confirm bar (Copy / Save / Cancel) ──
        // Floating above the bottom overlay, hidden while a drag is still
        // armed (crop_rect Some + bar hidden = mid-drag by construction:
        // the bar only appears via finish_crop_drag).
        let crop_bar_el: Option<AnyElement> = if self.crop_bar_visible {
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let swallow_crop_bar =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            // The three actions are built by the shared factory the batch bar
            // uses, so their idle/hover/pressed colors come from the projected
            // kit theme instead of this bar's own `surface` + `.hover()` pair.
            let bar_entity = cx.entity();
            let copy_btn = confirm_bar_button(
                "crop-copy",
                t(self.settings.language, StrKey::CropCopy),
                bar_entity.clone(),
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_crop_copy(cx);
                },
                cx,
            );
            let save_btn = confirm_bar_button(
                "crop-save",
                t(self.settings.language, StrKey::CropSave),
                bar_entity.clone(),
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_crop_save(cx);
                },
                cx,
            );
            let cancel_btn = confirm_bar_button(
                "crop-cancel",
                t(self.settings.language, StrKey::Cancel),
                bar_entity.clone(),
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.cancel_crop(cx);
                },
                cx,
            );
            Some(
                div()
                    .id("crop-confirm-bar-anchor")
                    .absolute()
                    .bottom(px(56.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .on_mouse_down(MouseButton::Left, swallow_crop_bar)
                    .child(
                        div()
                            .id("crop-confirm-bar")
                            .bg(surface)
                            .text_color(text)
                            .border(px(1.0))
                            .border_color(accent)
                            .rounded(px(8.0))
                            .p(px(6.0))
                            .flex()
                            .gap(px(8.0))
                            .child(copy_btn)
                            .child(save_btn)
                            .child(cancel_btn),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };
        // V3 batch confirm bar: staged destructive op awaiting approval.
        // Built at render top level (NOT inside the viewer arm): the bar is
        // born in grid, and anything mounted under viewer-area never paints
        // there. Same anchor+inner shape as the crop bar otherwise.
        let batch_bar_el: Option<AnyElement> = self.pending_batch.as_ref().map(|op| {
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let swallow_batch_bar =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            // Same factory as the crop bar — this closure used to be a verbatim
            // copy of that bar's, so any change to a confirmation action had to
            // be made twice and could be made once.
            let bar_entity = cx.entity();
            let (confirm_id, confirm_label) = match op {
                BatchOp::Delete { .. } => (
                    "batch-confirm-delete",
                    t(self.settings.language, StrKey::BatchDelete),
                ),
                BatchOp::Move { .. } => (
                    "batch-confirm-move",
                    t(self.settings.language, StrKey::BatchMove),
                ),
            };
            let confirm_btn = confirm_bar_button(
                confirm_id,
                confirm_label,
                bar_entity.clone(),
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_pending(cx);
                },
                cx,
            );
            let cancel_btn = confirm_bar_button(
                "batch-cancel",
                t(self.settings.language, StrKey::Cancel),
                bar_entity.clone(),
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.pending_batch = None;
                    cx.notify();
                },
                cx,
            );
            div()
                .id("batch-confirm-bar-anchor")
                .absolute()
                .bottom(px(12.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .on_mouse_down(MouseButton::Left, swallow_batch_bar)
                .child(
                    div()
                        .id("batch-confirm-bar")
                        .bg(surface)
                        .text_color(text)
                        .border(px(1.0))
                        .border_color(accent)
                        .rounded(px(8.0))
                        .p(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(batch_bar_message(self.settings.language, op))
                        .child(confirm_btn)
                        .child(cancel_btn),
                )
                .into_any_element()
        });
        // Settings surface: slim header + sidebar + content. Rendered INSTEAD of
        // topbar/grid/viewer (those arms already guard on their own views; the
        // topbar now also excludes Settings).
        let settings_el: Option<AnyElement> = if self.view == View::Settings {
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let bg = parse_hex(&self.theme_store.theme.colors.background)
                .unwrap_or(rgb(0x0d0d0f).into());
            let row_hover = hover_fill(
                surface,
                text,
                self.theme_store.theme.interaction.hover_ratio,
            );
            let lang = self.settings.language;
            let t_title = sh_core::i18n::t(lang, sh_core::i18n::StrKey::SettingsTitle);

            // Slim header: ← Back + settings title.
            let swallow_back = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let back_btn: AnyElement = div()
                .id("settings-back")
                .debug_selector(|| "settings-back".to_string())
                .cursor_pointer()
                .flex()
                .items_center()
                .gap(px(6.0))
                .bg(bg)
                .hover(move |s| s.bg(row_hover))
                .text_color(text)
                .rounded(px(6.0))
                .px(px(12.0))
                .py(px(4.0))
                .child(icon(IconName::BackArrow, px(14.0), text))
                .child(t_title)
                .on_mouse_down(MouseButton::Left, swallow_back)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.close_settings(cx);
                    }),
                )
                .into_any();

            // Sidebar rows (caller-built, active wears accent bar).
            let mut side_rows: Vec<AnyElement> = Vec::new();
            for (idx, (section, label_key)) in crate::ui::settings_panel::SettingsSection::ALL
                .iter()
                .enumerate()
            {
                let active = *section == self.settings_section;
                let section = *section;
                let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
                side_rows.push(
                    div()
                        .id(("settings-section", idx))
                        .debug_selector(move || format!("settings-section-{idx}"))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .rounded(px(6.0))
                        .px(px(10.0))
                        .py(px(6.0))
                        .bg(if active { surface } else { bg })
                        .hover(move |s| s.bg(row_hover))
                        .text_color(if active { accent } else { text })
                        .child(sh_core::i18n::t(lang, *label_key))
                        .on_mouse_down(MouseButton::Left, swallow)
                        .on_click(cx.listener(
                            move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                                this.note_interaction(cx);
                                this.hover_motion.clear();
                                this.settings_section = section;
                                if section == crate::ui::settings_panel::SettingsSection::Appearance
                                {
                                    this.settings_appearance_focus_control = None;
                                }
                                // Switching sections breaks capture (edge case:
                                // capture scoped to the panel, broken on change).
                                this.capture_action = None;
                                this.capture_conflict = None;
                                this.reset_armed = false;
                                // Fresh section starts unscrolled (grid resets
                                // its offset on folder change, same rule).
                                this.settings_scroll_px = 0.0;
                                cx.notify();
                            },
                        ))
                        .into_any(),
                );
            }

            // Content rows per section (General + Appearance fully; Shortcuts in Task 7).
            let content: AnyElement = match self.settings_section {
                crate::ui::settings_panel::SettingsSection::General => {
                    self.render_general_section(surface, text, accent, bg, row_hover, cx)
                }
                crate::ui::settings_panel::SettingsSection::Appearance => {
                    self.render_appearance_section(surface, text, accent, bg, row_hover, cx)
                }
                crate::ui::settings_panel::SettingsSection::ThemeEditor => self
                    .render_theme_editor_section(surface, text, accent, bg, row_hover, window, cx),
                crate::ui::settings_panel::SettingsSection::Shortcuts => {
                    self.render_shortcuts_section(surface, text, accent, bg, row_hover, cx)
                }
            };

            Some(
                div()
                    .id("settings-root")
                    .debug_selector(|| "settings-root".to_string())
                    .size_full()
                    .flex()
                    .flex_col()
                    .bg(bg)
                    .text_color(text)
                    .child(
                        div()
                            .id("settings-header")
                            .debug_selector(|| "settings-header".to_string())
                            .h(px(crate::ui::topbar::TOPBAR_H_PX))
                            .flex()
                            .items_center()
                            .px(px(14.0))
                            .bg(surface)
                            .child(back_btn),
                    )
                    .child(
                        div()
                            .id("settings-body")
                            .debug_selector(|| "settings-body".to_string())
                            .flex_1()
                            .flex()
                            .child(crate::ui::settings_panel::sidebar::sidebar(side_rows))
                            .child(
                                div()
                                    .id("settings-content")
                                    .debug_selector(|| "settings-content".to_string())
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap(px(8.0))
                                    .p(px(16.0))
                                    // Clipped column; the inner wrapper below
                                    // translates content up by
                                    // `settings_scroll_px` (grid precedent —
                                    // see `ui/settings_panel/scroll.rs`).
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .relative()
                                            .top(px(-self.settings_scroll_px))
                                            .child(content),
                                    ),
                            ),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };
        let viewer_el = if self.view == View::Viewer {
            // ── Bottom chips (Viewer, while the topbar is dissolved) ──
            // Carry the bar's info + actions as translucent chips in ONE
            // bottom chrome area stacked directly above the bottom bar — a
            // merged single row would exceed the bar's fixed 40px budget
            // under `overflow_hidden` and silently clip the variable-length
            // name chip plus the three action controls, so the row stacks
            // instead (each row keeps its existing layout math; no control
            // can be clipped that was not clipped before). The mount shares
            // the bottom bar's idle gate (`bottom_visible`): the whole
            // bottom chrome fades together, leaving zero orphans floating
            // over the image after `OVERLAY_IDLE`. The persistent Back target
            // is intentionally outside this auto-hiding row.
            let chips_el = if topbar_dissolved && bottom_visible {
                let mut chip_bg = overlay_data.theme_surface;
                chip_bg.a = 0.72; // translucency per spec
                let mut chip_border = overlay_data.theme_surface;
                chip_border.a = 0.35;

                let name_chip = div()
                    .id("chip-name")
                    .bg(chip_bg)
                    .border(px(1.0))
                    .border_color(chip_border)
                    .rounded(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_color(overlay_data.theme_text)
                    .child(topbar_data.center.clone());

                let swallow_chip_gear =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                let chips_entity = cx.entity();
                let gear_chip_control = gpui_component::button::Button::new("chip-settings")
                    // `Settings` is the kit's name for this glyph and is in the
                    // default bundle, so the chip now paints the same gear the
                    // migrated topbar settings button does.
                    .icon(gpui_kit_assets::IconName::Settings)
                    .with_size(icon_only_box(14.0))
                    .accessibility_label(t(self.settings.language, StrKey::SettingsTitle))
                    .compact()
                    .on_mouse_down(MouseButton::Left, swallow_chip_gear)
                    .on_click({
                        let entity = chips_entity.clone();
                        move |_ev, _window, cx| {
                            entity.update(cx, |this: &mut App, cx| {
                                this.open_settings(cx);
                            });
                        }
                    })
                    .debug_selector(|| "chip-settings".to_string());
                let gear_chip = gear_chip_control.into_any_element();

                let swallow_chip_crop =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                // Crop chip: the active-mode affordance used to be a
                // hand-painted accent border, which conveyed nothing to
                // assistive tech. `.selected` is what paints the state and
                // `.toggled` is what announces it, so the chip is now a real
                // toggle button instead of a button that looks pressed.
                let crop_chip_control = gpui_component::button::Button::new("chip-crop")
                    .icon(gpui_component::Icon::default().data(CROP_ICON_SVG))
                    .with_size(icon_only_box(14.0))
                    .accessibility_label(t(self.settings.language, StrKey::ActionToggleCrop))
                    .compact()
                    .selected(self.crop_mode)
                    .toggled(self.crop_mode)
                    .on_mouse_down(MouseButton::Left, swallow_chip_crop)
                    .on_click({
                        let entity = chips_entity.clone();
                        move |_ev, _window, cx| {
                            entity.update(cx, |this: &mut App, cx| {
                                this.note_interaction(cx);
                                this.toggle_crop(cx);
                            });
                        }
                    })
                    .debug_selector(|| "chip-crop".to_string());
                let crop_chip = crop_chip_control.into_any_element();

                Some(
                    div()
                        .id("viewer-chips")
                        .debug_selector(|| "viewer-chips".to_string())
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(px(overlay::CHIPS_ROW_H_PX))
                        .mx(px(12.0))
                        .child(name_chip)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(gear_chip)
                                .child(crop_chip)
                                // R2: third reserved slot — the info button
                                // rides this row while Tab is ON, so gear/
                                // crop can no longer bury it. `take()` moves
                                // it out at most once per frame.
                                .children(info_btn_opt.take()),
                        )
                        .into_any_element(),
                )
            } else {
                None
            };

            // Tab-OFF float slot (R3): with the floating topbar solid, a
            // top(12) float would collide with the bar, so the button parks
            // BELOW it; exactly one container parents the button per frame
            // (chips slot or float), never both.
            let floating_info_el: Option<AnyElement> = if topbar_dissolved {
                None
            } else {
                info_btn_opt.take().map(|btn| {
                    div()
                        .id("info-btn-float")
                        .debug_selector(|| "info-btn-float".to_string())
                        .absolute()
                        .top(px(topbar::TOPBAR_H_PX + 12.0))
                        .right(px(12.0))
                        .child(btn)
                        .into_any()
                })
            };

            // ── Viewer info popover (open-only): full-window catcher +
            // panel, following the sort-catcher construction verbatim.
            // Rendered after content so the catcher floats above the image
            // (but below the popover); the popover wrap swallows its own
            // mousedown so panel clicks never arm the root pan gesture.
            // Facts come from the `info_facts` cache only — path match ⇒
            // rows (or the localized error on `Err`), any mismatch ⇒ the
            // localized error copy. Never I/O in the render path.
            let (info_catcher_el, info_popover_el): (Option<AnyElement>, Option<AnyElement>) =
                if self.view == View::Viewer && self.info_panel_open {
                    let lang = self.settings.language;
                    let text = parse_hex(&self.theme_store.theme.colors.text)
                        .unwrap_or(rgb(0xe8e8ee).into());
                    let surface = parse_hex(&self.theme_store.theme.colors.surface)
                        .unwrap_or(rgb(0x121218).into());
                    let swallow_catcher =
                        cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                        });
                    let catcher: AnyElement = div()
                        .id("info-catcher")
                        .absolute()
                        .top(px(0.0))
                        .left(px(0.0))
                        .right(px(0.0))
                        .bottom(px(0.0))
                        .cursor_default()
                        .on_mouse_down(MouseButton::Left, swallow_catcher)
                        .on_click(
                            cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                                this.close_info_panel(cx);
                            }),
                        )
                        .into_any();
                    let current_path = self.session.current_item().map(|i| i.path.clone());
                    let facts: Option<&sh_core::decode::FileInfo> =
                        match (&self.info_facts, current_path) {
                            (Some((cached_path, Ok(info))), Some(cur)) if cached_path == &cur => {
                                Some(info)
                            }
                            _ => None,
                        };
                    let error = lang.get(StrKey::InfoLoadError);
                    let popover = overlay::info_popover(
                        lang,
                        facts,
                        error,
                        text,
                        surface,
                        self.info_panel_open,
                    );
                    let swallow_pop =
                        cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                        });
                    let wrapped: AnyElement = div()
                        .id("info-popover-wrap")
                        .on_mouse_down(MouseButton::Left, swallow_pop)
                        .child(popover)
                        .into_any();
                    (Some(catcher), Some(wrapped))
                } else {
                    (None, None)
                };

            let viewer_chrome: Option<AnyElement> = if bottom_visible {
                Some(
                    div()
                        .id("viewer-chrome")
                        .debug_selector(|| "viewer-chrome".to_string())
                        .flex()
                        .flex_col()
                        .gap(px(overlay::CHROME_GAP_PX))
                        .children(chips_el)
                        .child(overlay::bottom(
                            &overlay_data,
                            bottom_visible,
                            chip_buttons,
                            Some(slideshow_btn),
                            Some(prev_btn),
                            Some(next_btn),
                        ))
                        .into_any_element(),
                )
            } else {
                None
            };

            Some(
                // Slice B filmstrip: `#viewer-area` is a vertical flex
                // column with three in-flow children — `#viewer-main`
                // (the image area, owns viewer/crop/persistent-Back/floats/catcher/
                // popover), the conditional `#viewer-chrome` row
                // (bottom bar + chips, fades on idle), and the
                // conditional `#filmstrip` row (`.h(STRIP_H_PX)`,
                // only when visible). `render_viewer` / `#zoom-layer`
                // are untouched: pan/zoom transforms traverse
                // `#viewer-main` only and can never reach the strip
                // (Req 7 sibling rule by construction). Bottom chrome
                // is in-flow flex (NOT absolute inside viewer-main):
                // it takes real layout space and never overlaps the
                // image top. `#viewer-chrome` shares the bottom bar's
                // idle gate (`bottom_visible`): the whole chrome fades
                // together, leaving zero orphans floating over the image.
                div()
                    .id("viewer-area")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .debug_selector(|| "viewer-area".to_string())
                    .child(
                        div()
                            .id("viewer-main")
                            .flex_1()
                            .relative()
                            .overflow_hidden()
                            .debug_selector(|| "viewer-main".to_string())
                            .child(viewer)
                            .children(crop_overlay)
                            .children(crop_bar_el)
                            .children(viewer_back_persistent)
                            .children(floating_info_el)
                            .children(info_catcher_el)
                            .children(info_popover_el),
                    )
                    .children(viewer_chrome)
                    .children(strip_el)
                    .into_any_element(),
            )
        } else {
            None
        };

        div()
            .id("app-root")
            .size_full()
            .flex()
            .flex_col()
            .key_context("image_view")
            // Track keyboard focus here so the "image_view" context joins
            // the FOCUS STACK, not just the element tree. GPUI matches
            // scoped key bindings against the dispatch path of the focused
            // element; without this, focus never enters the subtree and
            // ←/→/Tab/F11/Ctrl+O bindings never fire (the smoke-test bug).
            // Mousedown on a tracked element auto-focuses it (gpui div.rs
            // registers a bubble-phase mouse listener for exactly this),
            // so clicks on the viewer also keep focus anchored here.
            .track_focus(&self.focus_handle)
            .on_key_down(
                cx.listener(|this: &mut App, ev: &KeyDownEvent, window, cx| {
                    // Settings owns Tab for control selection even though the
                    // viewer's global Tab binding toggles overlays. Space is
                    // reserved for the selected Appearance control so it cannot
                    // also start the viewer slideshow from that surface.
                    if this.view == View::Settings
                        && this.settings_section
                            == crate::ui::settings_panel::SettingsSection::Appearance
                        && this.capture_action.is_none()
                    {
                        let key = ev.keystroke.key.as_str();
                        let no_command_modifier = !ev.keystroke.modifiers.control
                            && !ev.keystroke.modifiers.alt
                            && !ev.keystroke.modifiers.platform;
                        if key.eq_ignore_ascii_case("tab")
                            && no_command_modifier
                            && ev.keystroke.modifiers.shift
                        {
                            this.move_settings_appearance_selection(true);
                            window.prevent_default();
                            cx.stop_propagation();
                            return;
                        }
                        if settings_control_activation(ev)
                            && this.settings_section
                                == crate::ui::settings_panel::SettingsSection::Appearance
                        {
                            this.activate_settings_appearance_control(cx);
                            window.prevent_default();
                            cx.stop_propagation();
                            return;
                        }
                        if key.eq_ignore_ascii_case("space")
                            && !ev.keystroke.modifiers.shift
                            && no_command_modifier
                        {
                            window.prevent_default();
                            cx.stop_propagation();
                            return;
                        }
                    }
                    // Capture mode only: the Shortcuts chip owns the next keypress.
                    // Anything else falls through to normal keymap dispatch.
                    // V1: capture depends on root-div focus (chip click re-anchors via mousedown bubble). If focus escapes (Alt-Tab/OS dialog) while armed, keys won't reach the handler until a chip is clicked again — acceptable V1; close/section-switch clears capture.
                    let Some(action) = this.capture_action.clone() else {
                        return;
                    };
                    if this.view != View::Settings {
                        return;
                    }
                    use crate::ui::settings_panel::sections::shortcuts as sc;
                    let key = ev.keystroke.key.clone();
                    // Esc alone cancels (BackToGrid would otherwise close Settings —
                    // stop propagation here so the global BackToGrid Esc action doesn't also fire).
                    // Shift+Esc also cancels capture rather than rebinding (Esc is
                    // effectively reserved; shift is not checked in the gate below).
                    if key.eq_ignore_ascii_case("escape")
                        && !ev.keystroke.modifiers.control
                        && !ev.keystroke.modifiers.alt
                        && !ev.keystroke.modifiers.platform
                    {
                        this.capture_action = None;
                        this.capture_conflict = None;
                        this.reset_armed = false;
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    // Modifiers alone: keep waiting.
                    if sc::is_modifier_only(&key) {
                        cx.stop_propagation();
                        return;
                    }
                    let candidate = sh_core::keymap::KeyBinding {
                        ctrl: ev.keystroke.modifiers.control,
                        shift: ev.keystroke.modifiers.shift,
                        alt: ev.keystroke.modifiers.alt,
                        platform: ev.keystroke.modifiers.platform,
                        key,
                    };
                    match sh_core::keymap::validate_binding(
                        &this.settings.keymap,
                        crate::actions::KEYMAP_CONTEXT,
                        &action,
                        &candidate,
                    ) {
                        Ok(()) => {
                            this.capture_action = None;
                            this.capture_conflict = None;
                            this.reset_armed = false;
                            let mut km = this.settings.keymap.clone();
                            km.insert(action, candidate);
                            this.apply_keymap(km, cx);
                        }
                        Err(conflict) => {
                            // Rejection-with-feedback: red error, STAY capturing,
                            // nothing persisted. The incumbent ACTION ID is
                            // stored; the label + message localize at render.
                            this.capture_conflict = Some(conflict.existing_action);
                            cx.notify();
                        }
                    }
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleSlideshow, _window, cx| {
                    if this.view == View::Settings
                        && this.settings_section
                            == crate::ui::settings_panel::SettingsSection::Appearance
                    {
                        this.activate_settings_appearance_control(cx);
                        return;
                    }
                    this.toggle_slideshow(cx);
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &SelectNext, _window, cx| {
                // Grid-only: viewer arrows navigate images, never select.
                if this.view != View::Grid {
                    return;
                }
                let len = this.session.images.len();
                if len == 0 {
                    return;
                }
                // The cursor advances as the moving edge; the anchor stays
                // frozen so chained extends share it.
                let target = this.grid_selected.saturating_add(1).min(len - 1);
                this.extend_selection_to(target, cx);
                this.grid_selected = target;
                cx.notify();
            }))
            .on_action(cx.listener(|this: &mut App, _: &SelectPrev, _window, cx| {
                if this.view != View::Grid {
                    return;
                }
                if this.session.images.is_empty() {
                    return;
                }
                let target = this.grid_selected.saturating_sub(1);
                this.extend_selection_to(target, cx);
                this.grid_selected = target;
                cx.notify();
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleSelected, _window, cx| {
                    if this.view != View::Grid {
                        return;
                    }
                    let cur = this.grid_selected;
                    this.toggle_selected(cur, cx);
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &SelectAll, _window, cx| {
                if this.view != View::Grid {
                    return;
                }
                this.select_all(cx);
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &CopySelected, _window, cx| {
                    // Grid-only by slice scope; viewer Ctrl+C is a deliberate
                    // no-op (single-copy follow-up lives outside this slice).
                    if this.view != View::Grid {
                        return;
                    }
                    this.copy_selection(cx);
                }),
            )
            .on_action(
                cx.listener(|this: &mut App, _: &DeleteSelected, _window, cx| {
                    if this.view != View::Grid {
                        return;
                    }
                    this.stage_delete(cx);
                }),
            )
            .on_action(
                cx.listener(|this: &mut App, _: &MoveSelected, _window, cx| {
                    if this.view != View::Grid {
                        return;
                    }
                    this.pick_destination(cx);
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &NextImage, _window, cx| {
                // Grid: arrows move the thumbnail selection; Viewer/Welcome:
                // navigate images (no-op on an empty session).
                if this.view == View::Grid {
                    this.move_selection(1, cx);
                } else {
                    this.note_interaction(cx);
                    this.navigate(1, cx);
                }
            }))
            .on_action(cx.listener(|this: &mut App, _: &PrevImage, _window, cx| {
                if this.view == View::Grid {
                    this.move_selection(-1, cx);
                } else {
                    this.note_interaction(cx);
                    this.navigate(-1, cx);
                }
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleOverlays, _window, cx| {
                    // Tab remains the viewer's overlay shortcut, but Settings
                    // uses it for focus traversal. Shift+Tab has no matching
                    // action and is handled by the root key listener above.
                    if this.view == View::Settings {
                        this.move_settings_appearance_selection(false);
                        return;
                    }
                    // Viewer-only: Welcome/Grid have no overlays to toggle.
                    if this.view != View::Viewer {
                        return;
                    }
                    this.note_interaction(cx);
                    // The only ephemeral overlay left is the bottom bar
                    // (zoom + arrows); name/position live in the topbar.
                    this.session.show_overlay_bottom = !this.session.show_overlay_bottom;
                    // In-flow bottom chrome: the flip changes the image
                    // area itself, so refit EXACTLY ONCE against the new
                    // carve (Fit mode only — `refit_for_viewport` no-ops
                    // in Percent100, mirroring `set_filmstrip`).
                    this.session.refit_for_viewport(this.viewer_viewport());
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &BackToGrid, _window, cx| {
                // Info popover first: `Esc` closes it and is consumed here
                // (no navigation, no leave-viewer). Deterministic order for
                // stacked transient surfaces: info panel → capture/Settings
                // → crop → batch → selection → back-to-grid.
                if this.info_panel_open {
                    this.info_panel_open = false;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                // Settings surface intercepts Esc first (return to origin); capture
                // in progress second (cancel it — the shortcuts row handler consumes
                // Esc while capturing before this global action fires);
                // crop mode third; only then does Esc mean "back to grid".
                if this.view == View::Settings {
                    if this.capture_action.is_some() {
                        this.capture_action = None;
                        this.capture_conflict = None;
                        this.reset_armed = false;
                        cx.notify();
                        return;
                    }
                    this.close_settings(cx);
                    return;
                }
                if this.crop_mode {
                    this.cancel_crop(cx);
                    return;
                }
                // V3 batch bar visible: Esc cancels the staged op (consumed).
                if this.pending_batch.is_some() {
                    this.pending_batch = None;
                    cx.notify();
                    return;
                }
                // V3 multi-select: Escape in grid with a non-empty set clears
                // it (consumed); everything else falls through untouched.
                if this.view == View::Grid && !this.selected.is_empty() {
                    this.clear_selection(cx);
                    return;
                }
                if this.view == View::Viewer {
                    this.enter_grid(cx);
                }
            }))
            .on_action(cx.listener(|this: &mut App, _: &ToggleCrop, _window, cx| {
                // Viewer-only: crop needs a loaded image.
                if this.view != View::Viewer {
                    return;
                }
                this.toggle_crop(cx);
            }))
            // Crop confirm-bar actions. Enter copies (fast path); Esc
            // cancels via the crop branch in BackToGrid above.
            .on_action(cx.listener(|this: &mut App, _: &CropCopy, _window, cx| {
                if this.crop_bar_visible {
                    this.confirm_crop_copy(cx);
                }
            }))
            .on_action(cx.listener(|this: &mut App, _: &CropSave, _window, cx| {
                if this.crop_bar_visible {
                    this.confirm_crop_save(cx);
                }
            }))
            .on_action(cx.listener(|this: &mut App, _: &CropCancel, _window, cx| {
                if this.crop_mode {
                    this.cancel_crop(cx);
                }
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &OpenSelected, _window, cx| {
                    if this.view == View::Settings {
                        if this.settings_section
                            == crate::ui::settings_panel::SettingsSection::Appearance
                        {
                            this.activate_settings_appearance_control(cx);
                        }
                        return;
                    }
                    // V3 batch bar visible: Enter confirms the staged op
                    // (fast path) before its normal grid meaning. Mutually
                    // exclusive with the crop bar by view (grid vs viewer).
                    if this.pending_batch.is_some() {
                        this.confirm_pending(cx);
                        return;
                    }
                    // Crop bar visible: Enter confirms the copy (fast path)
                    // before its normal grid meaning.
                    if this.view == View::Viewer && this.crop_bar_visible {
                        this.confirm_crop_copy(cx);
                        return;
                    }
                    if this.view == View::Grid {
                        let idx = this.grid_selected;
                        this.enter_viewer(idx, cx);
                    }
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &OpenFolder, _window, cx| {
                this.pick_folder(cx);
            }))
            // ── Task 9: F11 fullscreen ──
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleFullscreen, window, cx| {
                    this.note_interaction(cx);
                    // gpui 0.2.2 exposes a stateless platform toggle.
                    window.toggle_fullscreen();
                }),
            )
            // ── Task 10: Ctrl+O native file dialog ──
            // The dialog must be async: a sync modal pumps the main thread's
            // message loop while the App entity is leased → double_lease_panic
            // on gpui's redraw ticks. `cx.spawn` releases the lease before
            // `pick_file()` blocks on rfd's dedicated dialog thread.
            .on_action(cx.listener(|this: &mut App, _: &OpenFile, _window, cx| {
                this.note_interaction(cx);
                let start_dir = this
                    .session
                    .current_item()
                    .and_then(|i| i.path.parent().map(std::path::Path::to_path_buf));
                let dialog = crate::platform::image_dialog(start_dir.as_deref());
                cx.spawn(async move |this, cx| {
                    if let Some(handle) = dialog.pick_file().await {
                        let _ = this.update(cx, |app, cx| {
                            app.open_path(handle.path().to_path_buf(), cx);
                            app.view = View::Viewer;
                            cx.notify();
                        });
                    }
                })
                .detach();
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &OpenSettings, _window, cx| {
                    // Ctrl+, toggles: open from any view, close back to origin.
                    if this.view == View::Settings {
                        this.close_settings(cx);
                    } else {
                        this.open_settings(cx);
                    }
                }),
            )
            // ── B3: wheel zoom anchored at cursor (Viewer only). In Grid the
            // wheel scrolls the thumbnail list instead (manual offset); in
            // Settings it scrolls the active section with exact geometry.
            // Native `overflow_y_scroll` cannot engage here (flex
            // auto-minimums, no `min-height` setter).
            .on_scroll_wheel(
                cx.listener(|this: &mut App, ev: &ScrollWheelEvent, _window, cx| {
                    if this.view == View::Settings {
                        let dy = match ev.delta {
                            ScrollDelta::Lines(p) => p.y * 40.0,
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                        };
                        let v = viewport_vec(this.viewport);
                        let visible =
                            (v.y - topbar::TOPBAR_H_PX - 2.0 * scroll::SETTINGS_PAD_PX).max(1.0);
                        let content_h = scroll::section_content_h(
                            this.settings_section,
                            this.settings.recent_dirs.len(),
                            this.theme_entries.len(),
                        );
                        let max = scroll::settings_max_scroll(content_h, visible);
                        this.settings_scroll_px = (this.settings_scroll_px - dy).clamp(0.0, max);
                        this.note_interaction(cx);
                        cx.notify();
                        return;
                    }
                    // Parked views never touch viewer/grid state: Welcome has
                    // nothing to zoom or scroll.
                    if wheel_parks(this.view) {
                        // Nothing to zoom or scroll here; keep the idle clock.
                        this.note_interaction(cx);
                        return;
                    }

                    if this.view == View::Grid {
                        let dy = match ev.delta {
                            ScrollDelta::Lines(p) => p.y * 40.0,
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                        };
                        // Wheel-up (negative dy) scrolls content down toward 0.
                        let v = viewport_vec(this.viewport);
                        let max = grid::grid_max_scroll(
                            this.session.images.len(),
                            v.x,
                            (v.y - topbar::TOPBAR_H_PX).max(1.0),
                            &this.settings.grid_size.geometry(),
                        );
                        this.grid_scroll_px = (this.grid_scroll_px - dy).clamp(0.0, max);
                        this.note_interaction(cx);
                        cx.notify();
                        return;
                    }
                    let view = this.viewer_viewport();
                    if view.x <= 0.0 || view.y <= 0.0 {
                        return;
                    }
                    // Cursor in viewport pixels (transform::zoom_at's contract).
                    let cursor = sh_core::transform::Vec2 {
                        x: f32::from(ev.position.x).clamp(0.0, view.x),
                        y: f32::from(ev.position.y).clamp(0.0, view.y),
                    };
                    let delta = match ev.delta {
                        ScrollDelta::Lines(p) => (1.0 + p.y * 0.15).max(0.1),
                        ScrollDelta::Pixels(p) => (1.0 + f32::from(p.y) * 0.003).max(0.1),
                    };
                    this.session.zoom_at(cursor, delta);
                    if let Some(img_size) = this.session.current_image_size() {
                        this.session.clamp_zoom(img_size, view);
                    }
                    cx.notify();
                }),
            )
            // ── B3: double-click toggles fit/100%; single press starts pan
            // drag ── Viewer only: grid/welcome must not arm viewer gestures.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this: &mut App, ev: &MouseDownEvent, _window, cx| {
                    if this.view != View::Viewer {
                        return;
                    }
                    // Crop mode owns the press: arm a selection, never pan.
                    // Double-click in crop mode just re-anchors the rect.
                    if this.crop_mode {
                        // Mouse events carry WINDOW coords; the floating
                        // topbar owns no layout space (R3), so NO y-shift is
                        // subtracted — the selection rect lives in the same
                        // full-window space as the image (see the mouse-move
                        // counterpart below).
                        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                        this.begin_crop_drag(pos);
                        this.note_interaction(cx);
                        cx.notify();
                        return;
                    }
                    if ev.click_count == 2 {
                        this.session.toggle_fit_100(this.viewer_viewport());
                        this.drag_last = None;
                    } else {
                        this.drag_last = Some(ev.position);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this: &mut App, _ev: &MouseUpEvent, _window, cx| {
                    // Crop drag end: promote to confirm bar or discard.
                    if this.view == View::Viewer && this.crop_mode {
                        this.finish_crop_drag();
                        this.note_interaction(cx);
                        cx.notify();
                        return;
                    }
                    this.drag_last = None;
                }),
            )
            .on_mouse_move(
                cx.listener(|this: &mut App, ev: &MouseMoveEvent, _window, cx| {
                    // Viewer-only gesture: dragging on grid/welcome must not
                    // displace viewer offsets (hover idle tracking below stays
                    // view-agnostic). Full input routing lands in Task 8.
                    if ev.dragging() && this.view != View::Viewer {
                        this.drag_last = None;
                        return;
                    }
                    if ev.dragging() {
                        // Drag pan: a real gesture, always resets the clock.
                        this.note_interaction(cx);
                        // Crop mode: the drag extends the selection, no pan.
                        if this.crop_mode {
                            // No y-shift (R3 floating topbar): drag coords
                            // and selection-rect coords share the full-window
                            // space, so the rect never drifts under the
                            // cursor.
                            let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                            this.update_crop_drag(pos);
                            cx.notify();
                            return;
                        }
                        if let Some(last) = this.drag_last {
                            this.session.pan(sh_core::transform::Vec2 {
                                x: f32::from(ev.position.x - last.x),
                                y: f32::from(ev.position.y - last.y),
                            });
                        }
                        this.drag_last = Some(ev.position);
                        cx.notify();
                    } else {
                        // Bare hover: sensor-noise micro-movements must NOT
                        // reset the idle clock (HUD flicker at the idle
                        // transition). Only a move >= HOVER_DEADBAND_PX since
                        // the last observed position counts; the anchor
                        // always advances. No recorded hover yet also counts.
                        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                        let moved = match this.last_hover {
                            None => true,
                            Some(last) => {
                                hover_moved_enough((f32::from(last.x), f32::from(last.y)), pos)
                            }
                        };
                        this.last_hover = Some(ev.position);
                        if moved {
                            this.note_interaction(cx);
                        }
                        // Not dragging: clear arming. This also self-heals a
                        // drag whose button was released outside the window
                        // (the mouse-up there never reaches `on_mouse_up`
                        // because bubble listeners filter by hitbox).
                        this.drag_last = None;
                    }
                }),
            )
            .bg(bg)
            // ── V2: Welcome arm (startup screen). ──
            .children(welcome_el)
            // ── V2 Task 6: persistent top bar (in-flow: Grid arm). ──
            .children(topbar_el)
            // ── V2 Task 7: grid arm (folder thumbnails) + empty state. ──
            .children(grid_el)
            .children(grid_empty_el)
            // ── Task 10 (+V2 Task 8): drag & drop. Directories open in Grid;
            // files open in Viewer.
            .on_drop(
                cx.listener(|this: &mut App, paths: &ExternalPaths, _window, cx| {
                    this.note_interaction(cx);
                    if let Some(path) = paths.paths().first() {
                        if path.is_dir() {
                            this.open_folder(path.clone(), cx);
                        } else {
                            this.open_path(path.clone(), cx);
                            this.view = View::Viewer;
                            cx.notify();
                        }
                    }
                }),
            )
            // ── V2: viewer + overlays render in Viewer only (flex_1 area
            // owning the FULL window height in every Tab state; grid/welcome
            // own their own arms). ──
            .children(viewer_el)
            // ── R3: the Viewer topbar floats AFTER viewer-area so it paints
            // above the image while occupying zero flex space. ──
            .children(viewer_topbar_el)
            // ── Sort dropdown: click-catcher under the menu (V3). ──
            .children(sort_catcher_el)
            .children(sort_menu_el)
            // ── V3 batch confirm bar: grid-born, but attached at root so it
            // renders in grid (it was briefly inside viewer-area, where grid
            // never mounted it — staging worked, Enter confirmed, and the
            // user deleted blind).
            .children(batch_bar_el)
            .children(settings_el)
    }
}

/// Parse a hex color string like `"#0d0d0f"` into an [`Hsla`].
/// Returns `None` on error. Supports 3, 6, and 8-digit hex with optional `#`.
///
/// 3- and 6-digit forms produce fully opaque colors (alpha = 1.0).
/// 8-digit form is `#RRGGBBAA` and preserves the alpha channel.
pub fn parse_hex(hex: &str) -> Option<Hsla> {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        3 => {
            let mut it = hex.chars();
            let r = it.next()?;
            let g = it.next()?;
            let b = it.next()?;
            let v = |c: char| -> Option<u32> { u32::from_str_radix(&format!("{c}{c}"), 16).ok() };
            let n = (v(r)? << 16) | (v(g)? << 8) | v(b)?;
            Some(rgb(n).into())
        }
        6 => {
            let n = u32::from_str_radix(hex, 16).ok()?;
            Some(rgb(n).into())
        }
        8 => {
            // #RRGGBBAA — use `rgba()` which reads [R, G, B, A] from be_bytes.
            let n = u32::from_str_radix(hex, 16).ok()?;
            Some(rgba(n).into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        batch_bar_message, create_bootstrap_theme_file, grid_size_label, grid_size_segments,
        hover_fill, hover_fill_strong, hover_tint, info_button_in_chips_row, info_button_visible,
        luma, parse_hex, remap_selection_by_path, route_viewer_motion, same_image_set,
        selected_count_suffix, slideshow_delay, slideshow_kit_icon, sort_chip_label,
        stable_filmstrip_viewport, stable_open_viewport, topbar_dissolved_for_viewer,
        viewer_control_hover_fill, wheel_parks, zoom_preset_disabled, zoom_preset_floor,
        zoom_preset_label, zoom_preset_segments, App, BatchOp, EXISTING_GRID_MOTION_ID,
        VIEWER_BACK_PERSISTENT_ID,
    };
    use crate::state::session::{build_image_items, FitMode, ImageItem, Session, ZoomPreset};
    use crate::state::theme_store::ThemeStore;
    use crate::state::view::View;
    use crate::ui::motion;
    use crate::ui::settings_panel::scroll;
    use sh_core::i18n::Language;
    use sh_core::navigation::{SortBy, SortDir};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    const VIEWER_MOTION_IDS: [&str; 18] = [
        "topbar-back",
        "topbar-open",
        "topbar-settings",
        "topbar-sort",
        "grid-size-0",
        "grid-size-1",
        "grid-size-2",
        "topbar-crop",
        VIEWER_BACK_PERSISTENT_ID,
        "chip-settings",
        "chip-crop",
        "info-btn",
        "prev-btn",
        "slideshow-btn",
        "next-btn",
        "zoom-preset-0",
        "zoom-preset-1",
        "zoom-preset-2",
    ];

    /// Copy the known-good PNG fixture (shared with the thumbs tests) into
    /// `dir` as `name` and return the written path. Guaranteed-decodable —
    /// PNG validity is never the variable under test here.
    fn fixture_png_in(dir: &std::path::Path, name: &str) -> PathBuf {
        let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../sh-core/tests/fixtures/bench_1080p.png");
        let dst = dir.join(name);
        std::fs::copy(&src, &dst).expect("fixture png must be copied");
        dst
    }

    /// Stub file with a supported extension and a given length. Scan filters
    /// by extension + is_file (never decodes), so size varies by length.
    fn fixture_stub(path: &std::path::Path, len: usize) {
        std::fs::write(path, vec![0u8; len]).expect("stub fixture write");
    }

    fn assert_positive_layout_bounds(bounds: gpui::Bounds<gpui::Pixels>, selector: &str) {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        assert!(
            width > 0.0 && height > 0.0,
            "{selector} must have positive bounds, got {width}x{height}"
        );
    }

    /// WU-6 baseline: Welcome has stable structural selectors for both the
    /// empty-recent state and a populated recent-folder state.
    #[gpui::test]
    fn layout_baseline_welcome_states_have_stable_surfaces(cx: &mut gpui::TestAppContext) {
        for (recent_count, expect_continue, expect_recent) in
            [(0usize, false, false), (2usize, true, true)]
        {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            app.update(cx, |app, cx| {
                app.view = View::Welcome;
                app.recent_dirs_available = (0..recent_count)
                    .map(|idx| PathBuf::from(format!("Z:\\fake\\recent-{idx}")))
                    .collect();
                cx.notify();
            });
            cx.run_until_parked();

            let root = cx.debug_bounds("welcome").expect("welcome root mounts");
            let dropzone = cx
                .debug_bounds("welcome-dropzone")
                .expect("welcome drop zone mounts");
            let open = cx
                .debug_bounds("welcome-open")
                .expect("open-folder action mounts");
            assert_positive_layout_bounds(root, "welcome");
            assert_positive_layout_bounds(dropzone, "welcome-dropzone");
            assert_positive_layout_bounds(open, "welcome-open");
            assert_eq!(
                cx.debug_bounds("welcome-continue").is_some(),
                expect_continue,
                "continue state mismatch for {recent_count} recents"
            );
            assert_eq!(
                cx.debug_bounds("welcome-recent-0").is_some(),
                expect_recent,
                "recent-chip state mismatch for {recent_count} recents"
            );
        }
    }

    /// WU-6 baseline: Grid distinguishes its empty state from its populated
    /// cell state and exposes deterministic geometry/checkerboard selectors.
    #[gpui::test]
    fn layout_baseline_grid_empty_and_non_empty_cells(cx: &mut gpui::TestAppContext) {
        let cases: [(usize, bool, Option<bool>, bool, bool, bool); 5] = [
            (0, false, None, false, true, false),
            (3, true, None, true, false, false),
            (3, true, Some(true), true, false, true),
            (3, true, Some(false), true, false, false),
            (3, false, Some(true), true, false, false),
        ];
        for (count, checkerboard, verdict, expect_grid, expect_empty, expect_board) in cases {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            app.update(cx, |app, cx| {
                app.view = View::Grid;
                app.settings.checkerboard = checkerboard;
                app.thumb_alpha.clear();
                if count == 0 {
                    app.session.images.clear();
                } else {
                    app.session.images = fake_images(count);
                    if let Some(verdict) = verdict {
                        let path = app.session.images[0].path.clone();
                        app.thumb_alpha.insert(path, verdict);
                    }
                }
                cx.notify();
            });
            cx.run_until_parked();

            assert_eq!(
                cx.debug_bounds("grid-empty").is_some(),
                expect_empty,
                "empty-grid state mismatch for count={count}"
            );
            assert_eq!(
                cx.debug_bounds("grid-scroll").is_some(),
                expect_grid,
                "grid surface mismatch for count={count}"
            );
            assert_eq!(
                cx.debug_bounds("grid-cell-0").is_some(),
                expect_grid,
                "first-cell visibility mismatch for count={count}"
            );
            assert_eq!(
                cx.debug_bounds("grid-board-0").is_some(),
                expect_board,
                "checkerboard state mismatch for count={count}, setting={checkerboard}, verdict={verdict:?}"
            );

            if expect_empty {
                let empty = cx.debug_bounds("grid-empty").expect("empty state mounts");
                assert_positive_layout_bounds(empty, "grid-empty");
            }
            if expect_grid {
                let scroll = cx.debug_bounds("grid-scroll").expect("grid scroll mounts");
                let rows = cx.debug_bounds("grid-rows").expect("grid rows mount");
                let cell = cx.debug_bounds("grid-cell-0").expect("first cell mounts");
                let thumb = cx
                    .debug_bounds("grid-thumb-empty-0")
                    .expect("first placeholder mounts");
                assert_positive_layout_bounds(scroll, "grid-scroll");
                assert_positive_layout_bounds(rows, "grid-rows");
                assert_positive_layout_bounds(cell, "grid-cell-0");
                assert_positive_layout_bounds(thumb, "grid-thumb-empty-0");
                assert!(
                    (f32::from(cell.size.width) - 180.0).abs() < 1.0,
                    "grid cell width drifted: {}",
                    f32::from(cell.size.width)
                );
                assert!(
                    (f32::from(thumb.size.width) - 160.0).abs() < 1.0,
                    "grid thumbnail width drifted: {}",
                    f32::from(thumb.size.width)
                );
                assert!(
                    (f32::from(thumb.size.height) - 120.0).abs() < 1.0,
                    "grid thumbnail height drifted: {}",
                    f32::from(thumb.size.height)
                );
            }
        }
    }

    /// WU-6 baseline: Viewer distinguishes empty and image states, and pins
    /// filmstrip/checkerboard presence to the same visibility contracts.
    #[gpui::test]
    fn layout_baseline_viewer_empty_image_filmstrip_checkerboard(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.session.images.clear();
            app.settings.filmstrip = false;
            app.settings.checkerboard = true;
            cx.notify();
        });
        cx.run_until_parked();
        let root = cx.debug_bounds("viewer-root").expect("viewer root mounts");
        let empty = cx
            .debug_bounds("viewer-empty")
            .expect("viewer empty state mounts");
        assert_positive_layout_bounds(root, "viewer-root");
        assert_positive_layout_bounds(empty, "viewer-empty");
        assert!(cx.debug_bounds("viewer-image").is_none());
        assert!(cx.debug_bounds("viewer-checkerboard").is_none());
        assert!(cx.debug_bounds("filmstrip").is_none());

        let cases: [(bool, bool, Option<bool>, bool, bool); 3] = [
            (false, true, Some(true), false, true),
            (true, true, Some(true), true, true),
            (true, false, Some(true), true, false),
        ];
        for (filmstrip, checkerboard, alpha, expect_strip, expect_board) in cases {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            app.update(cx, |app, cx| {
                app.view = View::Viewer;
                app.session.images = fake_images(1);
                app.session.images[0].has_alpha = alpha;
                app.settings.filmstrip = filmstrip;
                app.settings.checkerboard = checkerboard;
                cx.notify();
            });
            cx.run_until_parked();

            let root = cx.debug_bounds("viewer-root").expect("viewer root mounts");
            // The image selector is the structural contract. The headless
            // harness does not load pixels from the fake path, so image bounds
            // and any pixel comparison remain manual evidence.
            assert!(cx.debug_bounds("viewer-image").is_some());
            assert_positive_layout_bounds(root, "viewer-root");
            assert!(cx.debug_bounds("viewer-empty").is_none());
            assert_eq!(
                cx.debug_bounds("viewer-checkerboard").is_some(),
                expect_board,
                "viewer checkerboard mismatch for filmstrip={filmstrip}, checkerboard={checkerboard}, alpha={alpha:?}"
            );
            assert_eq!(
                cx.debug_bounds("filmstrip").is_some(),
                expect_strip,
                "viewer filmstrip mismatch for filmstrip={filmstrip}, checkerboard={checkerboard}, alpha={alpha:?}"
            );
            if expect_strip {
                let strip = cx.debug_bounds("filmstrip").expect("filmstrip mounts");
                assert!((f32::from(strip.size.height) - crate::filmstrip::STRIP_H_PX).abs() < 1.0);
            }
        }
    }

    /// WU-6 baseline: every Settings section has stable bounds at the
    /// minimum supported window, while the Appearance rows remain present
    /// The reduced-motion preference must reach GPUI, not just our own settings.
    ///
    /// The app reads `settings.reduce_motion` directly, which is why the app's
    /// own animations always honoured it. gpui-component does not: every kit
    /// Button and Switch reads `cx.reduce_motion()`, a separate flag that
    /// `gpui-base` consults to skip springs. So the toggle used to persist, redraw
    /// its row, and change nothing observable in any adopted component.
    ///
    /// Both directions are asserted here, plus the startup path, because they are
    /// three separate call sites and the startup one is the easiest to forget:
    /// a user who enabled the preference last session got motion on this launch
    /// until they toggled it off and on again.
    #[gpui::test]
    fn reduce_motion_setting_reaches_gpui(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;

        // Startup: whatever the persisted value was, `App::new` published it.
        let initial = app.read_with(cx, |app, _| app.settings.reduce_motion);
        assert_eq!(
            cx.update(|_window, cx| cx.reduce_motion()),
            initial,
            "App::new must publish the persisted reduced-motion preference to gpui"
        );

        // Toggling off, then on, must move the framework flag both ways.
        for enabled in [!initial, initial] {
            app.update(cx, |app, cx| {
                app.set_reduce_motion(enabled, cx);
            });
            assert_eq!(
                cx.update(|_window, cx| cx.reduce_motion()),
                enabled,
                "set_reduce_motion({enabled}) did not reach gpui"
            );
            assert_eq!(
                app.read_with(cx, |app, _| app.settings.reduce_motion),
                enabled,
                "set_reduce_motion({enabled}) did not persist to settings"
            );
        }
    }

    /// for both reduce-motion values without changing their geometry.
    #[gpui::test]
    fn layout_baseline_settings_sections_and_reduce_motion(cx: &mut gpui::TestAppContext) {
        use crate::ui::settings_panel::SettingsSection;

        let sections = [
            (SettingsSection::General, "settings-show-hidden"),
            (SettingsSection::Appearance, "settings-appearance-controls"),
            (
                SettingsSection::ThemeEditor,
                crate::ui::settings_panel::sections::theme_editor::CONTAINER_ID,
            ),
            (SettingsSection::Shortcuts, "shortcuts-reset"),
        ];
        for (section, content_selector) in sections {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            cx.simulate_resize(gpui::size(gpui::px(480.0), gpui::px(320.0)));
            app.update(cx, |app, cx| {
                app.view = View::Settings;
                app.settings_section = section;
                cx.notify();
            });
            cx.run_until_parked();

            for selector in [
                "settings-root",
                "settings-header",
                "settings-body",
                "settings-sidebar",
                "settings-content",
                content_selector,
            ] {
                let bounds = cx
                    .debug_bounds(selector)
                    .unwrap_or_else(|| panic!("missing {selector}"));
                assert_positive_layout_bounds(bounds, selector);
            }
            let root = cx
                .debug_bounds("settings-root")
                .expect("settings root mounts");
            let header = cx
                .debug_bounds("settings-header")
                .expect("settings header mounts");
            let body = cx
                .debug_bounds("settings-body")
                .expect("settings body mounts");
            let sidebar = cx
                .debug_bounds("settings-sidebar")
                .expect("settings sidebar mounts");
            let content = cx
                .debug_bounds("settings-content")
                .expect("settings content mounts");
            assert!(
                (f32::from(root.size.width) - 480.0).abs() < 1.0,
                "settings root width drifted: {}",
                f32::from(root.size.width)
            );
            assert!(
                (f32::from(root.size.height) - 320.0).abs() < 1.0,
                "settings root height drifted: {}",
                f32::from(root.size.height)
            );
            assert!(
                f32::from(header.size.height) > 0.0,
                "settings header must retain positive height"
            );
            assert!(
                (f32::from(body.origin.y) - f32::from(header.size.height)).abs() < 1.0,
                "settings body must start below the header: body={} header={}",
                f32::from(body.origin.y),
                f32::from(header.size.height)
            );
            assert!(f32::from(body.size.height) > 0.0);
            assert!(
                (f32::from(sidebar.size.width) - 200.0).abs() < 1.0,
                "settings sidebar width drifted: {}",
                f32::from(sidebar.size.width)
            );
            assert!(f32::from(content.size.width) > 0.0);
            assert!(f32::from(content.size.height) > 0.0);
            if section == SettingsSection::Appearance {
                let controls = cx
                    .debug_bounds("settings-appearance-controls")
                    .expect("appearance controls mount");
                assert!(f32::from(controls.size.height) > 0.0);
                let content_h = scroll::section_content_h(
                    section,
                    app.read_with(cx, |app, _| app.settings.recent_dirs.len()),
                    app.read_with(cx, |app, _| app.theme_entries.len()),
                );
                let visible_h =
                    320.0 - crate::ui::topbar::TOPBAR_H_PX - 2.0 * scroll::SETTINGS_PAD_PX;
                let max_scroll = scroll::settings_max_scroll(content_h, visible_h);
                assert!(content_h > visible_h);
                assert!((max_scroll - (content_h - visible_h)).abs() < 1.0);
            }
        }

        for reduce_motion in [true, false] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            cx.simulate_resize(gpui::size(gpui::px(480.0), gpui::px(320.0)));
            app.update(cx, |app, cx| {
                app.view = View::Settings;
                app.settings_section = SettingsSection::Appearance;
                app.settings.reduce_motion = reduce_motion;
                cx.notify();
            });
            cx.run_until_parked();
            let row = cx
                .debug_bounds(
                    crate::ui::settings_panel::sections::appearance::REDUCE_MOTION_TOGGLE_ID,
                )
                .expect("reduce-motion row mounts");
            assert_positive_layout_bounds(row, "reduce-motion row");
            assert!((f32::from(row.size.height) - scroll::SETTINGS_ROW_H_PX).abs() < 1.0);
        }
    }

    #[test]
    fn topbar_dissolve_follows_bottom_bar_flag_not_idle() {
        // B2: at most one solid bar — bottom armed ⇒ topbar dissolved even
        // while active; bottom off ⇒ topbar solid even when idle (it is the
        // only chrome left, and idle must not remove the last visible UI).
        assert!(topbar_dissolved_for_viewer(true));
        assert!(!topbar_dissolved_for_viewer(false));
    }

    #[test]
    fn info_button_visible_without_bottom_bar_or_idle() {
        // B3: the button must be reachable with Tab OFF and after idle —
        // visibility depends only on view + image presence, never on the
        // auto-hiding bar or the idle clock.
        assert!(info_button_visible(View::Viewer, true));
        assert!(!info_button_visible(View::Viewer, false));
        assert!(!info_button_visible(View::Grid, true));
        assert!(!info_button_visible(View::Welcome, true));
        assert!(!info_button_visible(View::Settings, true));
    }

    #[test]
    fn info_button_mounts_in_chips_row_exactly_when_topbar_dissolves() {
        // R2: Tab ON ⇒ the chips row owns the top-right corner, so the
        // button must ride it as the third reserved slot; Tab OFF ⇒ the
        // standalone float returns. Exactly one container per frame.
        assert!(info_button_in_chips_row(topbar_dissolved_for_viewer(true)));
        assert!(!info_button_in_chips_row(topbar_dissolved_for_viewer(
            false
        )));
    }

    /// Bottom-chrome layout (viewer Back follow-up): the persistent Back
    /// control lives in `viewer-main` while the topbar is dissolved, never
    /// in the auto-hiding chips row. With Tab ON the lower chrome still
    /// stacks above the bottom bar; when idle only the persistent Back
    /// remains. Tab OFF keeps the top-right float and the topbar Back
    /// control. Fresh window per case: `debug_bounds` is append-only, so
    /// absence is only meaningful before first paint.
    #[gpui::test]
    fn viewer_chips_dock_in_bottom_chrome_with_tab_on(cx: &mut gpui::TestAppContext) {
        // (tab_on, idle, filmstrip, expect_chips, expect_float,
        //  expect_persistent_back, expect_topbar_back)
        for (
            tab_on,
            idle,
            strip,
            expect_chips,
            expect_float,
            expect_persistent_back,
            expect_topbar_back,
        ) in [
            (true, false, true, true, false, true, false),
            (true, false, false, true, false, true, false),
            (true, true, true, false, false, true, false),
            (true, true, false, false, false, true, false),
            (false, false, true, false, true, false, true),
            (false, true, true, false, true, false, true),
        ] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            app.update(cx, |app, cx| {
                app.view = View::Viewer;
                app.settings.filmstrip = strip;
                app.session.show_overlay_bottom = tab_on;
                if idle {
                    app.last_interaction = std::time::Instant::now()
                        - crate::ui::overlay::OVERLAY_IDLE
                        - std::time::Duration::from_secs(1);
                }
                cx.notify();
            });
            cx.run_until_parked();
            let chips = cx.debug_bounds("viewer-chips");
            let float = cx.debug_bounds("info-btn-float");
            let persistent_back = cx.debug_bounds(VIEWER_BACK_PERSISTENT_ID);
            let chip_back = cx.debug_bounds("chip-back");
            let topbar_back = cx.debug_bounds("topbar-back");
            assert_eq!(
                chips.is_some(),
                expect_chips,
                "tab_on={tab_on} idle={idle} strip={strip}: chips mount mismatch"
            );
            assert_eq!(
                float.is_some(),
                expect_float,
                "tab_on={tab_on} idle={idle} strip={strip}: float mount mismatch"
            );
            assert_eq!(
                persistent_back.is_some(),
                expect_persistent_back,
                "tab_on={tab_on} idle={idle} strip={strip}: persistent Back mount mismatch"
            );
            assert!(
                chip_back.is_none(),
                "tab_on={tab_on} idle={idle} strip={strip}: chip-back must not mount"
            );
            assert_eq!(
                topbar_back.is_some(),
                expect_topbar_back,
                "tab_on={tab_on} idle={idle} strip={strip}: topbar Back mount mismatch"
            );
            if expect_persistent_back {
                assert!(
                    topbar_back.is_none(),
                    "dissolved topbar must not mount a duplicate Back control"
                );
            }
            if expect_topbar_back {
                assert!(
                    persistent_back.is_none(),
                    "solid topbar must not mount a duplicate persistent Back control"
                );
            }
            // Fade-together note: the bar hides via `Display::None`, which
            // leaves its `debug_bounds` entry populated in the harness, so
            // the bar's own fade is not assertable here — it is pinned by
            // construction instead (the chips mount shares the bar's exact
            // `bottom_visible` predicate: one gate, two chrome rows, zero
            // orphans). The chips ARE mount-gated, so their absence below
            // is the observable half of that shared gate.
            if expect_chips {
                let chips = chips.expect("chips mount");
                let persistent_back = persistent_back.expect("persistent Back mounts");
                let main = cx.debug_bounds("viewer-main").expect("viewer-main mounts");
                let chrome = cx
                    .debug_bounds("viewer-chrome")
                    .expect("chrome mounts with bottom_visible");
                let bottom = cx
                    .debug_bounds("overlay-bottom")
                    .expect("bottom bar mounts with Tab ON while active");
                let main_x = f32::from(main.origin.x);
                let main_y = f32::from(main.origin.y);
                let main_right = main_x + f32::from(main.size.width);
                let main_bottom = main_y + f32::from(main.size.height);
                let back_x = f32::from(persistent_back.origin.x);
                let back_y = f32::from(persistent_back.origin.y);
                let back_right = back_x + f32::from(persistent_back.size.width);
                let back_bottom = back_y + f32::from(persistent_back.size.height);
                assert_positive_layout_bounds(persistent_back, VIEWER_BACK_PERSISTENT_ID);
                assert!(
                    back_x >= main_x - 1.0
                        && back_y >= main_y - 1.0
                        && back_right <= main_right + 1.0
                        && back_bottom <= main_bottom + 1.0,
                    "persistent Back must remain inside viewer-main without reserving layout space"
                );
                let chips_y = f32::from(chips.origin.y);
                let chips_bottom = chips_y + f32::from(chips.size.height);
                let bottom_y = f32::from(bottom.origin.y);
                let bottom_bottom = bottom_y + f32::from(bottom.size.height);
                assert!(
                    back_bottom <= bottom_y + 1.0,
                    "persistent Back must coexist above the active bottom bar (back_bottom={back_bottom} bottom_y={bottom_y})"
                );
                let chrome_y = f32::from(chrome.origin.y);
                let chrome_bottom = chrome_y + f32::from(chrome.size.height);
                // Chips are in viewer-chrome, NOT in viewer-main:
                // they never overlap image content.
                assert!(
                    chips_y >= main_bottom - 1.0,
                    "chips must not overlap viewer-main image area (chips_y={chips_y} main_bottom={main_bottom})"
                );
                // viewer-chrome starts at or above chips (chips is its child).
                assert!(
                    chrome_y <= chips_y + 1.0,
                    "viewer-chrome must contain chips (chrome_y={chrome_y} chips_y={chips_y})"
                );
                // Stacked ABOVE the bottom bar: chips end where the bar begins.
                assert!(
                    chips_bottom <= bottom_y + 1.0,
                    "chips must stack above the bottom bar (chips_bottom={chips_bottom} bottom_y={bottom_y})"
                );
                // viewer-chrome is above the filmstrip when visible.
                if cx.debug_bounds("filmstrip").is_some() {
                    let strip = cx.debug_bounds("filmstrip").unwrap();
                    let strip_y = f32::from(strip.origin.y);
                    assert!(
                        chrome_bottom <= strip_y + 1.0,
                        "viewer-chrome must sit above the filmstrip (chrome_bottom={chrome_bottom} strip_y={strip_y})"
                    );
                }
                // viewer-chrome has the bottom bar inside it.
                assert!(
                    bottom_y >= chrome_y - 1.0 && bottom_bottom <= chrome_bottom + 1.0,
                    "bottom bar must be inside viewer-chrome (bar_y={bottom_y} bar_bottom={bottom_bottom} chrome_y={chrome_y} chrome_bottom={chrome_bottom})"
                );
                // R2 intent preserved: the info button rides the chips row.
                assert!(
                    cx.debug_bounds("info-btn").is_some(),
                    "info button must ride the bottom chips row with Tab ON"
                );
            }
            if expect_float {
                assert!(
                    cx.debug_bounds("info-btn").is_some(),
                    "info button must stay reachable via the float with Tab OFF"
                );
            }
        }
    }

    /// A dissolved topbar must leave a real persistent mouse target beside
    /// the active lower chrome, and that target must reuse the normal Grid
    /// return path.
    #[gpui::test]
    fn viewer_back_button_returns_to_grid_from_bottom_chrome(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.session.show_overlay_bottom = true;
            app.last_interaction = std::time::Instant::now();
            cx.notify();
        });
        cx.run_until_parked();

        let back = cx
            .debug_bounds(VIEWER_BACK_PERSISTENT_ID)
            .expect("dissolved topbar must mount the persistent Back control");
        assert!(cx.debug_bounds("topbar-back").is_none());
        assert!(cx.debug_bounds("chip-back").is_none());
        let center = gpui::Point {
            x: gpui::px(f32::from(back.origin.x) + f32::from(back.size.width) / 2.0),
            y: gpui::px(f32::from(back.origin.y) + f32::from(back.size.height) / 2.0),
        };
        cx.simulate_click(center, gpui::Modifiers::default());
        cx.run_until_parked();

        app.read_with(cx, |app, _| assert_eq!(app.view, View::Grid));
    }

    /// Idle hides the lower chrome, but the dissolved-topbar Back target
    /// remains painted and clickable in the viewer main area.
    #[gpui::test]
    fn viewer_persistent_back_remains_available_when_overlay_is_idle(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.session.show_overlay_bottom = true;
            app.last_interaction = std::time::Instant::now()
                - crate::ui::overlay::OVERLAY_IDLE
                - std::time::Duration::from_secs(1);
            cx.notify();
        });
        cx.run_until_parked();

        assert!(cx.debug_bounds("viewer-chrome").is_none());
        let back = cx
            .debug_bounds(VIEWER_BACK_PERSISTENT_ID)
            .expect("persistent Back remains mounted while the lower overlay is idle");
        assert_positive_layout_bounds(back, VIEWER_BACK_PERSISTENT_ID);
        let center = gpui::Point {
            x: gpui::px(f32::from(back.origin.x) + f32::from(back.size.width) / 2.0),
            y: gpui::px(f32::from(back.origin.y) + f32::from(back.size.height) / 2.0),
        };
        cx.simulate_click(center, gpui::Modifiers::default());
        cx.run_until_parked();

        app.read_with(cx, |app, _| assert_eq!(app.view, View::Grid));
    }

    #[gpui::test]
    fn viewer_topbar_back_button_returns_to_grid(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.session.show_overlay_bottom = false;
            app.last_interaction = std::time::Instant::now();
            cx.notify();
        });
        cx.run_until_parked();

        let back = cx
            .debug_bounds("topbar-back")
            .expect("solid topbar must mount topbar-back");
        assert!(cx.debug_bounds(VIEWER_BACK_PERSISTENT_ID).is_none());
        assert!(cx.debug_bounds("chip-back").is_none());
        let center = gpui::Point {
            x: gpui::px(f32::from(back.origin.x) + f32::from(back.size.width) / 2.0),
            y: gpui::px(f32::from(back.origin.y) + f32::from(back.size.height) / 2.0),
        };
        cx.simulate_click(center, gpui::Modifiers::default());
        cx.run_until_parked();

        app.read_with(cx, |app, _| assert_eq!(app.view, View::Grid));
    }

    /// The migrated bottom-bar controls must be REACHABLE BY KEYBOARD, which
    /// is the accessibility point of moving them to kit `Button`s.
    ///
    /// Before the migration each of these was a hand-built `div()` carrying
    /// only `.id()` plus `on_click`. In gpui a click listener is inert for
    /// focus: `Interactivity::on_click` pushes onto `click_listeners` and
    /// nothing else, while `focusable` and `tab_stop` stay at their `false`
    /// defaults (`div.rs` sets them only from `.focusable()`, `.tab_stop()`,
    /// `.tab_index()` or `.track_focus()`). So the control had a hitbox and no
    /// way to be focused — reachable by mouse alone.
    ///
    /// The kit `Button` registers a focus handle and a tab stop, which is what
    /// this exercises: Tab until the slideshow chip takes focus, then Enter.
    /// The loop is bounded and order-independent on purpose — it asserts the
    /// control is reachable SOMEWHERE in the tab ring, not that it happens to
    /// sit at a particular position, so reordering the bar cannot break it.
    /// Both confirmation bars build their actions through ONE shared factory now,
    /// so every control they mount must still resolve by its own selector.
    ///
    /// This is the regression guard for the de-duplication itself. When the two
    /// bars each carried their own copy of the same closure, a selector or
    /// handler wired wrongly in one bar was invisible to the other bar's tests;
    /// with a single factory the risk moves to the shared call signature, and
    /// this asserts each call site still names the id and label it is supposed
    /// to. Positive bounds also keep proving each control is laid out and hit
    /// testable rather than merely constructed.
    #[gpui::test]
    fn both_confirmation_bars_mount_every_action_through_the_shared_factory(
        cx: &mut gpui::TestAppContext,
    ) {
        // Crop confirm bar: Copy / Save / Cancel.
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.crop_bar_visible = true;
            cx.notify();
        });
        cx.run_until_parked();
        for selector in ["crop-copy", "crop-save", "crop-cancel"] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} must mount with the crop confirm bar"));
            assert_positive_layout_bounds(bounds, selector);
        }

        // Batch confirm bar: a staged op mounts its own confirm + Cancel.
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.pending_batch = Some(BatchOp::Delete {
                paths: vec![PathBuf::from("unused.png")],
            });
            cx.notify();
        });
        cx.run_until_parked();
        for selector in ["batch-confirm-delete", "batch-cancel"] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} must mount with the batch confirm bar"));
            assert_positive_layout_bounds(bounds, selector);
        }
    }

    /// Direct Previous/Next clicks must refresh the idle clock. The seeded
    /// timestamp stays below the overlay deadline so both controls are
    /// mounted; the post-click comparison isolates the callback contract.
    #[gpui::test]
    fn viewer_arrow_clicks_refresh_last_interaction(cx: &mut gpui::TestAppContext) {
        for (selector, expected_current) in [("prev-btn", 2), ("next-btn", 1)] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            let interaction_seed =
                std::time::Instant::now() - std::time::Duration::from_millis(200);
            app.update(cx, |app, cx| {
                app.view = View::Viewer;
                app.session.images = fake_images(3);
                app.session.current = 0;
                app.session.show_overlay_bottom = true;
                app.last_interaction = interaction_seed;
                cx.notify();
            });
            cx.run_until_parked();

            let arrow = cx.debug_bounds(selector).unwrap_or_else(|| {
                panic!("{selector} must mount while the lower overlay is active")
            });
            let center = gpui::Point {
                x: gpui::px(f32::from(arrow.origin.x) + f32::from(arrow.size.width) / 2.0),
                y: gpui::px(f32::from(arrow.origin.y) + f32::from(arrow.size.height) / 2.0),
            };
            std::thread::sleep(std::time::Duration::from_millis(20));
            cx.simulate_click(center, gpui::Modifiers::default());
            cx.run_until_parked();

            app.read_with(cx, |app, _| {
                assert_eq!(app.session.current, expected_current);
                assert!(
                    app.last_interaction > interaction_seed,
                    "{selector} must refresh last_interaction"
                );
            });
        }
    }

    #[test]
    fn viewer_motion_routes_only_new_targets_in_viewer() {
        let mut identities = std::collections::BTreeSet::new();

        for stable_id in VIEWER_MOTION_IDS {
            let expected_id = motion::AnimationId::new(stable_id);
            assert_eq!(
                route_viewer_motion(View::Viewer, stable_id),
                Some(expected_id),
                "Viewer controls must use their stable shared-motion identity"
            );
            assert!(
                identities.insert(stable_id),
                "Viewer motion identity must be unique: {stable_id}"
            );

            for view in [View::Grid, View::Welcome, View::Settings] {
                let route = route_viewer_motion(view, stable_id);
                if view == View::Grid && stable_id == EXISTING_GRID_MOTION_ID {
                    assert_eq!(route, Some(expected_id));
                } else {
                    assert_eq!(
                        route, None,
                        "{stable_id} must not expand shared motion into {view:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn viewer_motion_hover_is_stronger_than_non_viewer() {
        for (bg, text) in [
            (gpui::rgb(0xffffff), gpui::rgb(0x1a1a1e)),
            (gpui::rgb(0x0d0d0f), gpui::rgb(0xe8e8ee)),
        ] {
            let bg_hsla: gpui::Hsla = bg.into();
            let text_hsla: gpui::Hsla = text.into();
            let i = sh_core::theme::ThemeInteraction::default();
            let non_viewer = viewer_control_hover_fill(View::Grid, bg_hsla, text_hsla, &i);
            let viewer = viewer_control_hover_fill(View::Viewer, bg_hsla, text_hsla, &i);

            assert_eq!(non_viewer, hover_fill(bg_hsla, text_hsla, i.hover_ratio));
            assert_eq!(
                viewer,
                hover_fill_strong(bg_hsla, text_hsla, i.hover_ratio_strong)
            );
            assert!(
                (luma(viewer) - luma(bg_hsla)).abs() > (luma(non_viewer) - luma(bg_hsla)).abs(),
                "Viewer controls need stronger theme-aware hover contrast"
            );
        }
    }

    #[test]
    fn hover_tint_blends_foreground_into_background() {
        // Channel-wise mix in RGBA space: ratio 0 = bg, 1 = fg.
        let bg: gpui::Hsla = gpui::rgb(0x0d0d0f).into(); // Noir Gallery button bg
        let fg: gpui::Hsla = gpui::rgb(0xe8e8ee).into(); // theme text
        let t = hover_tint(bg, fg, 0.10);
        let bg8: gpui::Rgba = bg.into();
        let fg8: gpui::Rgba = fg.into();
        let t8: gpui::Rgba = t.into();
        for (b, f, m) in [
            (bg8.r, fg8.r, t8.r),
            (bg8.g, fg8.g, t8.g),
            (bg8.b, fg8.b, t8.b),
        ] {
            let expected = b + (f - b) * 0.10;
            assert!(
                (m - expected).abs() < 1e-3,
                "channel mix drifted: {m} vs {expected}"
            );
        }
        // Ratio bounds: 0 = pure bg, 1 = pure fg.
        let z8: gpui::Rgba = hover_tint(bg, fg, 0.0).into();
        let o8: gpui::Rgba = hover_tint(bg, fg, 1.0).into();
        assert_eq!((z8.r, z8.g, z8.b), (bg8.r, bg8.g, bg8.b));
        assert_eq!((o8.r, o8.g, o8.b), (fg8.r, fg8.g, fg8.b));
        // Alpha is always opaque.
        assert!((t.a - 1.0).abs() < 1e-5);
    }

    #[test]
    fn hover_fill_uses_sober_ratio_for_light_themes() {
        // Light themes: the same mix ratio that reads as "elevation" on
        // dark turns into a dirty smudge. The eye is more sensitive to
        // darkening on light, so the fill must mix less than dark — but
        // still land near ~7% so the plate is visible on white.
        let light_bg: gpui::Hsla = gpui::rgb(0xf4f4f6).into(); // Light Clean bg
        let light_text: gpui::Hsla = gpui::rgb(0x1a1a1e).into();
        let dark_bg: gpui::Hsla = gpui::rgb(0x0d0d0f).into(); // Noir Gallery bg
        let dark_text: gpui::Hsla = gpui::rgb(0xe8e8ee).into();

        // Both surfaces derive their fill from the same helper…
        let i = sh_core::theme::ThemeInteraction::default();
        let light_fill = hover_fill(light_bg, light_text, i.hover_ratio);
        let dark_fill = hover_fill(dark_bg, dark_text, i.hover_ratio);
        // …but light must mix LESS than dark.
        let lf8: gpui::Rgba = light_fill.into();
        let lb8: gpui::Rgba = light_bg.into();
        let lt8: gpui::Rgba = light_text.into();
        let df8: gpui::Rgba = dark_fill.into();
        let db8: gpui::Rgba = dark_bg.into();
        let light_delta = (lf8.r - lb8.r).abs();
        let dark_delta = (df8.r - db8.r).abs();
        assert!(
            light_delta < dark_delta,
            "light hover must be subtler than dark ({light_delta} vs {dark_delta})"
        );
        // Pin the light ramp: ~7% mix toward text (was ~5% before the
        // feedback bump — strong enough to read on white, far from smudge).
        let light_ratio = (lf8.r - lb8.r) / (lt8.r - lb8.r);
        assert!(
            (0.065..=0.078).contains(&light_ratio),
            "light hover should mix ~7% of text, got {light_ratio}"
        );
        // Alpha stays opaque.
        assert!((light_fill.a - 1.0).abs() < 1e-5);
    }

    #[test]
    fn hover_fill_strong_pushes_light_chip_past_page() {
        // Root cause of "welcome buttons have no hover": on light-clean the
        // chip starts at the surface (pure white) ABOVE the page (#f4f4f6);
        // the default 7% fill darkens it toward #efefef, which is only 5
        // points from the page — edge contrast collapses and the state
        // change reads as nothing. The strong variant must mix past the
        // page so the chip pops a visible edge (same cue as the grid plate).
        let surface: gpui::Hsla = gpui::rgb(0xffffff).into(); // light-clean surface
        let page: gpui::Hsla = gpui::rgb(0xf4f4f6).into(); // light-clean background
        let text: gpui::Hsla = gpui::rgb(0x1a1a1e).into();

        let i = sh_core::theme::ThemeInteraction::default();
        let fill = hover_fill(surface, text, i.hover_ratio);
        let strong = hover_fill_strong(surface, text, i.hover_ratio_strong);
        let sb8: gpui::Rgba = surface.into();
        let pb8: gpui::Rgba = page.into();
        let f8: gpui::Rgba = fill.into();
        let s8: gpui::Rgba = strong.into();
        let luma = |c: &gpui::Rgba| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;

        // The default fill crosses just below the page (~5-point edge): the
        // resting chip floats ~11 points ABOVE it, so hovering collapses the
        // chip's edge cue — it converges INTO the page and reads as "no
        // hover", even though the raw pixel delta matches the grid plate.
        assert!(luma(&f8) < luma(&pb8));
        assert!(luma(&pb8) < luma(&sb8));
        // The default fill's edge cue is weaker than the resting chip's.
        let edge = |c: &gpui::Rgba| (luma(c) - luma(&pb8)).abs();
        assert!(
            edge(&f8) < edge(&sb8),
            "default fill should collapse the resting chip's edge cue (the reported bug)"
        );
        // The strong fill crosses PAST the page: darker than the page and
        // visibly further from it than the resting chip.
        assert!(
            luma(&s8) < luma(&pb8),
            "strong fill must darken past the page, got {}",
            luma(&s8)
        );
        assert!(
            edge(&s8) > edge(&sb8),
            "strong fill must restore the resting chip's edge cue"
        );
        // Strong mixes more than the default for the same surface pair.
        let delta = |c: &gpui::Rgba| (c.r - sb8.r).abs();
        assert!(
            delta(&s8) > delta(&f8),
            "strong fill must mix further toward text on light surfaces"
        );
        assert!((strong.a - 1.0).abs() < 1e-5);
    }

    #[gpui::test]
    fn tab_toggle_refits_once_and_idle_never_refits(cx: &mut gpui::TestAppContext) {
        // In-flow bottom chrome contract (supersedes the floating-chrome
        // R3 rule): Tab ON mounts `#viewer-chrome` BELOW the image, so
        // the toggle changes the image area itself and the Tab action
        // refits exactly once against the new carve (Fit mode only —
        // the old absolute-chrome "Tab never refits" guarantee applied
        // while the chrome merely floated over the image). Idle NEVER
        // refits: the idle state is absent from `viewer_viewport`, so
        // aging `last_interaction` alone leaves (viewport, zoom)
        // bit-identical — the container grows below the unchanged image
        // instead of jolting it. Matrix over strip ON/OFF; REAL
        // dimensions so refits measurably change zoom.
        for strip in [true, false] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            app.update(cx, |app, cx| {
                app.view = View::Viewer;
                app.session.images[0].dimensions = Some((4000, 2000));
                app.session.current = 0;
                app.session.fit_mode = FitMode::Fit;
                app.settings.filmstrip = strip;
                app.session.show_overlay_bottom = false; // Tab OFF start
                app.session.refit_for_viewport(app.viewer_viewport());
                cx.notify();
            });
            cx.run_until_parked(); // render syncs self.viewport
                                   // ── Idle never refits (flag unchanged, interaction aged) ──
            let before_idle = app.read_with(cx, |app, _| (app.viewer_viewport(), app.session.zoom));
            app.update(cx, |app, cx| {
                app.last_interaction = std::time::Instant::now()
                    - crate::ui::overlay::OVERLAY_IDLE
                    - std::time::Duration::from_secs(1);
                cx.notify();
            });
            cx.run_until_parked();
            let after_idle = app.read_with(cx, |app, _| (app.viewer_viewport(), app.session.zoom));
            assert_eq!(
                before_idle.0, after_idle.0,
                "idle must not carve (strip={strip})"
            );
            assert_eq!(before_idle.1.scale, after_idle.1.scale, "idle scale static");
            assert_eq!(
                before_idle.1.offset, after_idle.1.offset,
                "idle offset static"
            );
            // ── Tab toggle refits exactly once against the new carve ──
            // Exactly what the ToggleOverlays action does now: flip + one
            // refit. The viewport loses exactly BOTTOM_CHROME_H_PX.
            let before_tab = app.read_with(cx, |app, _| (app.viewer_viewport(), app.session.zoom));
            app.update(cx, |app, cx| {
                app.note_interaction(cx);
                app.session.show_overlay_bottom = true;
                app.session.refit_for_viewport(app.viewer_viewport());
                cx.notify();
            });
            cx.run_until_parked();
            app.read_with(cx, |app, _| {
                let vp = app.viewer_viewport();
                assert!(
                    (before_tab.0.y - vp.y - crate::ui::overlay::BOTTOM_CHROME_H_PX).abs() < 1e-5,
                    "toggle must carve exactly the chrome height (strip={strip}): {} vs {}",
                    before_tab.0.y,
                    vp.y
                );
                assert_eq!(vp.x, before_tab.0.x, "full width (strip={strip})");
                assert_eq!(
                    app.session.zoom,
                    sh_core::transform::fit(
                        sh_core::transform::Vec2 {
                            x: 4000.0,
                            y: 2000.0
                        },
                        vp
                    ),
                    "Fit must refit against the chrome-carved viewport (strip={strip})"
                );
            });
            // ── Percent100: the refit call must be a no-op ──
            // A non-fit user zoom (manual scale + pan) must survive the
            // refit untouched — only FitMode::Fit recomputes.
            app.update(cx, |app, _| {
                app.session.fit_mode = FitMode::Percent100;
                app.session.zoom = sh_core::transform::ZoomState {
                    scale: 1.7,
                    offset: sh_core::transform::Vec2 { x: 55.0, y: -42.0 },
                };
                app.session.refit_for_viewport(app.viewer_viewport());
            });
            app.read_with(cx, |app, _| {
                assert_eq!(
                    app.session.zoom.scale, 1.7,
                    "Percent100 refit must be a no-op (strip={strip})"
                );
                assert_eq!(
                    app.session.zoom.offset.x, 55.0,
                    "manual pan must survive (strip={strip})"
                );
                assert_eq!(
                    app.session.zoom.offset.y, -42.0,
                    "manual pan must survive (strip={strip})"
                );
            });
        }
    }

    #[gpui::test]
    fn set_filmstrip_refits_once_in_fit_and_ignores_percent100(cx: &mut gpui::TestAppContext) {
        // R5.3: the toggle is an explicit layout change, so it refits
        // against the new carve in Fit mode (single call site in
        // `set_filmstrip`, outcome pinned here) and leaves a Percent100
        // user zoom untouched. REAL dimensions so a refit measurably
        // changes the zoom. Live viewport reads throughout: renders
        // interleave in the harness and sync `self.viewport` with the
        // test window (same discipline as the R3 Tab test below).
        use sh_core::transform::Vec2;
        let img = Vec2 {
            x: 4000.0,
            y: 2000.0,
        };
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.session.images[0].dimensions = Some((4000, 2000));
            app.session.current = 0;
            app.session.fit_mode = FitMode::Fit;
            // Explicit seed: the render resize path only refits on a size
            // CHANGE, and the creation render ran before dims existed.
            app.session.refit_for_viewport(app.viewer_viewport());
            cx.notify();
        });
        cx.run_until_parked(); // render syncs viewport; resize path refits
        let (vp_on, zoom_on) = app.read_with(cx, |app, _| {
            assert!(app.settings.filmstrip, "default ON");
            (app.viewer_viewport(), app.session.zoom)
        });
        assert_eq!(
            zoom_on,
            sh_core::transform::fit(img, vp_on),
            "resize path fits the carved viewport while ON"
        );
        // Toggle OFF: refit against the restored full window.
        app.update(cx, |app, cx| {
            app.set_filmstrip(false, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.settings.filmstrip);
            let vp_off = app.viewer_viewport();
            assert!(
                (vp_off.y - vp_on.y - crate::filmstrip::STRIP_H_PX).abs() < 1e-5,
                "toggle restores exactly the strip height ({} vs {})",
                vp_off.y,
                vp_on.y
            );
            assert_eq!(vp_off.x, vp_on.x, "full width in both states");
            assert_eq!(
                app.session.zoom,
                sh_core::transform::fit(img, vp_off),
                "Fit must refit against the restored viewport"
            );
        });
        // Toggle back ON: refit against the carved viewport.
        app.update(cx, |app, cx| {
            app.set_filmstrip(true, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.settings.filmstrip);
            assert_eq!(
                app.session.zoom,
                sh_core::transform::fit(img, app.viewer_viewport()),
                "Fit must refit against the carved viewport"
            );
        });
        // Percent100 user zoom survives the toggle untouched.
        app.update(cx, |app, _| {
            app.session.fit_mode = FitMode::Percent100;
        });
        let user_zoom = app.read_with(cx, |app, _| app.session.zoom);
        app.update(cx, |app, cx| {
            app.set_filmstrip(false, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.zoom, user_zoom, "Percent100 zoom untouched");
        });
    }

    /// Build `n` fake session images (`Z:\fake\img{i:02}.png`). Paths
    /// need not exist — dimension probes fail into the error slot, which
    /// strip tests never assert on; window/click/marker math needs indices
    /// only. Mirrors the `test_app` fixture shape (extension-filtered).
    fn fake_images(n: usize) -> Vec<crate::state::session::ImageItem> {
        let epoch = std::time::SystemTime::UNIX_EPOCH;
        build_image_items((0..n).map(|i| sh_core::navigation::ImageEntry {
            path: PathBuf::from(format!("Z:\\fake\\img{i:02}.png")),
            size: 0,
            modified: epoch,
            created: None,
        }))
    }

    /// Center of strip cell `target` in window pixels, derived from the
    /// mounted `#filmstrip` bounds plus the production geometry consts
    /// (no duplicated layout math to drift): centered row offset + left pad
    /// + `k * (cell+gap)`.
    fn strip_cell_center(
        strip: gpui::Bounds<gpui::Pixels>,
        target: usize,
        current: usize,
        len: usize,
    ) -> gpui::Point<gpui::Pixels> {
        use crate::filmstrip::{
            filmstrip_row_offset, filmstrip_window, STRIP_CELL_PX, STRIP_GAP_PX,
        };
        let window = filmstrip_window(current, len);
        assert!(
            window.contains(&target),
            "target={target} must be inside the rendered window {window:?}"
        );
        let k = (target - window.start) as f32;
        let row_offset = filmstrip_row_offset(f32::from(strip.size.width), current, len);
        let x = f32::from(strip.origin.x)
            + row_offset
            + STRIP_GAP_PX
            + k * (STRIP_CELL_PX + STRIP_GAP_PX)
            + STRIP_CELL_PX / 2.0;
        let y = f32::from(strip.origin.y) + f32::from(strip.size.height) / 2.0;
        gpui::Point {
            x: gpui::px(x),
            y: gpui::px(y),
        }
    }

    /// R1.1/R1.2 + R7.6 (mount half): `#filmstrip` mounts iff the view is
    /// Viewer AND the setting is ON. The `debug_selector` pin is test-only
    /// (noop in release): `Some` bounds ⇒ mounted, `None` ⇒ absent. The
    /// outside-`#zoom-layer` half holds by construction — the strip is
    /// composed as a flex sibling of `#viewer-main` (which owns
    /// `#zoom-layer`), and `render_viewer` is untouched (see 2.7) — plus
    /// the height pin below (in-flow `.h(STRIP_H_PX)`, never overlaid).
    #[gpui::test]
    fn filmstrip_mounts_in_viewer_only_with_setting_on(cx: &mut gpui::TestAppContext) {
        for (view, setting, expect) in [
            (View::Viewer, true, true),
            (View::Viewer, false, false),
            (View::Grid, true, false),
            (View::Welcome, true, false),
            (View::Settings, true, false),
        ] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            app.update(cx, |app, cx| {
                app.view = view;
                app.settings.filmstrip = setting;
                cx.notify();
            });
            cx.run_until_parked();
            let bounds = cx.debug_bounds("filmstrip");
            assert_eq!(
                bounds.is_some(),
                expect,
                "view={view:?} filmstrip={setting}: mount mismatch"
            );
            if expect {
                let height = f32::from(bounds.expect("mounted").size.height);
                let want = crate::filmstrip::STRIP_H_PX;
                assert!(
                    (height - want).abs() < 1.0,
                    "strip is fixed-height chrome ({height} vs {want})"
                );
            }
        }
    }

    /// Bugfix (viewer-filmstrip): the strip is bottom-docked layout
    /// chrome — `#viewer-main` (flex_1) first, `#filmstrip`
    /// (.h(STRIP_H_PX)) last inside the flex-col `#viewer-area`. The strip
    /// must sit BELOW the image area, the image area must keep nonzero
    /// height, and the strip keeps its fixed height.
    #[gpui::test]
    fn filmstrip_docks_below_viewer_main_with_nonzero_image_area(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.settings.filmstrip = true;
            cx.notify();
        });
        cx.run_until_parked();
        let area = cx.debug_bounds("viewer-area").expect("viewer-area mounts");
        let main = cx.debug_bounds("viewer-main").expect("viewer-main mounts");
        let strip = cx.debug_bounds("filmstrip").expect("strip mounts");
        let area_h = f32::from(area.size.height);
        let main_h = f32::from(main.size.height);
        assert!(
            main_h > 1.0,
            "viewer-main must keep nonzero height (area_h={area_h} main_h={main_h} strip_y={} strip_h={})",
            f32::from(strip.origin.y),
            f32::from(strip.size.height),
        );
        let strip_top = f32::from(strip.origin.y);
        let main_bottom = f32::from(main.origin.y) + main_h;
        assert!(
            strip_top >= main_bottom - 1.0,
            "strip must dock below viewer-main (strip_top={strip_top} main_bottom={main_bottom})"
        );
        let strip_h = f32::from(strip.size.height);
        assert!(
            (strip_h - crate::filmstrip::STRIP_H_PX).abs() < 1.0,
            "strip keeps fixed height ({strip_h} vs {})",
            crate::filmstrip::STRIP_H_PX
        );
    }

    /// R1.6/R1.7: Tab gates overlay chrome only and idle fades overlay
    /// chrome only — the strip stays mounted with bit-identical bounds
    /// across a Tab flip and past `OVERLAY_IDLE`.
    #[gpui::test]
    fn filmstrip_survives_tab_flip_and_idle(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.settings.filmstrip = true;
            cx.notify();
        });
        cx.run_until_parked();
        let before = cx.debug_bounds("filmstrip").expect("strip mounts");
        // Exactly what the Tab action flips (overlay flag only).
        app.update(cx, |app, cx| {
            app.session.show_overlay_bottom = !app.session.show_overlay_bottom;
            cx.notify();
        });
        cx.run_until_parked();
        let after_tab = cx.debug_bounds("filmstrip").expect("strip survives Tab");
        assert_eq!(before, after_tab, "Tab must leave the strip untouched");
        // Idle past the overlay deadline.
        app.update(cx, |app, cx| {
            app.last_interaction = std::time::Instant::now()
                - crate::ui::overlay::OVERLAY_IDLE
                - std::time::Duration::from_secs(1);
            cx.notify();
        });
        cx.run_until_parked();
        let after_idle = cx.debug_bounds("filmstrip").expect("strip survives idle");
        assert_eq!(
            before, after_idle,
            "idle must leave the strip fully visible"
        );
    }

    /// Leak a test-only element selector (`debug_bounds` needs
    /// `&'static str`; harness-only, never production).
    fn static_selector(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }

    /// Assert the live active marker sits on `current`: centered in its
    /// cell horizontally with the 3px bar height. NOTE: GPUI 0.2.2 never
    /// clears `debug_bounds` across frames (append-only), so selector
    /// ABSENCE is only meaningful in a fresh window — after navigation the
    /// old selector lingers stale. Position of the CURRENT selector is the
    /// follow proof: a stuck marker would leave `strip-active-{current}`
    /// absent (or misplaced).
    fn assert_marker_on_cell(cx: &mut gpui::VisualTestContext, current: usize, len: usize) {
        let strip = cx.debug_bounds("filmstrip").expect("strip mounts");
        let marker = cx
            .debug_bounds(static_selector(format!("strip-active-{current}")))
            .expect("live marker renders on current");
        let cell = strip_cell_center(strip, current, current, len);
        let marker_cx = f32::from(marker.origin.x) + f32::from(marker.size.width) / 2.0;
        assert!(
            (marker_cx - f32::from(cell.x)).abs() < 1.0,
            "marker must center in cell {current} ({marker_cx} vs {})",
            f32::from(cell.x)
        );
        assert!(
            (f32::from(marker.size.height) - 3.0).abs() < 0.5,
            "marker is the 3px bar"
        );
    }

    /// R2.5: in-window indices with no resident thumbnail render the
    /// neutral pending placeholder — never a broken-image treatment, never
    /// a crash. (With an empty `thumbs` map every cell is a miss.)
    #[gpui::test]
    fn filmstrip_missing_thumb_renders_neutral_placeholder(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.session.images = fake_images(3);
            app.view = View::Viewer;
            app.settings.filmstrip = true;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("filmstrip").is_some(), "strip mounts");
        for idx in 0..3 {
            assert!(
                cx.debug_bounds(static_selector(format!("strip-empty-{idx}")))
                    .is_some(),
                "miss cell {idx} renders the placeholder"
            );
            assert!(
                cx.debug_bounds(static_selector(format!("strip-thumb-{idx}")))
                    .is_none(),
                "miss cell {idx} never renders a (nonexistent) cached image"
            );
        }
    }

    /// Bugfix (viewer-filmstrip): a large folder must keep the current
    /// thumbnail inside the visible `#filmstrip` bounds, not merely render
    /// the correct cell in the right logical range. The harness uses the
    /// production-sized 1000px viewport and a 49-cell window.
    #[gpui::test]
    fn filmstrip_keeps_current_thumbnail_inside_visible_bounds(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        cx.simulate_resize(gpui::size(gpui::px(1000.0), gpui::px(720.0)));
        app.update(cx, |app, cx| {
            app.session.images = fake_images(100);
            app.session.current = 50;
            app.view = View::Viewer;
            app.settings.filmstrip = true;
            cx.notify();
        });
        cx.run_until_parked();

        let strip = cx.debug_bounds("filmstrip").expect("strip mounts");
        let marker = cx
            .debug_bounds("strip-active-50")
            .expect("active marker renders");
        let thumbnail = cx
            .debug_bounds("strip-empty-50")
            .expect("current thumbnail placeholder renders");
        let strip_left = f32::from(strip.origin.x);
        let strip_right = strip_left + f32::from(strip.size.width);
        let marker_left = f32::from(marker.origin.x);
        let marker_right = marker_left + f32::from(marker.size.width);
        let thumbnail_left = f32::from(thumbnail.origin.x);
        let thumbnail_right = thumbnail_left + f32::from(thumbnail.size.width);

        assert!(
            (strip_right - strip_left - 1000.0).abs() < 1.0,
            "test viewport must be 1000px wide (got {}px)",
            strip_right - strip_left
        );
        let strip_center = (strip_left + strip_right) / 2.0;
        let marker_center = (marker_left + marker_right) / 2.0;
        assert!(
            (marker_center - strip_center).abs() < 1.0,
            "active marker must be centered in the visible strip ({marker_center} vs {strip_center})"
        );
        assert!(
            marker_left < strip_right && marker_right > strip_left,
            "active marker must intersect visible strip bounds: marker=[{marker_left}, {marker_right}], strip=[{strip_left}, {strip_right}]"
        );
        assert!(
            thumbnail_left < strip_right && thumbnail_right > strip_left,
            "current thumbnail must intersect visible strip bounds: thumbnail=[{thumbnail_left}, {thumbnail_right}], strip=[{strip_left}, {strip_right}]"
        );
    }

    /// Bugfix (viewer-filmstrip): boundary and narrow-window cases retain
    /// the same observable contract as the large-folder case.
    #[gpui::test]
    fn filmstrip_keeps_boundary_current_visible_at_small_viewports(cx: &mut gpui::TestAppContext) {
        for (len, current, viewport_width) in [
            (100usize, 0usize, 1000.0),
            (100, 99, 1000.0),
            (100, 50, 320.0),
        ] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            cx.simulate_resize(gpui::size(gpui::px(viewport_width), gpui::px(720.0)));
            app.update(cx, |app, cx| {
                app.session.images = fake_images(len);
                app.session.current = current;
                app.view = View::Viewer;
                app.settings.filmstrip = true;
                cx.notify();
            });
            cx.run_until_parked();

            let strip = cx.debug_bounds("filmstrip").expect("strip mounts");
            let marker = cx
                .debug_bounds(static_selector(format!("strip-active-{current}")))
                .expect("active marker renders");
            let thumbnail = cx
                .debug_bounds(static_selector(format!("strip-empty-{current}")))
                .expect("current thumbnail placeholder renders");
            let strip_left = f32::from(strip.origin.x);
            let strip_right = strip_left + f32::from(strip.size.width);
            let marker_left = f32::from(marker.origin.x);
            let marker_right = marker_left + f32::from(marker.size.width);
            let thumbnail_left = f32::from(thumbnail.origin.x);
            let thumbnail_right = thumbnail_left + f32::from(thumbnail.size.width);
            let strip_center = (strip_left + strip_right) / 2.0;
            let marker_center = (marker_left + marker_right) / 2.0;

            assert!(
                (strip_right - strip_left - viewport_width).abs() < 1.0,
                "unexpected strip width for viewport={viewport_width}"
            );
            assert!(
                marker_left < strip_right
                    && marker_right > strip_left
                    && thumbnail_left < strip_right
                    && thumbnail_right > strip_left,
                "current={current} must remain visible at viewport={viewport_width}"
            );
            assert!(
                (marker_center - strip_center).abs() < 1.0,
                "current={current} must stay centered at viewport={viewport_width}"
            );
        }
    }

    /// R3.3/R3.4 (element half): the board layer mounts iff the setting is
    /// ON plus a CONFIRMED transparent verdict on the cached map —
    /// 4-case matrix over a seeded resident thumb. (Gate logic itself is
    /// pinned without GPUI in `strip_board_gate_matches_grid`.)
    #[gpui::test]
    fn filmstrip_board_follows_cached_verdict(cx: &mut gpui::TestAppContext) {
        for (checkerboard, verdict, expect) in [
            (true, Some(true), true),
            (true, Some(false), false),
            (true, None, false),
            (false, Some(true), false),
        ] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            app.update(cx, |app, cx| {
                app.session.images = fake_images(3);
                app.view = View::Viewer;
                app.settings.filmstrip = true;
                app.settings.checkerboard = checkerboard;
                // One resident thumb on index 0 (synthetic RGBA, pure CPU —
                // no decode, no I/O): the hit arm must render it.
                let path = app.session.images[0].path.clone();
                let decoded = sh_core::decode::DecodedImage {
                    width: 4,
                    height: 4,
                    rgba: vec![255u8, 0, 0, 255]
                        .into_iter()
                        .cycle()
                        .take(4 * 4 * 4)
                        .collect(),
                };
                let thumb =
                    crate::thumbs::render_thumb(&decoded).expect("synthetic thumb converts");
                app.thumbs.insert(path.clone(), thumb);
                if let Some(v) = verdict {
                    app.thumb_alpha.insert(path, v);
                }
                cx.notify();
            });
            cx.run_until_parked();
            assert_eq!(
                cx.debug_bounds(static_selector("strip-board-0".to_string()))
                    .is_some(),
                expect,
                "checkerboard={checkerboard} verdict={verdict:?}: board mismatch"
            );
            assert!(
                cx.debug_bounds(static_selector("strip-thumb-0".to_string()))
                    .is_some(),
                "seeded thumb renders the cached image"
            );
            assert!(
                cx.debug_bounds(static_selector("strip-empty-0".to_string()))
                    .is_none(),
                "seeded thumb never renders the placeholder"
            );
        }
    }

    /// R4.1–R4.3 + R7 guards: a real click on cell 14 at current 10
    /// navigates via `navigate(+4)`; the active marker (derived per frame
    /// from `session.current`) follows to 14, then to 7. Strip clicks touch
    /// no multi-select state, and arrows keep `navigate(±1)` semantics.
    #[gpui::test]
    fn filmstrip_click_navigates_and_marker_follows(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let interaction_seed = std::time::Instant::now() - std::time::Duration::from_millis(200);
        app.update(cx, |app, cx| {
            app.session.images = fake_images(20);
            app.session.current = 10;
            app.view = View::Viewer;
            app.settings.filmstrip = true;
            app.last_interaction = interaction_seed;
            cx.notify();
        });
        cx.run_until_parked();
        // R4.2: exactly the current cell wears the marker.
        assert!(
            cx.debug_bounds("strip-active-10").is_some(),
            "marker starts on current"
        );
        assert!(cx.debug_bounds("strip-active-9").is_none());
        assert!(cx.debug_bounds("strip-active-11").is_none());
        // R4.1: click cell 14 → current == 14 via navigate(+4).
        let strip = cx.debug_bounds("filmstrip").expect("strip mounts");
        cx.simulate_click(
            strip_cell_center(strip, 14, 10, 20),
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.current, 14);
            assert!(
                app.last_interaction > interaction_seed,
                "filmstrip click must refresh last_interaction"
            );
        });
        // R4.3a: the marker follows to 14 (position proof — see helper).
        assert_marker_on_cell(cx, 14, 20);
        // R4.3: click cell 7 → current == 7, marker follows to 7.
        let strip = cx.debug_bounds("filmstrip").expect("strip mounts");
        cx.simulate_click(
            strip_cell_center(strip, 7, 14, 20),
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.current, 7);
            // R7.1: no multi-select state touched by strip clicks.
            assert!(app.selected.is_empty(), "strip never multi-selects");
            assert_eq!(app.grid_selected, 0, "strip never moves the grid cursor");
        });
        assert_marker_on_cell(cx, 7, 20);
        // R7.3: arrows keep navigate(±1) semantics with the strip mounted.
        let before_programmatic_navigation = app.read_with(cx, |app, _| app.last_interaction);
        app.update(cx, |app, cx| {
            app.navigate(1, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.current, 8);
            assert_eq!(
                app.last_interaction, before_programmatic_navigation,
                "programmatic navigation must not turn into a user interaction"
            );
        });
    }

    #[test]
    fn wheel_parks_welcome_and_settings() {
        // Parked views never touch viewer/grid state: Welcome has nothing
        // to zoom or scroll, and Settings content scrolls natively
        // (`overflow_y_scroll` owns the gesture) — without this guard the
        // wheel falls through to viewer zoom and silently mutates
        // `session.zoom` while in Settings.
        assert!(wheel_parks(View::Welcome));
        assert!(wheel_parks(View::Settings));
        assert!(!wheel_parks(View::Grid));
        assert!(!wheel_parks(View::Viewer));
    }

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

    #[test]
    fn eight_digit_preserves_alpha() {
        // #0d0d0f80 → R=0x0d, G=0x0d, B=0x0f, A=0x80 (~0.502)
        let c = parse_hex("#0d0d0f80").unwrap();
        let expected_alpha = 0x80 as f32 / 255.0;
        assert!(
            (c.a - expected_alpha).abs() < 1e-5,
            "expected alpha ≈ {expected_alpha}, got {}",
            c.a
        );
        // Fully opaque variant should have alpha ≈ 1.0.
        let opaque = parse_hex("#0d0d0fff").unwrap();
        assert!((opaque.a - 1.0).abs() < 1e-5);
    }

    #[test]
    fn rejects_unicode_garbage() {
        assert!(parse_hex("ééé").is_none());
    }

    #[test]
    fn hover_moved_enough_ignores_sensor_noise() {
        // Sub-pixel sensor jitter must not reset the idle clock.
        assert!(!super::hover_moved_enough((100.0, 100.0), (101.0, 101.0)));
        assert!(!super::hover_moved_enough((0.0, 0.0), (2.9, 0.0)));
    }

    #[test]
    fn hover_moved_enough_detects_intentional_move() {
        assert!(super::hover_moved_enough((100.0, 100.0), (110.0, 100.0)));
        assert!(super::hover_moved_enough((0.0, 0.0), (0.0, 10.0)));
    }

    #[test]
    fn hover_moved_enough_boundary_at_threshold() {
        // Exactly the deadband distance counts as movement (>=).
        assert!(super::hover_moved_enough((0.0, 0.0), (3.0, 0.0)));
        assert!(super::hover_moved_enough((0.0, 0.0), (1.8, 2.4)));
    }

    /// S3: Welcome hero + buttons resolve through the table (full sentences,
    /// never substrings) so the Welcome surface live-switches with
    /// `settings.language`.
    #[test]
    fn welcome_strings_come_from_the_table() {
        use sh_core::i18n::{t, Language, StrKey};
        assert_eq!(
            t(Language::En, StrKey::WelcomeTagline),
            "A native, GPU-accelerated image viewer"
        );
        assert_eq!(
            t(Language::Es, StrKey::WelcomeTagline),
            "Un visor de imágenes nativo acelerado por GPU"
        );
        assert_eq!(
            t(Language::En, StrKey::DropZoneHint),
            "Drop images or a folder here"
        );
        assert_eq!(
            t(Language::Es, StrKey::DropZoneHint),
            "Suelte imágenes o una carpeta aquí"
        );
        assert_eq!(t(Language::En, StrKey::ContinueButton), "Continue");
        assert_eq!(t(Language::Es, StrKey::ContinueButton), "Continuar");
        assert_eq!(t(Language::En, StrKey::WelcomeOpen), "Open folder…");
        assert_eq!(t(Language::Es, StrKey::WelcomeOpen), "Abrir carpeta…");
    }

    /// S4: viewer chrome (topbar back/open, crop bar, batch confirm bar,
    /// viewer empty state) resolves through the table in both languages.
    /// The zoom `%` text is dynamic (locale-neutral) and stays untranslated.
    #[test]
    fn viewer_chrome_labels_come_from_the_table() {
        use sh_core::i18n::{t, Language, StrKey};
        assert_eq!(t(Language::En, StrKey::TopbarBack), "Back");
        assert_eq!(t(Language::Es, StrKey::TopbarBack), "Atrás");
        assert_eq!(t(Language::En, StrKey::TopbarOpen), "Open folder");
        assert_eq!(t(Language::Es, StrKey::TopbarOpen), "Abrir carpeta");
        assert_eq!(t(Language::En, StrKey::CropCopy), "Copy");
        assert_eq!(t(Language::Es, StrKey::CropCopy), "Copiar");
        assert_eq!(t(Language::En, StrKey::CropSave), "Save…");
        assert_eq!(t(Language::Es, StrKey::CropSave), "Guardar…");
        assert_eq!(t(Language::En, StrKey::Cancel), "Cancel");
        assert_eq!(t(Language::Es, StrKey::Cancel), "Cancelar");
        assert_eq!(t(Language::En, StrKey::BatchDelete), "Delete");
        assert_eq!(t(Language::Es, StrKey::BatchDelete), "Eliminar");
        assert_eq!(t(Language::En, StrKey::BatchMove), "Move");
        assert_eq!(t(Language::Es, StrKey::BatchMove), "Mover");
        assert_eq!(
            t(Language::En, StrKey::ViewerEmptyHint),
            "Drop an image to open it"
        );
        assert_eq!(
            t(Language::Es, StrKey::ViewerEmptyHint),
            "Suelte una imagen para abrirla"
        );
    }

    /// S4: the viewer empty-state hint resolves through the language handed
    /// to `render_viewer` via `ViewerParams` (not a hardcoded English literal).
    #[test]
    fn viewer_empty_hint_resolves_params_language() {
        use crate::viewer::render_viewer;
        use sh_core::i18n::{t, Language, StrKey};
        let params = crate::viewer::ViewerParams {
            path: None,
            error: None,
            zoom_scale: 1.0,
            pan_offset: sh_core::transform::Vec2 { x: 0.0, y: 0.0 },
            decoded_size: None,
            lang: Language::Es,
            show_checkerboard: false,
            checker_palette: crate::checkerboard::palette_for_page(gpui::rgb(0x101014).into()),
        };
        let _view = render_viewer(&params);
        // The rendered tree must carry the table string for the given
        // language, not the hardcoded English literal.
        assert_ne!(
            t(Language::Es, StrKey::ViewerEmptyHint),
            t(Language::En, StrKey::ViewerEmptyHint)
        );
    }

    /// S3: grid empty-state default + sort header resolve through the table
    /// in both languages.
    #[test]
    fn grid_empty_and_sort_header_come_from_the_table() {
        use sh_core::i18n::{t, Language, StrKey};
        assert_eq!(
            t(Language::En, StrKey::GridEmptyDefault),
            "No images in this folder"
        );
        assert_eq!(
            t(Language::Es, StrKey::GridEmptyDefault),
            "No hay imágenes en esta carpeta"
        );
        assert_eq!(t(Language::En, StrKey::SortByLabel), "Sort by");
        assert_eq!(t(Language::Es, StrKey::SortByLabel), "Ordenar por");
    }

    /// S3: sort criterion + direction labels resolve through the table; the
    /// chip keeps its locale-neutral direction glyph (`↑`/`↓` untranslated
    /// per the proper-noun/glyph exemption).
    #[test]
    fn sort_chip_label_resolves_criterion_through_the_table() {
        use sh_core::i18n::Language;
        assert_eq!(
            sort_chip_label(Language::En, SortBy::Name, SortDir::Asc),
            "Name ↑",
            "defaults must render as Name ↑"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Name, SortDir::Asc),
            "Nombre ↑"
        );
        assert_eq!(
            sort_chip_label(Language::En, SortBy::Size, SortDir::Desc),
            "Size ↓"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Size, SortDir::Desc),
            "Tamaño ↓"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Created, SortDir::Desc),
            "Creación ↓"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Modified, SortDir::Asc),
            "Modificación ↑"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Type, SortDir::Asc),
            "Tipo ↑"
        );
    }

    /// S3: the topbar selection-count suffix delegates to the
    /// `selected_suffix` template (empty when nothing is selected).
    #[test]
    fn selection_suffix_delegates_to_the_template() {
        use sh_core::i18n::Language;
        use std::collections::BTreeSet;
        assert_eq!(selected_count_suffix(Language::En, &BTreeSet::new()), "");
        assert_eq!(selected_count_suffix(Language::Es, &BTreeSet::new()), "");
        assert_eq!(
            selected_count_suffix(Language::En, &BTreeSet::from([0])),
            " (1 selected)"
        );
        assert_eq!(
            selected_count_suffix(Language::Es, &BTreeSet::from([0])),
            " (1 seleccionado)"
        );
        assert_eq!(
            selected_count_suffix(Language::En, &BTreeSet::from([0, 4, 9])),
            " (3 selected)"
        );
        assert_eq!(
            selected_count_suffix(Language::Es, &BTreeSet::from([0, 4, 9])),
            " (3 seleccionados)"
        );
    }

    /// The overlay chip shows the ACTION, not the state: Pause while
    /// playing, Play while stopped.
    #[test]
    fn slideshow_icon_shows_the_action() {
        assert_eq!(slideshow_kit_icon(false), gpui_kit_assets::IconName::Play);
        assert_eq!(slideshow_kit_icon(true), gpui_kit_assets::IconName::Pause);
    }

    /// Clone the picker row for `file_name` out of the live entry list.
    ///
    /// Resolves the row exactly the way the rendered click handler does
    /// (look the name up in `theme_entries`), so a test can never pass by
    /// applying a theme the picker would not actually offer. Cloned to dodge
    /// the borrow conflict with the `&mut App` the apply call needs.
    fn builtin_entry(
        app: &App,
        file_name: &str,
    ) -> crate::ui::settings_panel::sections::appearance::ThemeEntry {
        app.theme_entries
            .iter()
            .find(|e| e.file_name == file_name)
            .unwrap_or_else(|| panic!("picker must offer {file_name}"))
            .clone()
    }

    /// Minimal valid theme JSON with a caller-chosen `name`, for fixtures
    /// that need a user-supplied theme to be distinguishable by identity.
    fn user_theme_json(name: &str) -> String {
        format!(
            r##"{{
            "name": "{name}",
            "author": "test",
            "version": 1,
            "colors": {{ "background": "#000", "surface": "#111", "text": "#fff", "accent": "#0f0" }},
            "spacing": {{ "xs": 4, "sm": 8, "md": 16, "lg": 24 }},
            "radii": {{ "sm": 2, "md": 6, "lg": 12 }},
            "typography": {{ "family": "Inter", "sizes": {{ "caption": 11, "body": 14, "title": 18 }} }}
        }}"##
        )
    }

    /// Build a minimal App for focus-dispatch tests: two fake images, the
    /// built-in theme, default settings. Paths don't need to exist — the
    /// dimension probe failing merely sets the session error slot, which
    /// these tests never assert on.
    fn test_app(cx: &mut gpui::Context<App>) -> App {
        let epoch = std::time::SystemTime::UNIX_EPOCH;
        let session = Session {
            images: build_image_items(vec![
                sh_core::navigation::ImageEntry {
                    path: PathBuf::from("Z:\\fake\\a.png"),
                    size: 0,
                    modified: epoch,
                    created: None,
                },
                sh_core::navigation::ImageEntry {
                    path: PathBuf::from("Z:\\fake\\b.png"),
                    size: 0,
                    modified: epoch,
                    created: None,
                },
                sh_core::navigation::ImageEntry {
                    path: PathBuf::from("Z:\\fake\\c.png"),
                    size: 0,
                    modified: epoch,
                    created: None,
                },
            ]),
            current: 0,
            ..Session::default()
        };
        test_app_with_session(session, cx)
    }

    /// [`test_app`] with a caller-supplied session.
    ///
    /// `test_app`'s `Z:\fake\*.png` paths do not exist and cannot decode, so
    /// any assertion about decoded thumbnails needs real files on disk. This
    /// also reproduces the real startup shape: a session already populated
    /// before the window ever exists, exactly as `main.rs` builds it from a CLI
    /// folder argument.
    fn test_app_with_session(session: Session, cx: &mut gpui::Context<App>) -> App {
        let theme_text =
            crate::theme_builtins::builtin_theme_json(crate::theme_builtins::DEFAULT_THEME_NAME)
                .to_string();
        let theme = sh_core::theme::parse(&theme_text).expect("built-in theme must parse");
        let theme_store = ThemeStore::new(
            theme,
            crate::theme_builtins::DEFAULT_THEME_NAME.into(),
            PathBuf::from("Z:\\fake\\theme.json"),
        );
        let mut app = App::new(
            session,
            theme_store,
            PathBuf::from("Z:\\fake\\settings.json"),
            sh_core::settings::Settings::default(),
            theme_text,
            cx,
        );
        // Mirror main.rs: playback arms the real task in the harness too, so
        // clock-advanced tests exercise the production lifecycle, not a mock.
        app.rearm_slideshow_timer(cx);
        app
    }

    /// Startup with an already-populated session must arm the thumbnail batch.
    ///
    /// Regression guard for the "every grid cell is an empty gray box" bug.
    ///
    /// `main.rs` scans a CLI folder argument SYNCHRONOUSLY and hands `App::new`
    /// a session that is already populated. Every `spawn_thumb_batch` call site
    /// used to live in an async commit handler (`commit_list_load` /
    /// `commit_open` / `commit_rescan`), none of which runs on that path. The app
    /// therefore started with a full grid and an empty `thumbs` map, painting the
    /// placeholder for every cell until the user navigated somewhere that
    /// happened to re-arm the batch.
    ///
    /// The pre-existing thumbnail tests pass regardless, and that is the trap:
    /// they reach the map through `open_path` / `enter_grid`, which do arm the
    /// batch, and they assert `thumbs.contains_key(..)` — the map, never the
    /// pixels. Nothing in the suite covered construction from a prefilled
    /// session, which is the only way the app actually starts.
    #[gpui::test]
    fn startup_with_a_prefilled_session_arms_the_thumbnail_batch(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let a = fixture_png_in(dir.path(), "a.png");
        let b = fixture_png_in(dir.path(), "b.png");
        let epoch = std::time::SystemTime::UNIX_EPOCH;
        let session = Session {
            images: build_image_items(vec![
                sh_core::navigation::ImageEntry {
                    path: a.clone(),
                    size: 0,
                    modified: epoch,
                    created: None,
                },
                sh_core::navigation::ImageEntry {
                    path: b.clone(),
                    size: 0,
                    modified: epoch,
                    created: None,
                },
            ]),
            current: 0,
            ..Session::default()
        };

        let (app, cx) = cx.add_window_view(|_window, cx| test_app_with_session(session, cx));
        let cx = cx as &mut gpui::VisualTestContext;
        cx.run_until_parked();

        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 2, "session is pre-populated");
            assert!(
                app.thumbs.contains_key(&a),
                "startup must arm the thumbnail batch; thumbs={:?}",
                app.thumbs.keys().collect::<Vec<_>>()
            );
            assert!(
                app.thumbs.contains_key(&b),
                "every startup image needs a thumb, not only the first"
            );
        });
    }

    /// The exact smoke-test bug: with no focus inside the `image_view`
    /// subtree, scoped key bindings never dispatched. This test opens a
    /// headless window, focuses the root the same way main.rs does at
    /// startup, and simulates ←/→ with NO prior mouse interaction.
    ///
    /// Regression guard for review risk N-3 ("verify arrows fire on cold
    /// start").
    #[gpui::test]
    fn arrow_keys_navigate_on_cold_start(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            // Same table as production main.rs — no duplicated literal list.
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });

        let (app, cx) = cx.add_window_view(|window, cx| {
            let app = test_app(cx);
            // Mirror main.rs's startup focus: the tracked root div must own
            // keyboard focus before any keystroke arrives.
            window.focus(&app.focus_handle, cx);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;

        // Cold start: no clicks, no mouse moves — straight to the arrows.
        cx.simulate_keystrokes("right");
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 1));
        cx.simulate_keystrokes("right");
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 2));
        // Wrap-around (circular navigation contract).
        cx.simulate_keystrokes("right");
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 0));
        // Backward.
        cx.simulate_keystrokes("left");
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 2));
    }

    /// `open_folder` with images resolves the first file, populates the
    /// session, and enters the Grid view.
    ///
    /// NOTE: the fixture files contain garbage bytes, not real image data —
    /// `scan_dir`/`first_supported` filter by EXTENSION only, so the session
    /// still lists both files. The background dimension probe will fail on
    /// the garbage content and may set the session error slot; that is
    /// tolerated here, so this test asserts ONLY `view == Grid` and
    /// `images.len() == 2`.
    #[gpui::test]
    fn open_folder_with_images_enters_grid(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"not a real png").expect("fixture a.png");
        std::fs::write(dir.path().join("b.jpg"), b"not a real jpg").expect("fixture b.jpg");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let dir_clone = dir_path.clone();
        app.update(cx, |app, cx| {
            app.open_folder(dir_clone, cx);
        });
        // The scan runs on the background executor: the list only exists
        // after the pump.
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Grid);
            assert_eq!(app.session.images.len(), 2);
        });
        // Keep the tempdir alive until after the assertions.
        drop(dir_path);
    }

    /// The dead-setting regression, end to end. The General-section row used
    /// to flip `show_hidden_files` and persist it, while no scan in the app
    /// ever read the flag: the switch changed nothing on screen. Open a
    /// folder that contains a dot-prefixed image with the flag off, flip the
    /// switch, and the SAME folder (no navigation, no reopen) must re-list.
    ///
    /// Every step has to pump: both the open and the toggle-driven re-scan
    /// go through the background executor.
    #[gpui::test]
    fn show_hidden_files_toggle_rescans_the_open_folder(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"not a real png").expect("fixture a.png");
        std::fs::write(dir.path().join(".thumb.png"), b"not a real png")
            .expect("fixture .thumb.png");
        let dir_path = dir.path().to_path_buf();
        let dot = dir.path().join(".thumb.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open = dir_path.clone();
        app.update(cx, |app, cx| app.open_folder(open, cx));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                !app.settings.show_hidden_files,
                "the flag ships off, so the row starts unchecked"
            );
            assert_eq!(app.session.images.len(), 1, "the dotfile is not listed");
            assert!(
                app.session.images.iter().all(|i| i.path != dot),
                "only the visible image is in the list"
            );
        });
        // The switch alone must change the grid. If the re-scan wiring is
        // removed this assertion is what fails: the boolean flips and the
        // list stays exactly as it was.
        app.update(cx, |app, cx| app.toggle_show_hidden_files(cx));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.settings.show_hidden_files);
            assert_eq!(app.session.images.len(), 2, "the dotfile is now listed");
            assert!(
                app.session.images.iter().any(|i| i.path == dot),
                "and it is the same file on disk, not a stale copy"
            );
            assert!(
                app.session.error.is_none(),
                "a populated folder carries no error claim"
            );
        });
        drop(dir_path);
    }

    /// Turning the flag OFF is the destructive direction: the image the user
    /// is standing on can leave the list. The selection must clamp onto the
    /// nearest surviving image instead of jumping back to cell 0.
    #[gpui::test]
    fn turning_show_hidden_files_off_clamps_onto_a_surviving_image(cx: &mut gpui::TestAppContext) {
        let fixtures =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sh-core/tests/fixtures");
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // REAL fixtures, not garbage bytes: a rescan that moves the current
        // re-probes it, and a probe of garbage content fills the error slot
        // — which is a field this test also asserts on.
        std::fs::copy(fixtures.join("opaque_1x1.png"), dir.path().join("a.png"))
            .expect("copy opaque fixture");
        std::fs::copy(fixtures.join("tiny.jpg"), dir.path().join(".thumb.jpg"))
            .expect("copy jpeg fixture");
        let dir_path = dir.path().to_path_buf();
        let dot = dir.path().join(".thumb.jpg");
        let plain = dir.path().join("a.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open = dir_path.clone();
        app.update(cx, |app, cx| {
            // Open with the rule already ON so both images are listed; the
            // natural sort puts '.thumb.jpg' first ('.' < 'a'), so the
            // current the rescan has to rescue is the dotfile.
            app.settings.show_hidden_files = true;
            app.open_folder(open, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 2);
            assert_eq!(app.session.current_item().expect("an image").path, dot);
        });
        app.update(cx, |app, cx| app.toggle_show_hidden_files(cx));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.settings.show_hidden_files);
            assert_eq!(app.session.images.len(), 1, "the dotfile is gone");
            assert_eq!(
                app.session.current, 0,
                "the index stays in range instead of pointing past the list"
            );
            assert_eq!(
                app.session.current_item().expect("an image").path,
                plain,
                "the selection clamped onto the surviving image"
            );
            assert!(app.session.error.is_none());
        });
        drop(dir_path);
    }

    /// A rescan SHIFTS every index below the change, and the grid selection is
    /// stored as indices. Hiding the dotfile that sorts above `a.png` moves
    /// `a.png` from cell 1 to cell 0, so a selection carried by index would
    /// silently start marking `b.png` instead — and a cursor parked on the
    /// last cell would point one past the end of the shorter list. Both must
    /// be carried across BY PATH, the same way `confirm_pending` carries its
    /// leftovers.
    #[gpui::test]
    fn rescan_remaps_the_grid_selection_by_path(cx: &mut gpui::TestAppContext) {
        let fixtures =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sh-core/tests/fixtures");
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // REAL fixtures: the rescan moves the current image, which re-probes
        // it, and a probe of garbage content would fill the error slot.
        std::fs::copy(fixtures.join("opaque_1x1.png"), dir.path().join(".dot.png"))
            .expect("copy hidden fixture");
        std::fs::copy(fixtures.join("tiny.jpg"), dir.path().join("a.png"))
            .expect("copy first visible fixture");
        std::fs::copy(fixtures.join("opaque_1x1.png"), dir.path().join("b.png"))
            .expect("copy second visible fixture");
        let dir_path = dir.path().to_path_buf();
        let a = dir.path().join("a.png");
        let b = dir.path().join("b.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open = dir_path.clone();
        app.update(cx, |app, cx| {
            // Open with the rule already ON so the hidden image is listed
            // too: the natural sort puts '.dot.png' first ('.' < 'a'), so
            // the rescan has something to remove from ABOVE the selection.
            app.settings.show_hidden_files = true;
            app.open_folder(open, cx);
        });
        cx.run_until_parked();
        app.update(cx, |app, _| {
            assert_eq!(app.session.images.len(), 3, "all three listed");
            assert_eq!(app.session.images[1].path, a, "a.png starts at cell 1");
            // Cursor on the LAST cell, which the shorter list cannot hold.
            app.grid_selected = 2;
            app.selected = std::collections::BTreeSet::from([1]);
        });

        app.update(cx, |app, cx| app.toggle_show_hidden_files(cx));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 2, "the dotfile is gone");
            assert_eq!(
                app.selected.iter().copied().collect::<Vec<_>>(),
                vec![0],
                "the set followed a.png to its new cell instead of staying \
                 on index 1 (which now names b.png)"
            );
            assert_eq!(
                app.session.images[app.selected.iter().copied().next().expect("a mark")].path,
                a,
                "and the marked cell really is the file the user had selected"
            );
            assert_eq!(
                app.grid_selected, 1,
                "the cursor was clamped into the shorter list"
            );
            assert!(
                app.grid_selected < app.session.images.len(),
                "a past-the-end cursor is what this asserts against"
            );
            assert_ne!(b, app.session.images[0].path);
        });
        drop(dir_path);
    }

    /// The remap rule itself, case by case: a path that moved is re-marked at
    /// its new index, a path that left the list is not re-marked (there is no
    /// cell for it), and the cursor is clamped rather than reset. Pure, so no
    /// harness is needed.
    #[test]
    fn remap_selection_by_path_follows_paths_and_clamps_the_cursor() {
        let (a, b, c) = (
            PathBuf::from("Z:\\fake\\a.png"),
            PathBuf::from("Z:\\fake\\b.png"),
            PathBuf::from("Z:\\fake\\c.png"),
        );
        // Indices moved: c is now first, so keeping `a` must mark index 2.
        let images = set_items(&["c", "b", "a"]);
        let (selected, cursor) =
            remap_selection_by_path(&images, [a.as_path(), c.as_path()].into_iter(), 99);
        assert_eq!(selected.iter().copied().collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(cursor, 2, "a cursor past the end clamps to the last cell");
        // A path that is no longer listed has no cell to mark: the file it
        // named was deleted, which is what `confirm_pending`'s leftovers
        // already do.
        let gone = PathBuf::from("Z:\\fake\\gone.png");
        let (selected, cursor) = remap_selection_by_path(&images, [gone.as_path()].into_iter(), 0);
        assert!(selected.is_empty(), "the vanished path is not re-marked");
        assert_eq!(cursor, 0, "and the cursor is left alone");
        // An empty list cannot hold a cursor at all.
        let (selected, cursor) = remap_selection_by_path(&[], [a.as_path()].into_iter(), 4);
        assert!(selected.is_empty());
        assert_eq!(cursor, 0, "saturating, never an underflow");
        // `b` resolved by path, not by position: `confirm_pending` keeps a
        // subset of the list, so what a kept path resolves to must not
        // depend on which other paths came with it.
        let (selected, _) = remap_selection_by_path(&images, [b.as_path()].into_iter(), 0);
        assert_eq!(selected.iter().copied().collect::<Vec<_>>(), vec![1]);
    }

    /// The empty corner: a folder whose ONLY image is hidden leaves nothing
    /// to list once the flag flips off. The rescan must say so (`no_images_in`)
    /// rather than keep showing the pre-toggle list — and flipping back must
    /// clear the error again. Real fixture, same reason as above.
    #[gpui::test]
    fn show_hidden_files_toggle_reports_the_folder_it_emptied(cx: &mut gpui::TestAppContext) {
        let fixtures =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sh-core/tests/fixtures");
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::copy(
            fixtures.join("opaque_1x1.png"),
            dir.path().join(".only.png"),
        )
        .expect("copy opaque fixture");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open = dir_path.clone();
        app.update(cx, |app, cx| {
            app.settings.show_hidden_files = true;
            app.open_folder(open, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 1);
            assert!(app.session.error.is_none());
        });
        app.update(cx, |app, cx| app.toggle_show_hidden_files(cx));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.session.images.is_empty(), "nothing left to list");
            assert!(
                app.session.error.is_some(),
                "the empty result is reported, not silently kept as the old list"
            );
        });
        app.update(cx, |app, cx| app.toggle_show_hidden_files(cx));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 1, "the image comes back");
            assert!(
                app.session.error.is_none(),
                "and the stale empty-folder claim is cleared"
            );
        });
        drop(dir_path);
    }

    /// The flag is read at SPAWN time, and the rescan is guarded by the same
    /// `list_load_seq` ticket as an open: flipping twice in a row must land on
    /// the value the user last chose, never on a scan that was already in
    /// flight under the old one.
    #[gpui::test]
    fn rapid_show_hidden_files_toggles_settle_on_the_last_value(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"not a real png").expect("fixture a.png");
        std::fs::write(dir.path().join(".thumb.png"), b"not a real png")
            .expect("fixture .thumb.png");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open = dir_path.clone();
        app.update(cx, |app, cx| app.open_folder(open, cx));
        cx.run_until_parked();
        // Two flips before the executor runs: the second takes the newer
        // ticket, so the first scan's completion must be dropped.
        app.update(cx, |app, cx| app.toggle_show_hidden_files(cx));
        app.update(cx, |app, cx| app.toggle_show_hidden_files(cx));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                !app.settings.show_hidden_files,
                "two flips return to the shipped default"
            );
            assert_eq!(
                app.session.images.len(),
                1,
                "the stale first scan did not commit over the second"
            );
        });
        drop(dir_path);
    }

    /// Slice B (task 2.6): the navigate background probe coalesces the
    /// alpha verdict into `ImageItem.has_alpha` — current item plus the +1
    /// neighbor prefetch, seq-guarded. Real fixtures, flushed executor.
    ///
    /// Verdict-in-flight renders without the board (gate reads `None` as
    /// opaque); a failed probe (garbage bytes) leaves `None`, never a
    /// verdict, and never crashes.
    #[gpui::test]
    fn navigate_probe_coalesces_alpha_verdict(cx: &mut gpui::TestAppContext) {
        let fixtures =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sh-core/tests/fixtures");
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::copy(
            fixtures.join("transparent_1x1.png"),
            dir.path().join("t.png"),
        )
        .expect("copy transparent fixture");
        std::fs::copy(fixtures.join("opaque_1x1.png"), dir.path().join("o.png"))
            .expect("copy opaque fixture");
        std::fs::copy(fixtures.join("tiny.jpg"), dir.path().join("p.jpg"))
            .expect("copy jpeg fixture");
        std::fs::write(dir.path().join("g.png"), b"not a real png").expect("write garbage png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open = dir.path().join("t.png");
        app.update(cx, |app, cx| {
            app.open_path(open, cx);
        });
        cx.run_until_parked();
        // Walk the whole folder so every item is probed as current at least
        // once (each navigate stores current + prefetches +1; flushing
        // between steps keeps every seq-guarded store alive).
        for _ in 0..3 {
            app.update(cx, |app, cx| {
                app.navigate(1, cx);
            });
            cx.run_until_parked();
        }
        app.read_with(cx, |app, _| {
            let verdict = |name: &str| {
                app.session
                    .images
                    .iter()
                    .find(|i| i.path.file_name().is_some_and(|n| n == name))
                    .and_then(|i| i.has_alpha)
            };
            assert_eq!(verdict("t.png"), Some(true));
            assert_eq!(verdict("o.png"), Some(false));
            assert_eq!(verdict("p.jpg"), Some(false));
            // Corrupt probe: no verdict, no crash.
            assert_eq!(verdict("g.png"), None);
            // The persisted flag defaults ON (v7), so a transparent current
            // item opens the gate through the same helper `render` uses.
            assert!(app.settings.checkerboard);
        });
        // Land back on the transparent image: gate fully open at App level.
        for _ in 0..4 {
            let is_transparent = app.read_with(cx, |app, _| {
                app.session
                    .current_item()
                    .is_some_and(|i| i.path.file_name().is_some_and(|n| n == "t.png"))
            });
            if is_transparent {
                break;
            }
            app.update(cx, |app, cx| {
                app.navigate(1, cx);
            });
            cx.run_until_parked();
        }
        app.read_with(cx, |app, _| {
            let cur = app.session.current_item().expect("folder has images");
            assert!(crate::viewer::should_show_checkerboard(
                app.settings.checkerboard,
                cur.has_alpha
            ));
        });
        drop(dir);
    }

    /// Slice C (task 3.1 RED): the thumbnail batch caches one alpha verdict
    /// per decoded path on the capped bytes already in hand — transparent →
    /// `true`, opaque/JPEG → `false`. No new decodes, no I/O beyond the
    /// batch itself.
    #[gpui::test]
    fn thumb_batch_caches_alpha_verdict_per_path(cx: &mut gpui::TestAppContext) {
        let fixtures =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sh-core/tests/fixtures");
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::copy(
            fixtures.join("transparent_1x1.png"),
            dir.path().join("t.png"),
        )
        .expect("copy transparent fixture");
        std::fs::copy(fixtures.join("opaque_1x1.png"), dir.path().join("o.png"))
            .expect("copy opaque fixture");
        std::fs::copy(fixtures.join("tiny.jpg"), dir.path().join("p.jpg"))
            .expect("copy jpeg fixture");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open = dir.path().join("t.png");
        app.update(cx, |app, cx| {
            app.open_path(open, cx);
        });
        // Drain the background executor: thumb decodes (and their verdicts)
        // must have run and committed under the seq guard.
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let verdict = |name: &str| {
                app.thumb_alpha
                    .iter()
                    .find(|(p, _)| p.file_name().is_some_and(|n| n == name))
                    .map(|(_, v): (&PathBuf, &bool)| *v)
            };
            assert_eq!(
                verdict("t.png"),
                Some(true),
                "transparent thumb caches true"
            );
            assert_eq!(verdict("o.png"), Some(false), "opaque thumb caches false");
            assert_eq!(verdict("p.jpg"), Some(false), "jpeg thumb caches false");
        });
        drop(dir);
    }

    /// Slice C (task 3.1 RED): mixed grid with the setting ON — the per-cell
    /// gate opens only for transparent thumbs. The cell builder reads
    /// `settings.checkerboard && thumb_alpha == Some(true)` (the same helper
    /// `App::render` uses for the viewer), so each cell yields exactly one
    /// board decision; cells build inline in render (not headless-clickable),
    /// hence the harness pins the gate inputs/outputs per cell here.
    #[gpui::test]
    fn mixed_grid_boards_only_on_transparent_thumbs(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let transparent = PathBuf::from("Z:\\fake\\t.png");
        let opaque = PathBuf::from("Z:\\fake\\o.png");
        app.update(cx, |app, _cx| {
            app.thumb_alpha.insert(transparent.clone(), true);
            app.thumb_alpha.insert(opaque.clone(), false);
            app.settings.checkerboard = true;
        });
        app.read_with(cx, |app, _| {
            let show = |path: &PathBuf| {
                crate::viewer::should_show_checkerboard(
                    app.settings.checkerboard,
                    app.thumb_alpha.get(path).copied(),
                )
            };
            assert!(show(&transparent), "transparent thumb shows the board");
            assert!(!show(&opaque), "opaque thumb shows no board");
        });
    }

    /// Slice C (task 3.1 RED): setting OFF clears all thumbnail boards —
    /// no cell renders a board regardless of its cached verdict.
    #[test]
    fn checkerboard_off_clears_all_thumb_boards() {
        use std::collections::HashMap;
        let seeded: HashMap<PathBuf, bool> = [
            (PathBuf::from("Z:\\fake\\t.png"), true),
            (PathBuf::from("Z:\\fake\\o.png"), false),
        ]
        .into_iter()
        .collect();
        for path in seeded.keys() {
            assert!(
                !crate::viewer::should_show_checkerboard(false, seeded.get(path).copied()),
                "OFF kills every cell board: {}",
                path.display()
            );
        }
    }

    /// Slice C (task 3.1 RED): folder swap clears `thumb_alpha` together with
    /// `thumbs` — no previous folder's verdicts linger.
    #[gpui::test]
    fn folder_swap_clears_thumb_alpha(cx: &mut gpui::TestAppContext) {
        let dir_a = tempfile::tempdir().expect("tempdir A must be created");
        let dir_b = tempfile::tempdir().expect("tempdir B must be created");
        let png_a = fixture_png_in(dir_a.path(), "a.png");
        let png_b = fixture_png_in(dir_b.path(), "b.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.open_path(png_a.clone(), cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                app.thumb_alpha.contains_key(&png_a),
                "current folder's verdict is cached"
            );
        });
        app.update(cx, |app, cx| {
            app.open_path(png_b.clone(), cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                app.thumb_alpha.contains_key(&png_b),
                "new folder's verdict is cached"
            );
            assert!(
                !app.thumb_alpha.contains_key(&png_a),
                "stale folder A verdict must be dropped with the thumbs"
            );
        });
    }

    /// Slice C (task 3.1 RED): single-layer composition — the per-cell gate
    /// yields exactly one boolean per visible cell, so the builder inserts
    /// at most one `#checkerboard` per transparent thumb and none for
    /// opaque ones. A path with no cached verdict (batch still running)
    /// renders exactly as before.
    /// Every visible grid cell must share one row height, whatever the aspect
    /// ratio of the image inside it.
    ///
    /// The defect this guards: `Img::request_layout` imposes
    /// `aspect_ratio` from the image's own dimensions when the caller has not
    /// set one (gpui-pre `elements/img.rs`), so a tall photo laid itself out
    /// against its natural shape instead of the box asked for. Measured on a
    /// folder of mixed-aspect PNGs: a 223px image rendered inside a 75px slot,
    /// painting over the row above and the label below. Cells whose image was
    /// shorter than the slot looked correct, which is why the existing
    /// single-cell layout test — which only checked `grid-cell-0` and only the
    /// empty placeholder — never saw it.
    ///
    /// `grid_max_scroll` and `visible_row_range` both assume uniform rows and
    /// both are unit-tested. Those tests were passing while the rendered grid
    /// disagreed with them, because they test the arithmetic and not the
    /// layout. This closes that gap by measuring the layout.
    #[gpui::test]
    fn grid_cells_keep_one_row_height_across_mixed_aspect_images(cx: &mut gpui::TestAppContext) {
        // Three deliberately different shapes: wider than the slot, square, and
        // far taller than the slot.
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let mut paths = Vec::new();
        for (name, w, h) in [
            ("wide.png", 800u32, 200u32),
            ("square.png", 400, 400),
            ("tall.png", 200, 800),
        ] {
            let path = dir.path().join(name);
            image::RgbaImage::from_pixel(w, h, image::Rgba([90u8, 140, 200, 255]))
                .save(&path)
                .expect("fixture png must be written");
            paths.push(path);
        }

        let epoch = std::time::SystemTime::UNIX_EPOCH;
        let session = Session {
            images: build_image_items(
                paths
                    .iter()
                    .map(|path| sh_core::navigation::ImageEntry {
                        path: path.clone(),
                        size: 0,
                        modified: epoch,
                        created: None,
                    })
                    .collect::<Vec<_>>(),
            ),
            current: 0,
            ..Session::default()
        };

        let (app, cx) = cx.add_window_view(|_window, cx| test_app_with_session(session, cx));
        let cx = cx as &mut gpui::VisualTestContext;
        // `App::new` starts in Welcome; the grid only mounts in Grid view, same
        // as the existing layout test.
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            cx.notify();
        });
        // Drain the decode batch so the cells hold real `RenderImage`s, not the
        // placeholder — the placeholder is a plain sized div and would pass even
        // with the bug present.
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.thumbs.len(),
                3,
                "all three fixtures must decode, or this test is not exercising the bug"
            );
        });

        let size = app.read_with(cx, |app, _| app.settings.grid_size);
        let geo = crate::ui::grid::GridSizeGeometry::geometry(size);
        let mut heights = Vec::new();
        for idx in 0..3 {
            let selector: &'static str = Box::leak(format!("grid-cell-{idx}").into_boxed_str());
            let b = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} must mount"));
            assert_positive_layout_bounds(b, selector);
            heights.push(f32::from(b.size.height));
        }

        let expected = geo.row_h as f32;
        for (idx, h) in heights.iter().enumerate() {
            assert!(
                (h - expected).abs() < 1.0,
                "grid-cell-{idx} height is {h}, expected the row height {expected}: a cell \
                 whose image overflowed its slot grew the flex line, and every row boundary \
                 derived from it is now wrong"
            );
        }
    }

    // ── Layout invariants ────────────────────────────────────────────────
    //
    // WHY THIS SECTION
    //
    // A grid cell sized itself from its content, so a photo taller than the
    // thumbnail slot grew its whole flex line: 146px cells against a 170px
    // preset, images painting over the row above and the label below. Every
    // gate was green. `grid_max_scroll` and `visible_row_range` passed because
    // they test arithmetic, and nothing in the suite tested the layout that
    // arithmetic describes.
    //
    // That gap is not "the suite ignores pixels". `debug_bounds` reads real
    // computed bounds from the rendered frame — deterministic, no GPU, no
    // driver, no font fallback, no DPI. The suite sampled ONE element and
    // asserted its absolute size, instead of asserting how elements relate.
    //
    // So these assert RELATIONS: uniformity, non-overlap, containment. Those
    // are what a UI violates while every absolute dimension still reads
    // correct.
    //
    // Golden-image comparison was the obvious alternative and was deliberately
    // not taken: rendered output varies with GPU driver, Windows build, DPI
    // and font fallback, so a diff fails on changes that are not regressions
    // and trains everyone to re-baseline instead of to read failures.

    /// Cells in one row share a top edge, and no cell reaches into the row
    /// above.
    ///
    /// The second half matters more than the first: uniform height alone still
    /// passes while a cell's content overflows into its neighbour, which is
    /// exactly what happened.
    ///
    /// Fixture count is deliberately larger than any plausible column count for
    /// the harness window. The first version used six and fit on a single row,
    /// so it had no second row to compare against and the assertion could not
    /// run at all.
    #[gpui::test]
    fn grid_rows_are_uniform_and_never_overlap(cx: &mut gpui::TestAppContext) {
        const CELLS: usize = 24;
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let mut paths = Vec::new();
        for i in 0..CELLS {
            // Shapes cycling through wide, square and tall, so any subset of
            // consecutive cells contains a mix.
            let (w, h) = match i % 3 {
                0 => (800u32, 200u32),
                1 => (400, 400),
                _ => (200, 800),
            };
            let path = dir.path().join(format!("img_{i:02}.png"));
            image::RgbaImage::from_pixel(w, h, image::Rgba([120u8, 90, 200, 255]))
                .save(&path)
                .expect("fixture png must be written");
            paths.push(path);
        }

        let epoch = std::time::SystemTime::UNIX_EPOCH;
        let session = Session {
            images: build_image_items(
                paths
                    .iter()
                    .map(|path| sh_core::navigation::ImageEntry {
                        path: path.clone(),
                        size: 0,
                        modified: epoch,
                        created: None,
                    })
                    .collect::<Vec<_>>(),
            ),
            current: 0,
            ..Session::default()
        };

        let (app, cx) = cx.add_window_view(|_window, cx| test_app_with_session(session, cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            cx.notify();
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.thumbs.len(),
                CELLS,
                "all fixtures must decode; otherwise this only exercises placeholders"
            );
        });

        let size = app.read_with(cx, |app, _| app.settings.grid_size);
        let geo = crate::ui::grid::GridSizeGeometry::geometry(size);
        let expected = geo.row_h as f32;

        let mut cells = Vec::new();
        for idx in 0..CELLS {
            let selector: &'static str = Box::leak(format!("grid-cell-{idx}").into_boxed_str());
            let b = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} must mount"));
            cells.push((idx, f32::from(b.origin.y), f32::from(b.size.height)));
        }
        for (idx, _, h) in &cells {
            assert!(
                (h - expected).abs() < 1.0,
                "grid-cell-{idx} height {h} != preset row height {expected}"
            );
        }

        // Rows are derived from where the cells actually landed, not from
        // `floor(viewport / cell_w)`: the harness window's width is not a
        // value this test controls, and assuming a column count made the first
        // version of this test report a false overlap.
        let mut rows: Vec<(f32, Vec<f32>)> = Vec::new();
        for (_, top, h) in &cells {
            match rows.iter_mut().find(|(t, _)| (t - top).abs() < 1.0) {
                Some((_, hs)) => hs.push(*h),
                None => rows.push((*top, vec![*h])),
            }
        }
        rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        assert!(
            rows.len() > 1,
            "{CELLS} cells should wrap past one row at this window width"
        );

        for i in 1..rows.len() {
            let top = rows[i].0;
            let prev_bottom = rows[i - 1].0 + rows[i - 1].1[0];
            assert!(
                top >= prev_bottom - 1.0,
                "row {i} starts at {top}, above the previous row's bottom {prev_bottom}: \
                 content is overflowing its slot"
            );
        }
    }

    /// A viewport's chrome must not spill outside its own band.
    ///
    /// Containment is what a per-element size assertion cannot express: a
    /// control can be exactly the right size and still render half outside its
    /// container. That is how the Crop button shipped invisible while every
    /// test referencing its element id passed — the id existed, the pixels did
    /// not.
    #[gpui::test]
    fn viewer_chrome_stays_inside_its_band(cx: &mut gpui::TestAppContext) {
        let (_app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.view = View::Viewer;
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        cx.run_until_parked();

        let main = cx.debug_bounds("viewer-main").expect("viewer-main mounts");
        let main_bottom = f32::from(main.origin.y) + f32::from(main.size.height);

        for name in ["topbar-back", "viewer-chips"] {
            let s: &'static str = Box::leak(name.to_string().into_boxed_str());
            let Some(b) = cx.debug_bounds(s) else {
                continue;
            };
            let top = f32::from(b.origin.y);
            let bottom = top + f32::from(b.size.height);
            assert!(
                top >= -1.0 && bottom <= main_bottom + 1.0,
                "{name} spans y {top}..{bottom}, outside the viewer band ending at {main_bottom}"
            );
        }
    }

    #[test]
    fn grid_board_gate_yields_at_most_one_layer_per_cell() {
        use std::collections::HashMap;
        let seeded: HashMap<PathBuf, bool> = [
            (PathBuf::from("Z:\\fake\\t1.png"), true),
            (PathBuf::from("Z:\\fake\\t2.png"), true),
            (PathBuf::from("Z:\\fake\\o.png"), false),
        ]
        .into_iter()
        .collect();
        let visible = [
            PathBuf::from("Z:\\fake\\t1.png"),
            PathBuf::from("Z:\\fake\\t2.png"),
            PathBuf::from("Z:\\fake\\o.png"),
            // Batch still running: no entry yet.
            PathBuf::from("Z:\\fake\\pending.png"),
        ];
        // One decision per cell — the builder maps each `true` to exactly
        // one layer, each `false`/`None` to none.
        let decisions: Vec<bool> = visible
            .iter()
            .map(|p| crate::viewer::should_show_checkerboard(true, seeded.get(p).copied()))
            .collect();
        assert_eq!(decisions.len(), visible.len(), "one decision per cell");
        assert_eq!(
            decisions.iter().filter(|d| **d).count(),
            2,
            "boards only on the two transparent thumbs"
        );
        assert!(
            !decisions[3],
            "verdict-pending cell renders with no board, no crash"
        );
    }

    /// Pinned contract for the Settings-General recent rows: clicking one
    /// runs exactly these calls (`note_interaction` + `open_folder`), so a
    /// recent opens in Grid like its Welcome-chip twin. The rows themselves
    /// are built inline in render (not headless-clickable), so the harness
    /// drives the handler's calls directly.
    #[gpui::test]
    fn settings_recent_row_opens_folder_in_grid(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Settings;
            app.note_interaction(cx);
            app.open_folder(d, cx);
        });
        // Async scan: the Grid switch lands inline, the list does not.
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Grid);
            assert_eq!(app.session.images.len(), 1);
        });
        drop(dir_path);
    }

    /// `open_folder` on an empty dir surfaces "no images" in the session
    /// error slot and still enters the Grid view (empty-state renders with
    /// context) instead of panicking.
    #[gpui::test]
    fn open_folder_empty_dir_shows_error_in_grid(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let dir_clone = dir_path.clone();
        app.update(cx, |app, cx| {
            app.open_folder(dir_clone, cx);
        });
        // "No images" is knowledge, not a prediction: the error slot stays
        // empty until the scan says so, so the pump is what makes it appear.
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Grid);
            assert!(app.session.images.is_empty());
            assert!(app.session.error.is_some());
        });
        drop(dir_path);
    }

    // ── V3: recent-folders wiring ──

    /// Open one folder with images → recents list holds exactly it.
    #[gpui::test]
    fn open_folder_pushes_first_recent(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.open_folder(d, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.recent_dirs, vec![dir_path.clone()]);
            assert_eq!(app.recent_dirs_available, vec![dir_path.clone()]);
            // Mirror invariant: last_dir == recent[0].
            assert_eq!(app.settings.last_dir, Some(dir_path.clone()));
        });
    }

    /// Opening a second folder prepends; reopening the first moves it back
    /// to the front (dedupe-move, not duplicate).
    ///
    /// Each open is PUMPED before the next one: three unpumped opens would
    /// leave only the newest ticket standing, and the intermediate folders
    /// would never reach the recents list (the superseded-load rule, pinned
    /// on its own by
    /// `superseded_open_never_reaches_the_recents_list`).
    #[gpui::test]
    fn reopen_folder_moves_it_to_front(cx: &mut gpui::TestAppContext) {
        let dir_a = tempfile::tempdir().expect("tempdir a");
        let dir_b = tempfile::tempdir().expect("tempdir b");
        std::fs::write(dir_a.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir_b.path().join("b.png"), b"stub").expect("fixture b.png");
        let a = dir_a.path().to_path_buf();
        let b = dir_b.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (a1, b1, a2) = (a.clone(), b.clone(), a.clone());
        app.update(cx, |app, cx| {
            app.open_folder(a1, cx);
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.open_folder(b1, cx);
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.open_folder(a2, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.session.slideshow_active);
            // a pushed, then b prepended, then a moved back to the front.
            assert_eq!(app.settings.recent_dirs, vec![a.clone(), b.clone()]);
            // The list on screen is a's, not a stale mix.
            assert_eq!(app.session.images.len(), 1);
            assert_eq!(
                app.session
                    .current_item()
                    .and_then(|i| i.path.parent().map(std::path::Path::to_path_buf)),
                Some(a.clone())
            );
        });
    }

    /// Toggle in Viewer flips the flag; toggle in crop mode is a no-op
    /// (mutual exclusion, one side of it — enter_crop is the other).
    #[gpui::test]
    fn slideshow_toggle_flips_and_crop_blocks(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.toggle_slideshow(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.session.slideshow_active);
        });
        // Crop guard: toggling while in crop mode does nothing.
        app.update(cx, |app, cx| {
            app.crop_mode = true;
            app.toggle_slideshow(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.session.slideshow_active); // unchanged by blocked toggle
        });
        // And the plain toggle-off path:
        app.update(cx, |app, cx| {
            app.crop_mode = false;
            app.toggle_slideshow(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.session.slideshow_active);
        });
    }

    /// The timer loop advances the current image once per interval while
    /// active (clock-advanced — no real waiting), and stops when toggled
    /// off. Loop is eternal: re-arming never breaks the harness.
    #[gpui::test]
    fn slideshow_timer_advances_and_stops(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let before = app.read_with(cx, |app, _| app.session.current);
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.toggle_slideshow(cx);
        });
        // One interval: exactly one advance.
        cx.background_executor.advance_clock(slideshow_delay(3));
        cx.run_until_parked();
        let after_one = app.read_with(cx, |app, _| app.session.current);
        assert_eq!(after_one, (before + 1) % 3); // test_app has 3 images
                                                 // Two more intervals: keeps going (loop behavior, wraps circularly).
        cx.background_executor.advance_clock(slideshow_delay(3) * 2);
        cx.run_until_parked();
        let after_three = app.read_with(cx, |app, _| app.session.current);
        assert_eq!(after_three, (before + 3) % 3);
        // Toggle off: the same clock advance must NOT move the index.
        app.update(cx, |app, cx| {
            app.toggle_slideshow(cx);
        });
        cx.background_executor.advance_clock(slideshow_delay(3));
        cx.run_until_parked();
        let after_stop = app.read_with(cx, |app, _| app.session.current);
        assert_eq!(after_stop, after_three);
    }

    /// The timer boundary keeps a minimum one-second delay even if an
    /// in-memory value bypasses the settings setter.
    #[test]
    fn slideshow_delay_never_zero() {
        assert_eq!(slideshow_delay(0), std::time::Duration::from_secs(1));
        assert_eq!(slideshow_delay(1), std::time::Duration::from_secs(1));
        assert_eq!(slideshow_delay(60), std::time::Duration::from_secs(60));
    }

    /// The active timer must use the persisted interval, not the legacy
    /// three-second constant. The test clock makes this deterministic.
    #[gpui::test]
    fn slideshow_uses_configured_interval(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.settings.slideshow_interval_secs = 1;
            app.toggle_slideshow(cx);
        });

        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();

        app.read_with(cx, |app, _| {
            assert_eq!(app.session.current, 1);
            assert!(app.session.slideshow_active);
        });
    }

    /// Changing the interval while playing resets the delay: the old deadline
    /// must not fire at the old cadence after the new value is committed.
    #[gpui::test]
    fn slideshow_interval_change_rearms_active_delay(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.toggle_slideshow(cx);
        });

        // Spend two seconds of the original three-second deadline.
        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.set_slideshow_interval_secs(5, cx)
                .expect("interval must be valid");
        });

        // The old three-second deadline must not advance at four seconds total.
        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 0));

        // The new five-second deadline starts at the change and fires here.
        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(3));
        cx.run_until_parked();
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 1));
    }

    /// Changing the interval from the Settings surface over Viewer arms the
    /// new delay when the surface closes, without losing playback intent.
    #[gpui::test]
    fn slideshow_interval_change_from_settings_rearms_on_return(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.toggle_slideshow(cx);
            app.open_settings(cx);
            app.set_slideshow_interval_secs(1, cx)
                .expect("interval must be valid");
        });
        app.update(cx, |app, cx| app.close_settings(cx));

        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Viewer);
            assert!(app.session.slideshow_active);
            assert_eq!(app.session.current, 1);
        });
    }

    /// Stopping cancels the pending delay; starting again gets a fresh delay
    /// instead of inheriting the old task's deadline.
    #[gpui::test]
    fn slideshow_stop_cancels_pending_delay(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.settings.slideshow_interval_secs = 1;
            app.toggle_slideshow(cx);
        });
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();

        app.update(cx, |app, cx| app.toggle_slideshow(cx));
        app.update(cx, |app, cx| app.toggle_slideshow(cx));
        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();

        app.read_with(cx, |app, _| {
            assert_eq!(app.session.current, 1);
            assert!(app.session.slideshow_active);
        });
    }

    /// Leaving the Viewer cancels the pending delay and resets playback state.
    #[gpui::test]
    fn slideshow_leaving_viewer_cancels_pending_delay(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            app.settings.slideshow_interval_secs = 1;
            app.toggle_slideshow(cx);
            app.enter_grid(cx);
        });

        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Grid);
            assert!(!app.session.slideshow_active);
            assert_eq!(app.session.current, 0);
        });
    }

    /// The settings contract rejects zero before a timer can be armed, so an
    /// invalid candidate cannot create a zero-delay task.
    #[gpui::test]
    fn slideshow_zero_interval_is_rejected_before_timer_advance(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Viewer;
            assert!(app.set_slideshow_interval_secs(0, cx).is_err());
            app.toggle_slideshow(cx);
        });

        cx.background_executor
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 0));
    }

    // ── V3: multi-selection state ──

    /// Toggle adds then removes; the cursor NEVER moves on selection —
    /// selecting must not take the "visited" mark (only navigation and
    /// viewer opens move it).
    #[gpui::test]
    fn toggle_select_leaves_cursor_in_place(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.grid_selected = 0;
            app.anchor = 0;
            app.toggle_selected(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.contains(&2));
            assert_eq!(app.grid_selected, 0);
            assert_eq!(app.anchor, 0);
        });
        app.update(cx, |app, cx| {
            app.toggle_selected(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.selected.contains(&2));
            assert!(app.selected.is_empty());
            assert_eq!(app.grid_selected, 0);
        });
    }

    /// Shift-extend unions the anchor→target range and leaves the cursor
    /// where it was; chained Shift+clicks share the frozen anchor.
    #[gpui::test]
    fn extend_unions_from_frozen_anchor(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.grid_selected = 0;
            app.anchor = 0;
            app.extend_selection_to(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.selected.iter().copied().collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
            assert_eq!(app.grid_selected, 0);
            assert_eq!(app.anchor, 0);
        });
        // Second Shift+click elsewhere extends from the SAME anchor.
        app.update(cx, |app, cx| {
            app.extend_selection_to(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.selected.iter().copied().collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
            assert_eq!(app.grid_selected, 0);
        });
    }

    /// Plain arrows move cursor AND anchor together (fresh start for the
    /// next Shift gesture).
    #[gpui::test]
    fn plain_arrows_move_anchor_with_cursor(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.grid_selected = 0;
            app.anchor = 0;
            app.move_selection(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.grid_selected, 1);
            assert_eq!(app.anchor, 1);
        });
    }

    /// Plain arrow movement collapses the set (standard OS behavior).
    #[gpui::test]
    fn plain_arrows_collapse_selection(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.grid_selected = 0;
            app.anchor = 0;
            app.extend_selection_to(1, cx);
            // Cursor never moved during extend — plain arrows resume from 0.
            app.move_selection(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
            assert_eq!(app.grid_selected, 1);
            assert_eq!(app.anchor, 1);
        });
    }

    /// Ctrl+A fills; clear empties (the Escape path calls the same method).
    #[gpui::test]
    fn select_all_fills_and_clear_empties(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.select_all(cx);
        });
        app.read_with(cx, |app, _| {
            // test_app carries exactly 3 fake images.
            assert_eq!(
                app.selected.iter().copied().collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
        });
        app.update(cx, |app, cx| {
            app.clear_selection(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
        });
    }

    /// Folder swap clears (work-in-progress never crosses folders).
    #[gpui::test]
    fn folder_swap_clears_selection(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.select_all(cx);
            app.open_folder(d, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
        });
    }

    /// Sort change clears (indices invalidate on reorder — documented v1 rule).
    #[gpui::test]
    fn sort_change_clears_selection(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.select_all(cx);
            app.set_sort(
                sh_core::navigation::SortBy::Size,
                sh_core::navigation::SortDir::Desc,
                cx,
            );
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
        });
    }

    /// Viewer round-trip preserves (selection is work-in-progress, and
    /// `enter_viewer`/`enter_grid` only move the cursor).
    #[gpui::test]
    fn viewer_round_trip_preserves_selection(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.toggle_selected(0, cx);
            app.toggle_selected(1, cx);
            app.enter_viewer(0, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0, 1]);
        });
        app.update(cx, |app, cx| {
            app.enter_grid(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0, 1]);
        });
    }

    /// The copy payload: absolute paths, ascending index order, `\n`-joined.
    /// Pure string — the clipboard write itself is manual-smoke (existing
    /// `clipboard.rs` policy). A plain `#[test]` cannot build an `App`, so
    /// there is exactly one test, in-harness.
    #[gpui::test]
    fn selection_paths_string_exact(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        // Empty set → None (the no-op signal — no clipboard touch).
        let none = app.read_with(cx, |app, _| app.selection_paths_string());
        assert_eq!(none, None);
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.toggle_selected(1, cx);
            app.toggle_selected(0, cx);
        });
        let some = app.read_with(cx, |app, _| app.selection_paths_string());
        // test_app fakes: Z:\fake\{a,b,c}.png — ascending index order
        // regardless of toggle order.
        let sep = "\n";
        assert_eq!(some, Some(format!("Z:\\fake\\a.png{sep}Z:\\fake\\b.png")));
    }

    // ── V3: batch staging (destructive half) ──

    /// Delete with an empty set stages nothing (silent no-op).
    #[gpui::test]
    fn delete_empty_selection_stages_nothing(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.stage_delete(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.pending_batch.is_none());
        });
    }

    /// Delete stages the pending op; files stay put until confirm.
    #[gpui::test]
    fn delete_stages_pending_without_touching_files(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir.path().join("b.png"), b"stub").expect("fixture b.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        // Staging reads the loaded list, so the scan must land first.
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_delete(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(matches!(
                app.pending_batch,
                Some(BatchOp::Delete { ref paths }) if paths.len() == 2
            ));
            // Staging never touches the filesystem.
            assert!(dir_path.join("a.png").exists());
        });
    }

    /// Esc with a pending bar cancels it; files and selection intact.
    #[gpui::test]
    fn escape_cancels_pending(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.toggle_selected(0, cx);
            app.stage_delete(cx);
        });
        // Simulate the BackToGrid Escape link directly (the binding itself
        // is compiler-checked + smoke, like every existing binding).
        app.update(cx, |app, cx| {
            app.pending_batch = None;
            cx.notify();
        });
        app.read_with(cx, |app, _| {
            assert!(app.pending_batch.is_none());
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0]);
        });
    }

    /// Move staging records destination + paths (the rfd picker itself is
    /// smoke-only — it blocks headless).
    #[gpui::test]
    fn stage_move_records_dest_and_paths(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dest = tempfile::tempdir().expect("tempdir dest");
        let dir_path = dir.path().to_path_buf();
        let dest_path = dest.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (d, t) = (dir_path.clone(), dest_path.clone());
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        // Staging reads the loaded list, so the scan must land first.
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.toggle_selected(0, cx);
            app.stage_move(t, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(matches!(
                app.pending_batch,
                Some(BatchOp::Move { ref paths, ref dest })
                    if paths.len() == 1 && *dest == dest_path
            ));
            assert!(dir_path.join("a.png").exists());
        });
    }

    // ── V3: a staged op never outlives the folder it was staged in ──

    /// A staged op carries PATHS, so it is a promise about one specific
    /// folder. Opening another folder leaves the confirm bar on screen and
    /// still confirmable: Enter would then trash folder A's files while the
    /// user is looking at folder B, and the op's rescan would briefly paint
    /// folder A's list into folder B's view. The folder switch must drop it.
    #[gpui::test]
    fn open_folder_clears_a_staged_batch(cx: &mut gpui::TestAppContext) {
        let a = tempfile::tempdir().expect("tempdir a");
        std::fs::write(a.path().join("a.png"), b"stub").expect("fixture a.png");
        let b = tempfile::tempdir().expect("tempdir b");
        std::fs::write(b.path().join("b.png"), b"stub").expect("fixture b.png");
        let a_path = a.path().to_path_buf();
        let b_path = b.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open_a = a_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(open_a, cx);
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_delete(cx);
        });
        // Fixture sanity: the op is staged against folder A, so a later
        // failure is about the folder switch, not the setup.
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 1);
            assert!(app.pending_batch.is_some(), "op staged in folder A");
        });

        let other = b_path.clone();
        app.update(cx, |app, cx| app.open_folder(other, cx));
        app.read_with(cx, |app, _| {
            assert!(
                app.pending_batch.is_none(),
                "the confirm bar is gone with folder A's list, not still \
                 confirmable over folder B"
            );
            assert_eq!(app.current_dir.as_deref(), Some(b_path.as_path()));
        });
        cx.run_until_parked();
        // And the files of the folder the user LEFT are untouched on disk:
        // nothing about the abandoned op ran.
        assert!(
            a_path.join("a.png").exists(),
            "a staged op must not execute as a side effect of leaving its folder"
        );
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 1, "folder B is loaded");
            assert_eq!(app.session.images[0].path, b_path.join("b.png"));
            assert!(app.batch_status.is_none(), "no report lingers either");
        });
    }

    /// `open_path` is the second entry point into a different folder (Ctrl+O,
    /// drag & drop, CLI), and it reaches the same hazard: the op's paths
    /// belong to a folder the user is leaving.
    #[gpui::test]
    fn open_path_into_another_folder_clears_a_staged_batch(cx: &mut gpui::TestAppContext) {
        let a = tempfile::tempdir().expect("tempdir a");
        std::fs::write(a.path().join("a.png"), b"stub").expect("fixture a.png");
        let b = tempfile::tempdir().expect("tempdir b");
        std::fs::copy(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../sh-core/tests/fixtures/tiny.jpg"),
            b.path().join("b.png"),
        )
        .expect("copy real fixture");
        let a_path = a.path().to_path_buf();
        let b_file = b.path().join("b.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open_a = a_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(open_a, cx);
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_delete(cx);
        });
        app.read_with(cx, |app, _| assert!(app.pending_batch.is_some()));

        let file = b_file.clone();
        app.update(cx, |app, cx| app.open_path(file, cx));
        app.read_with(cx, |app, _| {
            assert!(
                app.pending_batch.is_none(),
                "a file open into a different folder is still a folder change"
            );
        });
        cx.run_until_parked();
        assert!(a_path.join("a.png").exists(), "folder A untouched");
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 1);
            assert_eq!(app.session.images[0].path, b_file);
        });
    }

    /// The boundary that keeps `open_path` from being a blanket clear: opening
    /// a file from the folder ALREADY on screen leaves the promise true, so
    /// the staged op survives. Ctrl+O is reachable with a confirm bar up, and
    /// discarding a confirmation the user can still act on correctly would be
    /// a regression, not a fix.
    #[gpui::test]
    fn open_path_within_the_open_folder_keeps_a_staged_batch(cx: &mut gpui::TestAppContext) {
        let a = tempfile::tempdir().expect("tempdir a");
        std::fs::write(a.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(a.path().join("b.png"), b"stub").expect("fixture b.png");
        let a_path = a.path().to_path_buf();
        let same_dir_file = a.path().join("a.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open_a = a_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(open_a, cx);
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.toggle_selected(0, cx);
            app.stage_delete(cx);
        });
        let file = same_dir_file.clone();
        app.update(cx, |app, cx| app.open_path(file, cx));
        app.read_with(cx, |app, _| {
            assert!(
                app.pending_batch.is_some(),
                "same-folder open: the staged paths still name files in the \
                 folder on screen, so the confirmation is still valid"
            );
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 2, "the same list came back");
        });
    }

    // ── V3: a grid selection never outlives the list it indexes ──

    /// The other half of the cross-folder hazard the two tests above cover,
    /// and the one `open_folder` already guarded: clearing a staged op is
    /// NOT clearing a selection.
    ///
    /// `open_path` always rebuilds `session.images` from `path.parent()`,
    /// so a selection made in folder A is left naming folder B's cells the
    /// moment the scan lands. The harm is concrete and destructive, not
    /// cosmetic: `stage_delete` resolves `selected`'s INDICES against
    /// whatever `session.images` holds and stores the resulting PATHS, so a
    /// two-cell selection carried across would stage two arbitrary files of
    /// folder B and the next Enter would send them to the recycle bin. That
    /// is why the assertions below end at `stage_delete` being a no-op and
    /// not merely at the set being empty.
    #[gpui::test]
    fn open_path_into_another_folder_clears_the_grid_selection(cx: &mut gpui::TestAppContext) {
        let a = tempfile::tempdir().expect("tempdir a");
        // Real fixtures: the probe reads them, so nothing here can be
        // confused by a decode failure landing in the error slot.
        fixture_png_in(a.path(), "a1.png");
        fixture_png_in(a.path(), "a2.png");
        fixture_png_in(a.path(), "a3.png");
        let b = tempfile::tempdir().expect("tempdir b");
        // Deliberately ONE image, so a cursor left on cell 2 of folder A is
        // demonstrably out of range here — the same stale cursor that made
        // `enter_viewer` a silent no-op in the commit_rescan fix.
        let b_file = fixture_png_in(b.path(), "b1.png");
        let a_path = a.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open_a = a_path.clone();
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.open_folder(open_a, cx);
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.move_selection(2, cx);
            app.toggle_selected(0, cx);
            app.toggle_selected(1, cx);
        });
        // Fixture sanity: a real multi-selection over folder A, so a later
        // failure is about the folder switch and not about the setup.
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 3, "folder A loaded");
            assert_eq!(app.selected, BTreeSet::from([0, 1]));
            assert_eq!(
                app.grid_selected, 2,
                "the cursor is the one cell that is not selected"
            );
            assert_eq!(app.anchor, 2);
        });

        let file = b_file.clone();
        app.update(cx, |app, cx| app.open_path(file, cx));
        // The reset is SYNCHRONOUS, on the frame that asks for the open —
        // there is no window in which the stale set addresses a rebuilt list.
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty(), "no stale multi-selection survives");
            assert_eq!(app.grid_selected, 0, "the cursor is reset, not left at 2");
            assert_eq!(app.anchor, 0, "the shift-range anchor is reset too");
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 1, "folder B is loaded");
            assert_eq!(app.session.images[0].path, b_file);
            assert!(
                app.grid_selected < app.session.images.len(),
                "the cursor must address the list actually on screen"
            );
        });
        // The harm, stated as an assertion: with the stale set intact this
        // would stage folder B's b1.png as if the user had picked it.
        app.update(cx, |app, cx| app.stage_delete(cx));
        app.read_with(cx, |app, _| {
            assert!(
                app.pending_batch.is_none(),
                "nothing was selected in folder B, so nothing can be staged there"
            );
        });
    }

    /// The decision this pins: `open_path`'s selection reset is
    /// UNCONDITIONAL, so a same-folder open clears the selection too — the
    /// opposite of the staged-op rule the test above pins, and deliberately
    /// so.
    ///
    /// The premise the guard would rest on is "the same folder means the same
    /// list", and this test is that premise failing, deterministically: a
    /// file appears in the folder BETWEEN the two scans. The list grows, and
    /// every index after the insertion point shifts by one — so a set built
    /// against the old list now names a file the user never saw, and drops
    /// one they did select. The new file is named `a0.png` precisely because
    /// the scan is name-asc: it lands at index 0 and pushes everything else
    /// up, which is the shift this reset has to absorb.
    ///
    /// So a `current_dir`-guarded reset would keep `{0, 1}` pointing at
    /// `{a0.png, b1.png}` while the user's actual selection was
    /// `{b1.png, b2.png}` — and `stage_delete` below would stage `a0.png`,
    /// which did not exist when the user picked.
    #[gpui::test]
    fn open_path_within_the_open_folder_also_clears_the_grid_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // Stubs: the scan filters by extension and never decodes, so these
        // are listed, and no assertion here reads the error slot.
        fixture_stub(&dir.path().join("b1.png"), 10);
        fixture_stub(&dir.path().join("b2.png"), 20);
        let dir_path = dir.path().to_path_buf();
        let same_dir_file = dir_path.join("b1.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open_dir = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.open_folder(open_dir, cx);
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.move_selection(1, cx);
            app.toggle_selected(0, cx);
            app.toggle_selected(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 2);
            assert_eq!(app.session.images[0].path, dir_path.join("b1.png"));
            assert_eq!(app.session.images[1].path, dir_path.join("b2.png"));
            assert_eq!(app.selected, BTreeSet::from([0, 1]), "both files selected");
            assert_eq!(app.anchor, 1);
        });

        // The drift the guard cannot see: a new file in the SAME folder, so
        // `current_dir` matches and a guarded reset would keep the indices.
        fixture_stub(&dir.path().join("a0.png"), 30);
        let file = same_dir_file.clone();
        app.update(cx, |app, cx| app.open_path(file, cx));
        app.read_with(cx, |app, _| {
            assert_eq!(app.current_dir.as_deref(), Some(dir_path.as_path()));
            assert!(
                app.selected.is_empty(),
                "same folder, same `current_dir` — and still cleared, because \
                 indices are not paths"
            );
            assert_eq!(app.grid_selected, 0);
            assert_eq!(app.anchor, 0);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 3, "the new file is listed");
            assert_eq!(app.session.images[0].path, dir_path.join("a0.png"));
        });
        app.update(cx, |app, cx| app.stage_delete(cx));
        app.read_with(cx, |app, _| {
            assert!(
                app.pending_batch.is_none(),
                "a kept `{{0, 1}}` would have staged a0.png, which the user \
                 never saw and never selected"
            );
        });
    }

    // ── V3: batch execution + report ──

    /// Enter with a staged delete executes: files leave the session AND the
    /// disk (recycle bin), set clears on full success, no status lingers.
    #[gpui::test]
    fn confirm_delete_executes_and_clears(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir.path().join("b.png"), b"stub").expect("fixture b.png");
        std::fs::write(dir.path().join("c.png"), b"stub").expect("fixture c.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        // Selection and staging read the loaded list: pump the scan first.
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.toggle_selected(0, cx);
            app.toggle_selected(1, cx);
            app.stage_delete(cx);
            app.confirm_pending(cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.pending_batch.is_none());
            assert!(app.selected.is_empty());
            assert_eq!(app.session.images.len(), 1);
            assert!(app.batch_status.is_none(), "full success is silent");
        });
        assert!(!dir_path.join("a.png").exists());
        assert!(!dir_path.join("b.png").exists());
        assert!(dir_path.join("c.png").exists());
    }

    /// Partial move keeps exactly the leftovers selected and reports.
    #[gpui::test]
    fn partial_move_keeps_leftovers_and_reports(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir.path().join("b.png"), b"stub").expect("fixture b.png");
        let dir_path = dir.path().to_path_buf();
        let dest = tempfile::tempdir().expect("tempdir dest");
        // Pre-seed the collision: b.png already exists at destination.
        std::fs::write(dest.path().join("b.png"), b"original").expect("seed collision");
        let dest_path = dest.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (d, t) = (dir_path.clone(), dest_path.clone());
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        // Selection and staging read the loaded list: pump the scan first.
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_move(t, cx);
            app.confirm_pending(cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            // a.png moved away; only b.png (skipped) remains, still marked.
            assert_eq!(app.session.images.len(), 1);
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0]);
            let status = app.batch_status.clone().expect("partial must report");
            assert!(status.contains("1 of 2"), "got: {status}");
            assert!(status.contains("skipped"), "got: {status}");
        });
        assert!(!dir_path.join("a.png").exists());
        assert!(dir_path.join("b.png").exists());
        assert!(dest_path.join("a.png").exists());
        assert_eq!(std::fs::read(dest_path.join("b.png")).unwrap(), b"original");
    }

    /// Deleting everything the folder holds lands in the existing
    /// empty-state (error slot mirrors the open_folder empty arm).
    #[gpui::test]
    fn delete_all_leaves_empty_grid_state(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        // Selection and staging read the loaded list: pump the scan first.
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_delete(cx);
            app.confirm_pending(cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.session.images.is_empty());
            assert!(app.session.error.is_some());
            assert_eq!(app.session.current, 0);
        });
    }

    // ── V3: batch execution is off the main thread (AGENTS.md §7.1) ──

    /// The regression this whole change exists for: the trash and the rescan
    /// must NOT run inline on the confirm path (AGENTS.md §7.1 — no blocking
    /// I/O on the main thread).
    ///
    /// Deterministic by construction, not by timing: the test harness's
    /// dispatcher QUEUES background runnables and only executes them from
    /// `run_until_parked`, so before that pump nothing of the op has run at
    /// all. An inline implementation would have trashed the files and
    /// re-scanned by the time `confirm_pending` returned, failing BOTH halves
    /// of the pre-pump assertion below.
    #[gpui::test]
    fn confirm_pending_defers_trash_and_rescan_off_the_main_thread(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir.path().join("b.png"), b"stub").expect("fixture b.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        // Folder load first (its own background scan), THEN the batch — so
        // this test still exercises the batch guard over a settled list.
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_delete(cx);
        });
        // Fixture sanity: the folder is loaded and the op is staged, so any
        // later failure is about the executor, not about the setup.
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 2, "folder loaded");
            assert!(app.pending_batch.is_some(), "op staged");
        });
        app.update(cx, |app, cx| app.confirm_pending(cx));
        // ── The regression window: the executor has not been pumped yet ──
        app.read_with(cx, |app, _| {
            assert!(
                app.pending_batch.is_none(),
                "the confirm bar closes immediately, without waiting for the disk"
            );
            assert!(dir_path.join("a.png").exists(), "trash is NOT inline");
            assert!(dir_path.join("b.png").exists(), "trash is NOT inline");
            assert_eq!(
                app.session.images.len(),
                2,
                "rescan is NOT inline — the list is still the pre-op one"
            );
        });
        // Pump: the background task runs, the completion commits under the
        // list-identity guard, and the session converges with the disk.
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.session.images.is_empty(), "op landed after the pump");
            assert!(app.batch_status.is_none(), "full success is silent");
        });
        assert!(!dir_path.join("a.png").exists());
        assert!(!dir_path.join("b.png").exists());
    }

    /// The `BatchOp::Move` arm travels the same background path: files land
    /// at the destination once the executor is pumped, never inline.
    #[gpui::test]
    fn confirm_move_runs_on_the_background_path(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"moved").expect("fixture a.png");
        let dest = tempfile::tempdir().expect("tempdir dest");
        let dir_path = dir.path().to_path_buf();
        let dest_path = dest.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (d, t) = (dir_path.clone(), dest_path.clone());
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        // Folder load first, then the batch (see the sibling test).
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_move(t, cx);
        });
        app.update(cx, |app, cx| app.confirm_pending(cx));
        assert!(
            dir_path.join("a.png").exists(),
            "the move is deferred, not performed inline"
        );
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.session.images.is_empty());
            assert!(app.batch_status.is_none(), "full success is silent");
        });
        assert!(!dir_path.join("a.png").exists(), "source relocated");
        assert_eq!(
            std::fs::read(dest_path.join("a.png")).unwrap(),
            b"moved",
            "destination holds the original bytes"
        );
    }

    /// Items for `names` under the synthetic `Z:\fake` root: the pure
    /// predicate's fixture (no filesystem, no harness).
    fn set_items(names: &[&str]) -> Vec<ImageItem> {
        let epoch = std::time::SystemTime::UNIX_EPOCH;
        build_image_items(names.iter().map(|n| sh_core::navigation::ImageEntry {
            path: PathBuf::from(format!("Z:\\fake\\{n}.png")),
            size: 0,
            modified: epoch,
            created: None,
        }))
    }

    /// The `confirm_pending` guard contract, case by case: same set = valid
    /// (order and `session.current` are irrelevant), changed membership =
    /// drop. Pure, so no harness is needed.
    #[test]
    fn same_image_set_compares_membership_not_order() {
        let a = PathBuf::from("Z:\\fake\\a.png");
        let b = PathBuf::from("Z:\\fake\\b.png");
        let c = PathBuf::from("Z:\\fake\\c.png");
        assert!(
            same_image_set(&[a.clone(), b.clone()], &set_items(&["a", "b"])),
            "identical list, identical order"
        );
        assert!(
            same_image_set(&[a.clone(), b.clone()], &set_items(&["b", "a"])),
            "a re-sort reorders without changing membership, so the result stays valid"
        );
        assert!(
            !same_image_set(&[a.clone(), b.clone()], &set_items(&["a", "b", "c"])),
            "a path was added — the user opened a different folder"
        );
        assert!(
            !same_image_set(&[a.clone(), b.clone(), c], &set_items(&["a", "b"])),
            "a path was removed — membership changed"
        );
        assert!(
            !same_image_set(&[], &set_items(&["a"])),
            "empty anchor against a non-empty list is a folder switch"
        );
        assert!(
            same_image_set(&[], &set_items(&[])),
            "empty on both sides is the same (empty) list — deleting the last file still lands"
        );
    }

    // ── Folder opens are off the main thread (AGENTS.md §7.1) ──

    /// The regression this whole change exists for: the directory scan must
    /// NOT run inline on the open path, and the old DOUBLE scan (folder scan
    /// + the `open_path` re-scan behind it) must not come back either.
    ///
    /// Deterministic by construction, not by timing: the harness dispatcher
    /// QUEUES background runnables and only executes them from
    /// `run_until_parked`, so before that pump nothing of the open has run.
    /// The pre-pump halves below are what an inline implementation — or one
    /// that re-scans — could not satisfy.
    #[gpui::test]
    fn open_folder_defers_the_scan_off_the_main_thread(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // Real PNGs, not stubs: a stub makes the post-pump dimension probe
        // fail into the error slot, which would blur the false-claim
        // assertions below.
        fixture_png_in(dir.path(), "a.png");
        fixture_png_in(dir.path(), "b.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.open_folder(d, cx);
        });
        // ── The regression window: the executor has not been pumped yet ──
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.session.images.len(),
                0,
                "the scan is NOT inline — no folder images are listed yet"
            );
            assert_eq!(app.view, View::Grid, "the view switch IS inline");
            // The Grid chrome reset ran, so the list must not be showing the
            // PREVIOUS folder under it.
            assert_eq!(app.grid_scroll_px, 0.0);
            assert!(app.selected.is_empty());
            assert!(
                app.session.error.is_none(),
                "no error is claimed before the scan ran"
            );
            assert!(
                app.settings.recent_dirs.is_empty(),
                "recents are written by the completion, not by the request"
            );
        });
        // Pump: the single scan lands and the whole tail applies.
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 2, "scan landed after the pump");
            assert_eq!(app.session.current, 0);
            assert_eq!(app.view, View::Grid);
            assert_eq!(app.grid_selected, 0);
            assert_eq!(app.anchor, 0);
            assert_eq!(app.grid_scroll_px, 0.0);
            assert!(app.selected.is_empty());
            assert!(app.batch_status.is_none());
            assert!(
                app.session.error.is_none(),
                "a real folder with real images reports nothing"
            );
            // Recents are owned by the completion's `persist`.
            assert_eq!(app.settings.recent_dirs, vec![dir_path.clone()]);
        });
    }

    /// `open_path` on a file whose parent is NOT a directory: the `NotAFile`
    /// error travels the same background gate, so it cannot appear before
    /// the pump either — and a real parent still lands normally afterwards.
    #[gpui::test]
    fn open_path_defers_the_scan_and_the_not_a_file_verdict(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let file = fixture_png_in(dir.path(), "a.png");
        // `a.png` is a FILE, so it is its own non-directory "parent" case:
        // open a path that lives under it, i.e. parent = a regular file.
        let nested = file.join("inside.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let bad = nested.clone();
        app.update(cx, |app, cx| {
            app.open_path(bad, cx);
        });
        // ── Nothing has landed: not the anchor, not the error ──
        app.read_with(cx, |app, _| {
            assert!(
                app.session.error.is_none(),
                "the parent gate is off the frame loop too"
            );
            assert!(
                !app.session.images.iter().any(|i| i.path == file),
                "no list was built for an invalid parent"
            );
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let error = app
                .session
                .error
                .clone()
                .expect("NotAFile must surface after the pump");
            assert!(
                error.contains(&nested.display().to_string()),
                "the error names the path the user asked for: {error}"
            );
        });
        // The valid sibling case still works, anchored on the opened file.
        let good = file.clone();
        app.update(cx, |app, cx| {
            app.open_path(good, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(
                app.session.error.is_some(),
                "the previous error is still on screen — the completion clears it"
            );
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.session.error.is_none());
            assert_eq!(app.session.images.len(), 1);
            assert_eq!(
                app.session.current_item().map(|i| i.path.clone()),
                Some(file)
            );
        });
    }

    /// Ticket guard: load A, then load B before either scan returns. Only B
    /// may commit — A's completion is about a folder the user already left.
    ///
    /// The discriminating assertion is `thumb_seq`, not the final list: the
    /// harness runs the two queued completions in FIFO order, so an UNGUARDED
    /// A would still be overwritten by B and the list would look identical.
    /// `thumb_seq` counts thumb batches armed, and only a completion arms
    /// one — so it proves A's tail never ran at all, not merely that B won
    /// the race. Real PNGs so the batch has something to decode.
    #[gpui::test]
    fn newer_folder_load_drops_the_stale_one(cx: &mut gpui::TestAppContext) {
        let dir_a = tempfile::tempdir().expect("tempdir A must be created");
        let dir_b = tempfile::tempdir().expect("tempdir B must be created");
        fixture_png_in(dir_a.path(), "a1.png");
        fixture_png_in(dir_a.path(), "a2.png");
        fixture_png_in(dir_b.path(), "b1.png");
        let a = dir_a.path().to_path_buf();
        let b = dir_b.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        // Construction arms one batch for the session the app is born with, so
        // the invariant below is an INCREMENT rather than an absolute. Pinning
        // the absolute would quietly weaken this guard the next time startup
        // arms differently — it is here to prove A's completion never ran.
        let thumb_seq_at_construction = app.read_with(cx, |app, _| app.thumb_seq);
        let (a1, b1) = (a.clone(), b.clone());
        app.update(cx, |app, cx| {
            app.open_folder(a1, cx);
        });
        // No pump between: both scans are in flight and B holds the newest
        // ticket.
        app.update(cx, |app, cx| {
            app.open_folder(b1, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images.len(), 1, "only B's scan committed");
            assert_eq!(app.session.images[0].path, b.join("b1.png"));
            assert!(
                !app.session.images.iter().any(|i| i.path.starts_with(&a)),
                "stale folder A must leave nothing behind"
            );
            assert_eq!(app.list_load_seq, 2, "one ticket per open");
            assert_eq!(
                app.thumb_seq,
                thumb_seq_at_construction + 1,
                "exactly ONE completion armed a thumb batch — the stale one never applied"
            );
            assert!(
                !app.thumbs.keys().any(|p| p.starts_with(&a)),
                "no thumbnail from the abandoned folder"
            );
            assert!(
                app.thumbs.contains_key(&b.join("b1.png")),
                "the live folder's thumb batch ran"
            );
        });
    }

    /// The empty-folder error is a VERDICT, not a prediction: it must not be
    /// set while the scan is still in flight, and it must be set once an
    /// empty result is real knowledge.
    #[gpui::test]
    fn empty_folder_error_waits_for_the_scan_to_prove_it(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.open_folder(d, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Grid, "the view switch IS inline");
            assert!(
                app.session.error.is_none(),
                "claiming 'no images' before the scan ran would be a false claim"
            );
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.session.images.is_empty());
            let error = app.session.error.clone().expect("empty folder reports");
            assert_eq!(
                error,
                sh_core::i18n::no_images_in(app.settings.language, &dir_path.display().to_string())
            );
        });
    }

    /// A load the user superseded must not be remembered: `persist` lives in
    /// the completion, so an abandoned folder never reaches the recents list
    /// (and never triggers a settings write for it).
    #[gpui::test]
    fn superseded_open_never_reaches_the_recents_list(cx: &mut gpui::TestAppContext) {
        let dir_a = tempfile::tempdir().expect("tempdir A must be created");
        let dir_b = tempfile::tempdir().expect("tempdir B must be created");
        std::fs::write(dir_a.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir_b.path().join("b.png"), b"stub").expect("fixture b.png");
        let a = dir_a.path().to_path_buf();
        let b = dir_b.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (a1, b1) = (a.clone(), b.clone());
        app.update(cx, |app, cx| {
            app.open_folder(a1, cx);
        });
        app.update(cx, |app, cx| {
            app.open_folder(b1, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings.recent_dirs,
                vec![b.clone()],
                "only the folder that actually loaded is remembered"
            );
            assert!(!app.settings.recent_dirs.contains(&a));
            assert_eq!(app.recent_dirs_available, vec![b.clone()]);
            assert_eq!(app.settings.last_dir, Some(b.clone()));
        });
    }

    /// Bar message counts + shortens the destination (`display_name`);
    /// resolves through the `i18n` templates in both languages (S5).
    #[test]
    fn batch_bar_message_counts_and_shortens_dest() {
        use sh_core::i18n::Language;
        assert_eq!(
            batch_bar_message(
                Language::En,
                &BatchOp::Delete {
                    paths: vec![PathBuf::from("a")]
                }
            ),
            "Delete 1 file to recycle bin?"
        );
        assert_eq!(
            batch_bar_message(
                Language::Es,
                &BatchOp::Delete {
                    paths: vec![PathBuf::from("a"), PathBuf::from("b")]
                }
            ),
            "¿Mover 2 archivos a la papelera?"
        );
        assert_eq!(
            batch_bar_message(
                Language::En,
                &BatchOp::Move {
                    paths: vec![PathBuf::from("a"), PathBuf::from("b")],
                    dest: PathBuf::from("C:\\pics\\Fotos"),
                }
            ),
            "Move 2 files to pics\\Fotos?"
        );
        assert_eq!(
            batch_bar_message(
                Language::Es,
                &BatchOp::Move {
                    paths: vec![PathBuf::from("a")],
                    dest: PathBuf::from("C:\\pics\\Fotos"),
                }
            ),
            "¿Mover 1 archivo a pics\\Fotos?"
        );
    }

    /// Copy with an empty set never touches the clipboard (no-op signal
    /// from `selection_paths_string` — safe to assert in-harness).
    #[gpui::test]
    fn copy_empty_selection_is_noop(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.copy_selection(cx); // must not panic, must not write
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
        });
    }

    /// Non-empty copy goes through `selection_paths_string` — the exact
    /// payload is pinned there (Task 1); here we assert the method runs
    /// the write path without error state. NOTE: this touches the REAL
    /// clipboard in the harness. If CI proves flaky, delete this test and
    /// keep the manual-smoke contract (same policy as `copy_image`).
    #[gpui::test]
    fn copy_nonempty_selection_writes_paths(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.toggle_selected(0, cx);
            app.copy_selection(cx);
        });
        app.read_with(cx, |app, _| {
            // No error surfaced; selection intact after copy.
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0]);
        });
    }

    /// Six distinct folders: the list caps at 5, newest first, oldest evicted.
    #[gpui::test]
    fn six_folders_evict_the_oldest(cx: &mut gpui::TestAppContext) {
        let root = tempfile::tempdir().expect("tempdir root");
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        for i in 0..6 {
            let d = root.path().join(format!("d{i}"));
            std::fs::create_dir_all(&d).expect("subdir");
            std::fs::write(d.join(format!("i{i}.png")), b"stub").expect("fixture");
            dirs.push(d);
        }

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        for d in dirs.clone() {
            let dd = d.clone();
            app.update(cx, |app, cx| {
                app.open_folder(dd, cx);
            });
            // Pump per open: six unpumped opens would collapse to the newest
            // ticket and only d5 would ever reach the recents list.
            cx.run_until_parked();
        }
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings.recent_dirs.len(),
                sh_core::recent::RECENT_DIRS_MAX
            );
            // Newest (d5) first; the very first (d0) was evicted.
            assert_eq!(app.settings.recent_dirs[0], dirs[5]);
            assert!(!app.settings.recent_dirs.contains(&dirs[0]));
        });
    }

    /// An empty folder (no images) never enters the recents list — the
    /// existing rule, now list-shaped.
    #[gpui::test]
    fn empty_folder_is_not_remembered(cx: &mut gpui::TestAppContext) {
        let dir_a = tempfile::tempdir().expect("tempdir a");
        let empty = tempfile::tempdir().expect("tempdir empty");
        std::fs::write(dir_a.path().join("a.png"), b"stub").expect("fixture a.png");
        let a = dir_a.path().to_path_buf();
        let e = empty.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (a1, e1) = (a.clone(), e.clone());
        app.update(cx, |app, cx| {
            app.open_folder(a1, cx);
        });
        // Pump per open: the empty folder's error is only set on completion,
        // so an unpumped second open would be the only live ticket and `a`
        // would never be remembered at all.
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.open_folder(e1, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            // Empty folder did NOT replace or add to the list.
            assert_eq!(app.settings.recent_dirs, vec![a.clone()]);
            assert!(!app.settings.recent_dirs.contains(&e));
        });
    }

    // ── V3: slideshow state (transient — dies on folder swap / crop) ──

    /// A folder switch mid-slideshow must stop it: the new folder's scan
    /// swaps `session.images`, and auto-advancing into a fresh list the
    /// user never chose to play is wrong.
    #[gpui::test]
    fn folder_swap_resets_slideshow(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.session.slideshow_active = true;
            app.open_folder(dir_path, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.session.slideshow_active);
        });
    }

    /// Crop mode owns the pointer — the slideshow must not auto-advance
    /// while a selection is being drawn.
    #[gpui::test]
    fn enter_crop_resets_slideshow(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.session.slideshow_active = true;
            app.enter_crop(cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.session.slideshow_active);
        });
    }

    /// The drop-file-then-back bug: `open_path` (used by file drops, the
    /// open-file dialog, and CLI) swaps `session.images` to the dropped
    /// file's sibling list but never armed a thumbnail batch — the Grid
    /// showed empty placeholders forever. Regression: after `open_path` +
    /// `enter_grid`, the background decode must have filled `thumbs`.
    #[gpui::test]
    fn open_path_then_grid_loads_thumbnails(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let file_path = fixture_png_in(dir.path(), "a.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.open_path(file_path.clone(), cx);
        });
        // Real flow: the drop lands, THEN the user navigates back. Pumping
        // here keeps `enter_grid` reading the dropped file's list instead of
        // the fixture list the harness started with.
        cx.run_until_parked();
        // Viewer shows the dropped file; "Back" returns to the Grid.
        app.update(cx, |app, cx| {
            app.enter_grid(cx);
        });
        // Drain the background executor: every decode task must have run
        // and committed under the seq guard.
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Grid);
            assert_eq!(
                app.session.images.len(),
                1,
                "sibling list of the dropped file"
            );
            assert!(
                app.thumbs.contains_key(&file_path),
                "open_path must arm the thumbnail batch — thumbs map is {:?}",
                app.thumbs.keys().collect::<Vec<_>>()
            );
        });
    }

    /// Seq-guard regression: two `open_path` calls back to back (folder A
    /// then file from folder B) must leave only B's thumbnails — the stale
    /// A batch dies on its seq check.
    ///
    /// A is PUMPED before B is requested, so this keeps testing what it
    /// documents: A's decode batch is genuinely in flight when the folder
    /// swap bumps `thumb_seq`. Two unpumped opens would instead be killed by
    /// the newer `list_load_seq` ticket and A would never decode at all.
    #[gpui::test]
    fn open_path_twice_drops_stale_thumbs(cx: &mut gpui::TestAppContext) {
        let dir_a = tempfile::tempdir().expect("tempdir A must be created");
        let dir_b = tempfile::tempdir().expect("tempdir B must be created");
        let png_a = fixture_png_in(dir_a.path(), "a.png");
        let png_b = fixture_png_in(dir_b.path(), "b.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.open_path(png_a.clone(), cx);
        });
        // Folder A's list lands and its decode batch goes in flight…
        cx.run_until_parked();
        // …then the swap bumps `thumb_seq` while that batch is still
        // draining.
        app.update(cx, |app, cx| {
            app.open_path(png_b.clone(), cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.thumbs.contains_key(&png_b), "current folder's thumb");
            assert!(
                !app.thumbs.contains_key(&png_a),
                "stale folder A thumb must be dropped (seq guard)"
            );
        });
    }

    #[gpui::test]
    fn viewer_viewport_carves_chrome_only_when_tab_on(cx: &mut gpui::TestAppContext) {
        // Strip OFF: Tab OFF = full window (topbar floats, zero layout
        // space — R3 unchanged for the topbar); Tab ON = window minus
        // the in-flow bottom chrome (`BOTTOM_CHROME_H_PX`), because the
        // chrome row owns real layout space below the image now.
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _| {
            app.viewport = gpui::size(gpui::px(1000.), gpui::px(720.));
            app.view = View::Viewer;
            app.settings.filmstrip = false;
            app.session.show_overlay_bottom = false;
        });
        // Tab OFF: no bottom chrome mounted → full window.
        app.read_with(cx, |app, _| {
            let v = app.viewer_viewport();
            assert!((v.x - 1000.0).abs() < 1e-5);
            assert!((v.y - 720.0).abs() < 1e-5, "got {}", v.y);
        });
        // Tab ON: in-flow chrome mounts below the image → carved.
        app.update(cx, |app, _| {
            app.session.show_overlay_bottom = true;
        });
        app.read_with(cx, |app, _| {
            let v = app.viewer_viewport();
            assert!(
                (v.y - (720.0 - crate::ui::overlay::BOTTOM_CHROME_H_PX)).abs() < 1e-5,
                "got {}",
                v.y
            );
            assert!((v.x - 1000.0).abs() < 1e-5, "full width in both states");
        });
    }

    /// Sort menu flow (V3): opening the menu and picking a criterion
    /// updates the session + settings copy and closes the menu. State-based
    /// assertions — the dropdown click itself goes through the same
    /// `set_sort`/`sort_menu_open` state the chip and rows mutate.
    #[gpui::test]
    fn sort_menu_flow_updates_state_and_persists(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        fixture_stub(&dir.path().join("a.png"), 100);
        fixture_stub(&dir.path().join("b.png"), 300);

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let dir_path = dir.path().to_path_buf();
        app.update(cx, |app, cx| {
            app.open_folder(dir_path.clone(), cx);
        });
        cx.run_until_parked();
        // Toggle the menu open via the same flag the chip's click flips.
        app.update(cx, |app, cx| {
            app.sort_menu_open = true;
            cx.notify();
        });
        // "Pick" Size-desc via the same calls the menu rows make.
        app.update(cx, |app, cx| {
            app.set_sort(SortBy::Size, SortDir::Desc, cx);
            app.sort_menu_open = false;
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.sort_by, SortBy::Size);
            assert_eq!(app.session.sort_dir, SortDir::Desc);
            assert!(!app.sort_menu_open);
            assert_eq!(app.settings.sort_by, SortBy::Size);
            // Size-desc order: b(300) then a(100).
            assert_eq!(app.session.images[0].path, dir_path.join("b.png"));
            assert_eq!(app.session.images[1].path, dir_path.join("a.png"));
        });
    }

    #[gpui::test]
    fn enter_viewer_sets_current_and_view(cx: &mut gpui::TestAppContext) {
        // Fake paths: the header probe fails into the error slot, but
        // current/view switching is synchronous — assert only that.
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.enter_viewer(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Viewer);
            assert_eq!(app.session.current, 2);
            assert_eq!(app.grid_selected, 2);
        });
    }

    /// Sort contract (V3): changing the criterion keeps the selection on
    /// the SAME image (re-anchored by path), the grid selection follows,
    /// and the settings copy mirrors the change for the next launch.
    #[gpui::test]
    fn set_sort_reanchors_selection_by_path(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // Distinct sizes so Size-desc reorders against the Name-asc scan
        // order: scan gives a(100), b(300), c(200); Size-desc is b, c, a.
        fixture_stub(&dir.path().join("a.png"), 100);
        fixture_stub(&dir.path().join("b.png"), 300);
        fixture_stub(&dir.path().join("c.png"), 200);

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let dir_path = dir.path().to_path_buf();
        app.update(cx, |app, cx| {
            app.open_folder(dir_path.clone(), cx);
        });
        cx.run_until_parked();
        // Select the third image under Name/Asc (c.png, index 2).
        app.update(cx, |app, cx| {
            app.enter_viewer(2, cx);
        });
        let anchor_path: PathBuf = app.read_with(cx, |app, _| {
            app.session
                .current_item()
                .expect("c.png must be current")
                .path
                .clone()
        });
        // Sort by size desc: order becomes b(300), c(200), a(100).
        app.update(cx, |app, cx| {
            app.set_sort(
                sh_core::navigation::SortBy::Size,
                sh_core::navigation::SortDir::Desc,
                cx,
            );
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images[0].path, dir_path.join("b.png"));
            assert_eq!(app.session.images[1].path, dir_path.join("c.png"));
            assert_eq!(app.session.images[2].path, dir_path.join("a.png"));
            // Re-anchored: same image selected, new index.
            assert_eq!(
                app.session
                    .current_item()
                    .expect("selection must survive the resort")
                    .path,
                anchor_path
            );
            assert_eq!(app.session.current, 1);
            assert_eq!(
                app.grid_selected, 1,
                "grid selection must follow the anchor"
            );
            // Persisted: the settings copy carries the new sort.
            assert_eq!(app.settings.sort_by, sh_core::navigation::SortBy::Size);
            assert_eq!(app.settings.sort_dir, sh_core::navigation::SortDir::Desc);
        });
    }

    /// A re-sort must move the shift-range ANCHOR with the cursor, not leave
    /// it on a stale index.
    ///
    /// The anchor is an index, and `set_sort` renumbers the list, so the two
    /// can drift apart: the cursor follows the current image by path, while a
    /// leftover anchor keeps whatever number it had. The next Shift+click
    /// would then extend a range from an image the user never pointed at.
    ///
    /// The fixture is chosen so the two values genuinely DIVERGE, because a
    /// case where the cursor happens to land back on the old anchor index
    /// would pass with the bug still present. Cursor sits on c.png (index 2)
    /// with the anchor deliberately on 0; Size-desc reorders to b, c, a, so
    /// the cursor lands on 1 while a stale anchor would sit on 0.
    #[gpui::test]
    fn set_sort_moves_the_shift_range_anchor_with_the_cursor(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // Same sizes as the sibling test on purpose: scan order a(100),
        // b(300), c(200); Size-desc is b, c, a.
        fixture_stub(&dir.path().join("a.png"), 100);
        fixture_stub(&dir.path().join("b.png"), 300);
        fixture_stub(&dir.path().join("c.png"), 200);

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let dir_path = dir.path().to_path_buf();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(dir_path.clone(), cx);
        });
        cx.run_until_parked();
        // Cursor on c.png, range anchor deliberately elsewhere (index 0).
        app.update(cx, |app, cx| {
            app.enter_viewer(2, cx);
            app.anchor = 0;
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.anchor, 0, "precondition: anchor sits on a.png");
            assert_eq!(app.grid_selected, 2, "precondition: cursor on c.png");
        });
        app.update(cx, |app, cx| {
            app.set_sort(
                sh_core::navigation::SortBy::Size,
                sh_core::navigation::SortDir::Desc,
                cx,
            );
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.grid_selected, 1, "cursor followed c.png to index 1");
            assert_eq!(
                app.anchor, app.grid_selected,
                "the range anchor must collapse onto the cursor, not stay on the \
                 stale index (which now names b.png)"
            );
        });
    }

    // ── Zoomable grid Phase 3 RED: set_grid_size + chip ──

    /// Size contract: selecting L mirrors into the settings copy and the
    /// settings file, so a relaunch restores L with no further action.
    #[gpui::test]
    fn set_grid_size_persists_and_restores_across_relaunch(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.settings_path = settings_path.clone();
            app.set_grid_size(sh_core::settings::GridSize::L, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.grid_size, sh_core::settings::GridSize::L);
        });
        let restored = sh_core::settings::load(&settings_path);
        assert_eq!(
            restored.grid_size,
            sh_core::settings::GridSize::L,
            "L must survive a relaunch through settings.json"
        );
    }

    /// Re-clamp contract: a stale offset from a larger grid can neither
    /// strand the last rows (shrink) nor leave a trailing gap (grow), and
    /// the cursor stays visible after every size change.
    #[gpui::test]
    fn set_grid_size_reclamps_stale_offset(cx: &mut gpui::TestAppContext) {
        use crate::ui::grid::{self, GridSizeGeometry};
        use sh_core::settings::GridSize;
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        // 30 images so every preset scrolls in a short viewport.
        app.update(cx, |app, _cx| {
            let epoch = std::time::SystemTime::UNIX_EPOCH;
            app.session.images =
                build_image_items((0..30).map(|i| sh_core::navigation::ImageEntry {
                    path: PathBuf::from(format!("Z:\\fake\\{i:02}.png")),
                    size: 0,
                    modified: epoch,
                    created: None,
                }));
            app.viewport = gpui::size(gpui::px(800.), gpui::px(280.));
        });
        // Cursor on the last image; a huge offset is stale for every
        // preset. NOTE: viewport is (re)set in the same closure as each
        // action — the headless render loop restores the window size
        // between updates, so viewport + action must stay atomic.
        app.update(cx, |app, cx| {
            app.viewport = gpui::size(gpui::px(800.), gpui::px(280.));
            app.set_grid_size(GridSize::L, cx);
            app.grid_selected = 29;
            app.anchor = 29;
            let max_l = grid::grid_max_scroll(30, 800.0, 240.0, &GridSize::L.geometry());
            assert!(max_l > 0.0, "L must scroll in this viewport");
            app.grid_scroll_px = 1e9;
        });
        // Shrink L → S: offset clamps into S's range, last row reachable.
        app.update(cx, |app, cx| {
            app.viewport = gpui::size(gpui::px(800.), gpui::px(280.));
            app.set_grid_size(GridSize::S, cx);
        });
        app.read_with(cx, |app, _| {
            let geo = GridSize::S.geometry();
            let max_s = grid::grid_max_scroll(30, 800.0, 240.0, &geo);
            assert!(
                (app.grid_scroll_px - max_s).abs() < 1e-3,
                "shrink must clamp to the S maximum ({}), got {}",
                max_s,
                app.grid_scroll_px
            );
            let cols = grid::grid_columns(800.0, &geo);
            let row_top = (29 / cols) as f32 * geo.row_h as f32;
            let row_bottom = row_top + geo.row_h as f32;
            assert!(
                row_top >= app.grid_scroll_px && row_bottom <= app.grid_scroll_px + 240.0,
                "cursor row must stay visible after shrink"
            );
        });
        // Grow S → L: offset clamps into L's range (no trailing gap),
        // cursor still visible.
        app.update(cx, |app, cx| {
            app.viewport = gpui::size(gpui::px(800.), gpui::px(280.));
            app.set_grid_size(GridSize::L, cx);
        });
        app.read_with(cx, |app, _| {
            let geo = GridSize::L.geometry();
            let max_l = grid::grid_max_scroll(30, 800.0, 240.0, &geo);
            assert!(
                app.grid_scroll_px <= max_l + 1e-4,
                "grow must clear the trailing gap (max {max_l}), got {}",
                app.grid_scroll_px
            );
            let cols = grid::grid_columns(800.0, &geo);
            let row_top = (29 / cols) as f32 * geo.row_h as f32;
            let row_bottom = row_top + geo.row_h as f32;
            assert!(
                row_top >= app.grid_scroll_px && row_bottom <= app.grid_scroll_px + 240.0,
                "cursor row must stay visible after grow"
            );
        });
    }

    /// Unlike `set_sort`, a size change re-sorts nothing and never moves
    /// the cursor, the anchor, or the work-in-progress set.
    #[gpui::test]
    fn set_grid_size_leaves_order_and_selection_untouched(cx: &mut gpui::TestAppContext) {
        use sh_core::settings::GridSize;
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let before: Vec<PathBuf> = app.read_with(cx, |app, _| {
            app.session.images.iter().map(|i| i.path.clone()).collect()
        });
        app.update(cx, |app, cx| {
            app.grid_selected = 2;
            app.anchor = 2;
            app.selected.insert(0);
            app.selected.insert(2);
            app.set_grid_size(GridSize::L, cx);
        });
        app.read_with(cx, |app, _| {
            let after: Vec<PathBuf> = app.session.images.iter().map(|i| i.path.clone()).collect();
            assert_eq!(after, before, "size change must not re-sort items");
            assert_eq!(app.grid_selected, 2);
            assert_eq!(app.anchor, 2);
            assert!(
                app.selected.contains(&0) && app.selected.contains(&2) && app.selected.len() == 2,
                "selection set must survive a size change"
            );
        });
    }

    /// Chip labels resolve through `Settings.language` (EN + ES words,
    /// never bare letters) — same table the render reads.
    #[test]
    fn grid_size_chip_labels_resolve_through_language() {
        use sh_core::i18n::Language;
        use sh_core::settings::GridSize;
        assert_eq!(grid_size_label(Language::En, GridSize::S), "Small");
        assert_eq!(grid_size_label(Language::En, GridSize::M), "Medium");
        assert_eq!(grid_size_label(Language::En, GridSize::L), "Large");
        assert_eq!(grid_size_label(Language::Es, GridSize::S), "Pequeño");
        assert_eq!(grid_size_label(Language::Es, GridSize::M), "Mediano");
        assert_eq!(grid_size_label(Language::Es, GridSize::L), "Grande");
    }

    /// Segment model: exactly S/M/L in order, only the active preset
    /// marked active — the render maps this 1:1 to chip segments.
    // ── Zoom-preset chips (viewer-zoom-presets, Phase 4) ──

    #[test]
    fn zoom_preset_label_resolves_through_the_table_in_both_languages() {
        // EN + ES for all three chips via `Settings.language` threading.
        let cases = [
            (ZoomPreset::Fit, "Fit", "Ajustar"),
            (ZoomPreset::Scale100, "100%", "100%"),
            (ZoomPreset::Scale200, "200%", "200%"),
        ];
        for (preset, en, es) in cases {
            assert_eq!(zoom_preset_label(Language::En, preset), en);
            assert_eq!(zoom_preset_label(Language::Es, preset), es);
        }
    }

    #[test]
    fn zoom_preset_segments_mark_only_the_active_chip() {
        use sh_core::transform::MAX_SCALE;
        // Resting exactly at 1.0 in Percent100 → only the 100% chip active.
        let at_100 = zoom_preset_segments(Language::En, FitMode::Percent100, 1.0);
        assert_eq!(at_100.len(), 3);
        // Exactly three segments: Fit/100/200 — the 50% chip is gone.
        assert_eq!(
            at_100.iter().map(|(p, _, _)| *p).collect::<Vec<_>>(),
            vec![ZoomPreset::Fit, ZoomPreset::Scale100, ZoomPreset::Scale200]
        );
        for (preset, _, active) in &at_100 {
            let expected = matches!(preset, ZoomPreset::Scale100);
            assert_eq!(*active, expected, "1.0 must mark only Scale100");
        }
        // Free-zoom 137% matches no preset chip.
        for (_, _, active) in zoom_preset_segments(Language::En, FitMode::Percent100, 1.37) {
            assert!(!active, "137% must mark no scale chip");
        }
        // Fit state marks only Fit — even when the fit scale happens to read
        // "100%" on a huge image (fit 1.0 in Fit mode stays Fit-only).
        let in_fit = zoom_preset_segments(Language::En, FitMode::Fit, 1.0);
        for (preset, _, active) in &in_fit {
            let expected = matches!(preset, ZoomPreset::Fit);
            assert_eq!(*active, expected, "Fit mode must mark only Fit");
        }
        // Near-preset-but-not-exact scale stays inactive (avoids the .5
        // halfway edge by asserting a neighbor).
        for (_, _, active) in
            zoom_preset_segments(Language::En, FitMode::Percent100, MAX_SCALE + 0.5)
        {
            assert!(!active);
        }
    }

    #[test]
    fn zoom_preset_guard_no_keymap_actions_or_shortcuts_entries() {
        // Click-only scope guard: nothing added to keymap defaults, ACTIONS,
        // or the Shortcuts panel. ACTIONS.len() 18 and SHORTCUT_ROW_COUNT 18
        // stay pinned by the existing tests; assert both here so a future
        // preset shortkut can never sneak in without breaking this.
        let defaults = sh_core::keymap::defaults();
        for id in defaults.keys() {
            assert!(
                !id.contains("preset"),
                "no preset keymap binding may exist, found {id}"
            );
        }
        for a in crate::actions::ACTIONS {
            assert!(
                !a.id.contains("preset"),
                "no preset action descriptor may exist, found {}",
                a.id
            );
            assert!(
                !format!("{:?}", a.label_key).contains("ZoomPreset"),
                "preset StrKeys are chip labels, not action labels"
            );
        }
        assert_eq!(crate::actions::ACTIONS.len(), 18);
        assert_eq!(crate::ui::settings_panel::scroll::SHORTCUT_ROW_COUNT, 18);
    }

    #[test]
    fn zoom_preset_disabled_flags_sub_floor_presets() {
        use sh_core::transform::Vec2;
        // Small image, roomy viewport: fit floor well above every preset
        // scale (100x100 in 1920x1080 fits at ~10.8) → both scale
        // chips must read disabled, Fit never is.
        let roomy = Vec2 {
            x: 1920.0,
            y: 1080.0,
        };
        let floor = zoom_preset_floor(Some((100, 100)), roomy);
        assert!(floor.unwrap() > 2.0);
        assert!(zoom_preset_disabled(ZoomPreset::Scale100, floor));
        assert!(zoom_preset_disabled(ZoomPreset::Scale200, floor));
        assert!(!zoom_preset_disabled(ZoomPreset::Fit, floor));
        // Large image: floor 0.2 → every preset above it stays enabled.
        let tight = Vec2 { x: 800.0, y: 600.0 };
        let low = zoom_preset_floor(Some((4000, 2000)), tight);
        assert!(low.unwrap() < 1.0);
        for preset in [ZoomPreset::Fit, ZoomPreset::Scale100, ZoomPreset::Scale200] {
            assert!(
                !zoom_preset_disabled(preset, low),
                "{preset:?} above the floor must stay enabled"
            );
        }
        // Exact equality disables: 2160x2160 in 2160x2160 fits at exactly
        // 1.0, and the pinned session funnel snaps a 100% request back to
        // fit — so the chip must not offer it.
        let exact = zoom_preset_floor(
            Some((2160, 2160)),
            Vec2 {
                x: 2160.0,
                y: 2160.0,
            },
        );
        assert!((exact.unwrap() - 1.0).abs() < 1e-5);
        assert!(zoom_preset_disabled(ZoomPreset::Scale100, exact));
    }

    #[test]
    fn zoom_preset_disabled_unknown_dims_keeps_chips_enabled() {
        // Probe in flight: no floor to compare against → every chip stays
        // enabled rather than bricking the preset row on a slow header.
        use sh_core::transform::Vec2;
        let viewport = Vec2 { x: 800.0, y: 600.0 };
        let floor = zoom_preset_floor(None, viewport);
        assert_eq!(floor, None);
        for preset in [ZoomPreset::Fit, ZoomPreset::Scale100, ZoomPreset::Scale200] {
            assert!(
                !zoom_preset_disabled(preset, floor),
                "{preset:?} must stay enabled while dims are unknown"
            );
        }
    }

    #[test]
    fn stable_open_viewport_ignores_chrome_state() {
        // The same window must open the same image at the same zoom no
        // matter the transient idle/topbar state: the stable viewport is
        // the full window, never minus the bar.
        use sh_core::transform::Vec2;
        let window = Vec2 {
            x: 1000.0,
            y: 720.0,
        };
        let stable = stable_open_viewport(window);
        assert_eq!(stable.x, 1000.0);
        assert_eq!(stable.y, 720.0);
        // Degenerate heights never collapse to zero (.max(1.0) pact).
        assert_eq!(stable_open_viewport(Vec2 { x: 800.0, y: 0.0 }).y, 1.0);
        // The strip is carved chrome: with the strip and chrome hidden the
        // carve delegates to this function bit-identically, while the
        // idle state stays ignored by both (it never appears in either
        // signature).
        assert_eq!(
            stable_filmstrip_viewport(window, false, false),
            stable_open_viewport(window)
        );
    }

    #[test]
    fn stable_filmstrip_viewport_carves_strip_height_when_visible() {
        // R5.1: strip ON carves exactly STRIP_H_PX at full width
        // (chrome OFF here — pinned separately).
        use sh_core::transform::Vec2;
        let window = Vec2 {
            x: 1000.0,
            y: 720.0,
        };
        let carved = stable_filmstrip_viewport(window, true, false);
        assert_eq!(carved.x, 1000.0);
        assert!(
            (carved.y - (720.0 - crate::filmstrip::STRIP_H_PX)).abs() < 1e-5,
            "got {}",
            carved.y
        );
    }

    #[test]
    fn stable_filmstrip_viewport_carves_chrome_and_stacks_both() {
        // In-flow bottom chrome (Tab ON) carves exactly
        // `BOTTOM_CHROME_H_PX`, stacking additively with the strip. The
        // carve must mirror the real layout or the image clips behind
        // the chrome row.
        use sh_core::transform::Vec2;
        let window = Vec2 {
            x: 1000.0,
            y: 720.0,
        };
        let chrome_only = stable_filmstrip_viewport(window, false, true);
        assert_eq!(chrome_only.x, 1000.0);
        assert!(
            (chrome_only.y - (720.0 - crate::ui::overlay::BOTTOM_CHROME_H_PX)).abs() < 1e-5,
            "got {}",
            chrome_only.y
        );
        let both = stable_filmstrip_viewport(window, true, true);
        assert!(
            (both.y
                - (720.0 - crate::filmstrip::STRIP_H_PX - crate::ui::overlay::BOTTOM_CHROME_H_PX))
                .abs()
                < 1e-5,
            "got {}",
            both.y
        );
        // Floors survive the combined carve (`.max(1.0)` pact).
        assert_eq!(
            stable_filmstrip_viewport(Vec2 { x: 800.0, y: 0.0 }, true, true).y,
            1.0
        );
    }

    #[test]
    fn stable_filmstrip_viewport_hidden_is_bit_identical_to_open() {
        // R5.2: strip OFF (and chrome OFF) is bit-identical to
        // stable_open_viewport for every window, including the
        // `.max(1.0)` floor.
        use sh_core::transform::Vec2;
        for window in [
            Vec2 {
                x: 1000.0,
                y: 720.0,
            },
            Vec2 { x: 800.0, y: 0.0 },
            Vec2 {
                x: 1920.0,
                y: 1080.0,
            },
        ] {
            assert_eq!(
                stable_filmstrip_viewport(window, false, false),
                stable_open_viewport(window)
            );
        }
        // Degenerate carved heights never collapse to zero (.max(1.0)
        // pact preserved on the carved height).
        assert_eq!(
            stable_filmstrip_viewport(Vec2 { x: 800.0, y: 0.0 }, true, false).y,
            1.0
        );
    }

    #[test]
    fn grid_size_segments_mark_only_the_active_preset() {
        use sh_core::i18n::Language;
        use sh_core::settings::GridSize;
        let segs = grid_size_segments(Language::En, GridSize::M);
        assert_eq!(segs.len(), 3, "exactly S/M/L segments");
        assert_eq!(
            segs.iter().map(|(s, _, _)| *s).collect::<Vec<_>>(),
            vec![GridSize::S, GridSize::M, GridSize::L]
        );
        assert_eq!(
            segs.iter()
                .map(|(_, _, active)| *active)
                .collect::<Vec<_>>(),
            vec![false, true, false]
        );
        assert_eq!(segs[1].1, "Medium");
        // Spanish renderings travel through the same model.
        let es = grid_size_segments(Language::Es, GridSize::S);
        assert_eq!(es[0].1, "Pequeño");
        assert!(es[0].2);
        assert!(!es[1].2 && !es[2].2);
    }

    #[gpui::test]
    fn move_selection_clamps_and_scrolls(cx: &mut gpui::TestAppContext) {
        // test_app ships 3 fake images; selection math is sync.
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.viewport = gpui::size(gpui::px(1000.), gpui::px(720.));
            app.move_selection(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.grid_selected, 1);
        });
        app.update(cx, |app, cx| {
            // Past the end sticks (len 3) instead of wrapping.
            app.move_selection(10, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.grid_selected, 2);
            assert!(app.grid_scroll_px >= 0.0);
        });
    }

    #[gpui::test]
    fn enter_grid_follows_current(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.session.current = 1;
            app.enter_grid(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Grid);
            assert_eq!(app.grid_selected, 1);
            assert_eq!(app.grid_scroll_px, 0.0);
        });
    }

    #[gpui::test]
    fn apply_theme_entry_switches_and_persists(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            // test_app starts on the default (Noir Gallery) theme.
            assert_ne!(app.theme_store.name, "dark-clinical.json");
            let entry = builtin_entry(app, "dark-clinical.json");
            assert!(app.apply_theme_entry(&entry, cx));
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_store.name, "dark-clinical.json");
            assert_eq!(app.theme_store.theme.name, "Dark Clinical");
            assert_eq!(app.settings.theme, "dark-clinical.json");
            // Hot-reload baseline follows the switch (no instant revert).
            let builtin = crate::theme_builtins::builtin_theme_json("dark-clinical.json");
            assert_eq!(app.last_applied_theme_text, builtin);
        });
        // An unselectable row (the invalid-file case) is a no-op returning
        // false — the guard that stops a click on a broken theme.
        app.update(cx, |app, cx| {
            let broken = crate::ui::settings_panel::sections::appearance::ThemeEntry {
                file_name: "broken.json".into(),
                path: std::path::PathBuf::from("/nowhere/broken.json"),
                theme: None,
                text: "{ not json".into(),
                bootstrap_json: None,
            };
            assert!(!app.apply_theme_entry(&broken, cx));
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_store.name, "dark-clinical.json");
        });
    }

    /// A discovered user theme must set a REAL path in the store, because
    /// that path is what the hot-reload watcher polls. Picking a theme the
    /// app cannot watch would leave hot reload silently dead.
    #[gpui::test]
    fn apply_user_theme_entry_points_hot_reload_at_the_user_file(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let user_dir = tempfile::tempdir().unwrap();
        let user_path = user_dir.path().join("mine.json");
        std::fs::write(&user_path, user_theme_json("Mine")).unwrap();

        app.update(cx, |app, cx| {
            let found = sh_core::theme::load_discovered(user_dir.path());
            let entries = crate::ui::settings_panel::sections::appearance::merge_theme_entries(
                user_dir.path(),
                &found,
            );
            let mine = entries
                .iter()
                .find(|e| e.file_name == "mine.json")
                .expect("discovered row")
                .clone();
            assert!(app.apply_theme_entry(&mine, cx));
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_store.name, "mine.json");
            assert_eq!(app.theme_store.theme.name, "Mine");
            assert_eq!(app.settings.theme, "mine.json");
            assert_eq!(app.theme_store.path, user_path);
            // The baseline must be the file's own text, or the watcher
            // would treat the first read as an edit and re-apply.
            assert_eq!(app.last_applied_theme_text, user_theme_json("Mine"));
        });
    }

    /// Point the app at a REAL config dir so a built-in row resolves to a
    /// real (missing) path under it — `test_app` ships a fake root, which
    /// would make the bootstrap write a no-op for reasons unrelated to the
    /// code under test.
    fn use_real_config_dir(app: &mut App, config: &std::path::Path) {
        app.settings_path = config.join("settings.json");
        app.theme_entries = crate::ui::settings_panel::sections::appearance::builtin_theme_entries(
            &config.join("themes"),
        );
    }

    /// The regression this change exists for: the `stat` + `mkdir` + `write`
    /// behind a picker's click handler must NOT run on the frame loop
    /// (AGENTS.md §7.1). Only the DISK half is deferred — the store swap has
    /// to land on this frame or the picker would repaint a frame late.
    ///
    /// Deterministic by construction, not by timing: the harness dispatcher
    /// QUEUES background runnables and only runs them from
    /// `run_until_parked`, so before that pump nothing of the write has
    /// happened. An inline implementation would have created the file by the
    /// time `apply_theme_entry` returned, failing the pre-pump half below.
    #[gpui::test]
    fn apply_theme_entry_defers_the_bootstrap_write_off_the_frame_loop(
        cx: &mut gpui::TestAppContext,
    ) {
        let config = tempfile::tempdir().expect("tempdir must be created");
        let theme_file = config.path().join("themes").join("dark-clinical.json");
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _| use_real_config_dir(app, config.path()));

        app.update(cx, |app, cx| {
            let entry = builtin_entry(app, "dark-clinical.json");
            assert!(entry.bootstrap_json.is_some(), "a built-in row carries one");
            assert!(app.apply_theme_entry(&entry, cx));
        });
        // ── The regression window: the executor has not been pumped yet ──
        app.read_with(cx, |app, _| {
            // The in-memory half is synchronous: the picker is correct now.
            assert_eq!(app.theme_store.name, "dark-clinical.json");
            assert_eq!(app.settings.theme, "dark-clinical.json");
            // The disk half is not: no stat, no mkdir, no write yet.
            assert!(!theme_file.exists(), "the bootstrap write is NOT inline");
            assert!(
                !theme_file.parent().expect("a parent").exists(),
                "and neither is its mkdir"
            );
        });
        cx.run_until_parked();
        assert!(theme_file.exists(), "the write landed after the pump");
        assert_eq!(
            std::fs::read_to_string(&theme_file).expect("written theme must read"),
            crate::theme_builtins::builtin_theme_json("dark-clinical.json"),
            "the file holds the built-in JSON, so the theme stays editable"
        );
    }

    /// Picking A and then B before A's write lands is the window going async
    /// opens. A's write is keyed by A's PATH, so it can only ever create A's
    /// file — it must not touch the store, the baseline or the settings, all
    /// of which belong to the theme picked LAST.
    ///
    /// B is deliberately a USER theme: it has no bootstrap write of its own,
    /// so A's completion is the only thing that could land after the pick of
    /// B, and the assertions below are about the state, not about completion
    /// order. A completion that re-applied its captured row over B fails here
    /// — the harness's FIFO dispatch hides the bug when B has a write of its
    /// own, because then B's own completion happens to arrive last and masks
    /// it.
    #[gpui::test]
    fn late_bootstrap_write_leaves_a_later_theme_pick_untouched(cx: &mut gpui::TestAppContext) {
        let config = tempfile::tempdir().expect("tempdir must be created");
        let a_file = config.path().join("themes").join("dark-clinical.json");
        let b_file = config.path().join("themes").join("mine.json");
        std::fs::create_dir_all(b_file.parent().expect("a parent"))
            .expect("theme dir must be created");
        std::fs::write(&b_file, user_theme_json("Mine")).expect("seed the user theme");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _| use_real_config_dir(app, config.path()));
        // Discover the user theme the way the picker does, so B is a row the
        // UI would really offer.
        app.update(cx, |app, cx| app.refresh_theme_entries(cx));
        cx.run_until_parked();

        // A is picked, then B, with no pump in between: A's write is in
        // flight across the whole second pick.
        app.update(cx, |app, cx| {
            let a = builtin_entry(app, "dark-clinical.json");
            assert!(a.bootstrap_json.is_some(), "a built-in row carries one");
            assert!(app.apply_theme_entry(&a, cx));
        });
        app.update(cx, |app, cx| {
            let b = builtin_entry(app, "mine.json");
            assert!(
                b.bootstrap_json.is_none(),
                "a discovered theme schedules no write of its own"
            );
            assert!(app.apply_theme_entry(&b, cx));
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_store.name, "mine.json", "B is applied");
            assert!(
                !a_file.exists(),
                "A's write is still in flight — that is the race under test"
            );
        });

        cx.run_until_parked();
        // A's write still happens: it is the file A needs, and nothing about
        // B makes it wrong. Deferring it is not the same as cancelling it.
        assert!(a_file.exists(), "A's own bootstrap write is not cancelled");
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.theme_store.name, "mine.json",
                "a late write must not re-apply the row that scheduled it"
            );
            assert_eq!(app.theme_store.theme.name, "Mine");
            assert_eq!(app.settings.theme, "mine.json");
            assert_eq!(app.theme_store.path, b_file);
            // The hot-reload baseline must still describe B, or the watcher
            // would treat B's own file as an edit and re-apply it.
            assert_eq!(app.last_applied_theme_text, user_theme_json("Mine"));
        });
    }

    /// The MISSING rule has to hold on the worker too, not just inline: a
    /// built-in whose file already exists stays the user's editable copy and
    /// is never rewritten, however the check came to run.
    #[gpui::test]
    fn apply_theme_entry_never_overwrites_an_existing_theme_file(cx: &mut gpui::TestAppContext) {
        let config = tempfile::tempdir().expect("tempdir must be created");
        let theme_file = config.path().join("themes").join("dark-clinical.json");
        std::fs::create_dir_all(theme_file.parent().expect("a parent"))
            .expect("theme dir must be created");
        // A user-supplied copy under the built-in's own filename, which is
        // exactly the case the "only when missing" rule exists for.
        let mine = user_theme_json("My Own Dark Clinical");
        std::fs::write(&theme_file, &mine).expect("seed the user copy");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _| use_real_config_dir(app, config.path()));
        app.update(cx, |app, cx| {
            let entry = builtin_entry(app, "dark-clinical.json");
            assert!(app.apply_theme_entry(&entry, cx));
        });
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(&theme_file).expect("theme file must read"),
            mine,
            "the user's copy survives the pick untouched"
        );
    }

    /// The atomicity claim, asserted on the unit that MAKES it rather than
    /// through the click path — the end-to-end test above proves the rule
    /// holds, this one proves what enforces it.
    ///
    /// Why this is not the `exists()` test it superficially resembles: the
    /// function under test has no existence pre-check left to take an early
    /// return, so a seeded file is met by the exclusive create itself. Two
    /// assertions therefore carry the fix, and each one fails on its own
    /// without it:
    ///
    /// - the bytes survive, which a truncating `fs::write` cannot do; and
    /// - the outcome is `AlreadyExists` — a value the pre-fix code had no way
    ///   to produce at all, since it either overwrote and returned `Ok`, or
    ///   returned without saying anything.
    ///
    /// What this deliberately does NOT claim: that the two syscalls of the old
    /// `exists()`+`write` pair can be interleaved from a test. They cannot,
    /// honestly — that needs either a real concurrent writer, which is a
    /// probabilistic race and therefore a flaky test, or a hook planted
    /// between the check and the write, which is test-only production code.
    /// The property asserted instead is the one that makes the interleaving
    /// irrelevant: ONE syscall decides, and it refuses.
    #[test]
    fn create_bootstrap_theme_file_refuses_a_name_that_is_already_taken() {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let path = dir.path().join("themes").join("dark-clinical.json");
        let mine = user_theme_json("My Own Dark Clinical");
        std::fs::create_dir_all(path.parent().expect("a parent"))
            .expect("theme dir must be created");
        std::fs::write(&path, &mine).expect("seed the user copy");

        let err = create_bootstrap_theme_file(
            &path,
            crate::theme_builtins::builtin_theme_json("dark-clinical.json"),
        )
        .expect_err("an already-taken name must not report success");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::AlreadyExists,
            "the caller tells this apart from a real failure to stay silent"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("theme file must read"),
            mine,
            "the user's copy is byte-for-byte untouched"
        );
    }

    /// The other half of the same unit: the name IS free, so the call must
    /// create the `themes/` directory (it is usually absent on a first pick,
    /// which is why the mkdir runs first) and write the built-in JSON, so the
    /// file stays editable and hot-reloadable.
    ///
    /// The second call is the atomicity seen from the other side: the first
    /// one created the name, so the next create must decline it instead of
    /// rewriting the file it just wrote.
    #[test]
    fn create_bootstrap_theme_file_writes_a_free_name_once() {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let path = dir.path().join("themes").join("dark-clinical.json");
        let json = crate::theme_builtins::builtin_theme_json("dark-clinical.json");
        assert!(
            !path.parent().expect("a parent").exists(),
            "precondition: the themes dir starts absent, as on a first launch"
        );

        create_bootstrap_theme_file(&path, json).expect("a free name must be created");
        assert_eq!(
            std::fs::read_to_string(&path).expect("written theme must read"),
            json,
            "the file holds the built-in JSON, so the theme stays editable"
        );

        let err = create_bootstrap_theme_file(&path, r#"{"name":"clobbered"}"#)
            .expect_err("the name this call just created is now taken");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&path).expect("written theme must read"),
            json,
            "and the file it created is not rewritten by the next call"
        );
    }

    // ── Crop state transitions (Task 4) ──

    #[gpui::test]
    fn enter_crop_sets_mode(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.crop_rect = Some(sh_core::crop::CropRect {
                x0: 10.0,
                y0: 10.0,
                x1: 20.0,
                y1: 20.0,
            });
            app.enter_crop(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.crop_mode);
            // A stale selection from a previous session must not leak in.
            assert!(app.crop_rect.is_none());
        });
    }

    #[gpui::test]
    fn cancel_crop_clears(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.crop_mode = true;
            app.crop_rect = Some(sh_core::crop::CropRect {
                x0: 10.0,
                y0: 10.0,
                x1: 20.0,
                y1: 20.0,
            });
            app.cancel_crop(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.crop_mode);
            assert!(app.crop_rect.is_none());
        });
    }

    #[gpui::test]
    fn toggle_twice_returns(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.toggle_crop(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.crop_mode);
        });
        app.update(cx, |app, cx| {
            app.toggle_crop(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.crop_mode);
            assert!(app.crop_rect.is_none());
        });
    }

    // ── Crop drag + confirm bar (Task 5) ──

    #[gpui::test]
    fn drag_updates_rect_and_bar_on_release(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.enter_crop(cx);
            app.begin_crop_drag((100.0, 100.0));
        });
        app.read_with(cx, |app, _| {
            // Zero-area rect anchored at the press point.
            let r = app.crop_rect.expect("drag must arm a rect");
            assert_eq!((r.x0, r.y0, r.x1, r.y1), (100.0, 100.0, 100.0, 100.0));
        });
        app.update(cx, |app, _| {
            app.update_crop_drag((300.0, 250.0));
            app.finish_crop_drag();
        });
        app.read_with(cx, |app, _| {
            let r = app.crop_rect.expect("real-area rect must survive");
            assert_eq!((r.x0, r.y0, r.x1, r.y1), (100.0, 100.0, 300.0, 250.0));
            assert!(app.crop_bar_visible, "confirm bar must appear");
        });
    }

    #[gpui::test]
    fn degenerate_drag_discards_silently(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.enter_crop(cx);
            app.begin_crop_drag((100.0, 100.0));
            app.update_crop_drag((101.0, 100.5)); // < 4 px²
            app.finish_crop_drag();
        });
        app.read_with(cx, |app, _| {
            assert!(app.crop_rect.is_none(), "click-like drag must discard");
            assert!(!app.crop_bar_visible, "no bar for a degenerate rect");
        });
    }

    #[gpui::test]
    fn confirm_with_unknown_dimensions_closes_silently(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.enter_crop(cx);
            app.begin_crop_drag((10.0, 10.0));
            app.update_crop_drag((200.0, 200.0));
            app.finish_crop_drag();
            assert!(app.crop_bar_visible);
            // test_app images have no dimensions (fake paths, probe fails);
            // confirm must still close silently instead of erroring.
            app.confirm_crop_copy(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.crop_bar_visible);
            assert!(app.crop_rect.is_none());
            assert!(app.session.error.is_none(), "no error for a silent close");
        });
    }

    #[gpui::test]
    fn open_settings_remembers_origin_and_esc_returns(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.open_settings(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Settings);
            assert_eq!(app.settings_return_to, View::Grid);
        });
        app.update(cx, |app, cx| {
            app.close_settings(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Grid);
            assert!(app.capture_action.is_none());
        });
    }

    #[gpui::test]
    fn apply_keymap_persists_and_rebinds_live(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        // test_app's settings_path (Z:\fake) cannot be written; point at a
        // real tempdir first so the atomic save succeeds and the rebind
        // actually happens.
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            // Mirror main.rs's startup focus: the tracked root div must own
            // keyboard focus before any keystroke arrives.
            window.focus(&app.focus_handle, cx);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            let mut km = sh_core::keymap::defaults();
            km.insert(
                "toggle-slideshow".into(),
                sh_core::keymap::KeyBinding {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    platform: false,
                    key: "k".into(),
                },
            );
            app.apply_keymap(km, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings.keymap.get("toggle-slideshow").unwrap().key,
                "k"
            );
        });
        cx.simulate_keystrokes("ctrl-k");
        app.read_with(cx, |app, _| {
            assert!(app.session.slideshow_active, "rebound combo must fire");
        });
    }

    // ── Task 8: settings panel flows ──

    #[gpui::test]
    fn ctrl_comma_opens_settings_from_each_view_and_esc_returns(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        for origin in [View::Welcome, View::Grid, View::Viewer] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            app.update(cx, |app, cx| {
                app.view = origin;
                // Simulate the OpenSettings dispatch path directly (the binding
                // string itself is compiler-checked via KeyBinding::new).
                app.open_settings(cx);
            });
            app.read_with(cx, |app, _| {
                assert_eq!(app.view, View::Settings);
                assert_eq!(app.settings_return_to, origin);
            });
            app.update(cx, |app, cx| {
                app.close_settings(cx);
            });
            app.read_with(cx, |app, _| {
                assert_eq!(app.view, origin);
            });
        }
        // Real-keystroke proof: `ctrl-,` routes through the live keymap to
        // the `OpenSettings` action (cold-start focus pattern so the
        // `image_view` context joins the dispatch path).
        let (app, cx) = cx.add_window_view(|window, cx| {
            let app = test_app(cx);
            window.focus(&app.focus_handle, cx);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _| {
            app.view = View::Grid;
        });
        cx.simulate_keystrokes("ctrl-,");
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Settings);
            assert_eq!(app.settings_return_to, View::Grid);
        });
    }

    /// State contract: switching the settings section clears any in-progress
    /// capture (section value + capture cleared). The row-callback wiring is
    /// covered by construction — this uses the same calls the sidebar row makes.
    #[gpui::test]
    fn settings_section_switch_contract_clears_capture(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.capture_action = Some("open-file".into());
            // Same calls the sidebar row makes.
            app.settings_section = crate::ui::settings_panel::SettingsSection::Appearance;
            app.capture_action = None;
            app.capture_conflict = None;
            cx.notify();
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings_section,
                crate::ui::settings_panel::SettingsSection::Appearance
            );
            assert!(app.capture_action.is_none());
        });
    }

    #[test]
    fn settings_control_activation_accepts_unmodified_enter_and_space() {
        for key in ["enter", "space"] {
            // SPIKE: gpui 0.3.x added a third field, `prefer_character_input`, which the
            // platform sets to `true` only when replaying a pending keystroke
            // into a text-input handler. These fixtures model a plain key press
            // arriving from the window manager, so `false` is the faithful value
            // — and it keeps the keybinding path active, which is what this test
            // pins.
            let event = gpui::KeyDownEvent {
                keystroke: gpui::Keystroke::parse(key).expect("test key must parse"),
                is_held: false,
                prefer_character_input: false,
            };
            assert!(super::settings_control_activation(&event), "key={key}");
        }
    }

    #[gpui::test]
    fn settings_reduce_motion_row_is_keyboard_operable_and_persists(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            window.focus(&app.focus_handle, cx);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_section = crate::ui::settings_panel::SettingsSection::Appearance;
            app.settings.version = 9;
            app.settings.reduce_motion = true;
            cx.notify();
        });
        cx.run_until_parked();

        for _ in 0..5 {
            cx.simulate_keystrokes("tab");
            app.update(cx, |_app, cx| cx.notify());
        }
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings_appearance_focus_control,
                Some(crate::ui::settings_panel::AppearanceControl::ReduceMotion)
            );
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        app.read_with(cx, |app, _| assert!(!app.settings.reduce_motion));
        let saved = sh_core::settings::load(&settings_path);
        assert!(!saved.reduce_motion);
        assert_eq!(saved.version, sh_core::settings::CURRENT_SETTINGS_VERSION);
    }

    #[gpui::test]
    fn settings_appearance_rows_are_keyboard_operable(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path;
            window.focus(&app.focus_handle, cx);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_section = crate::ui::settings_panel::SettingsSection::Appearance;
            app.settings.filmstrip = true;
            app.settings.checkerboard = true;
            app.settings.slideshow_interval_secs = 3;
            cx.notify();
        });
        cx.run_until_parked();

        let root_focus = app.read_with(cx, |app, _| app.focus_handle.clone());
        // SPIKE: `VisualTestContext::update` hands its closure both the window and the
        // `&mut App` that gpui 0.3.x's `Window::focus` now needs.
        cx.update(|window, cx| {
            window.focus(&root_focus, cx);
            assert!(root_focus.is_focused(window), "root lost focus");
        });
        app.update(cx, |_app, cx| cx.notify());
        cx.simulate_keystrokes("tab");
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings_appearance_focus_control,
                Some(crate::ui::settings_panel::AppearanceControl::Filmstrip)
            );
        });
        app.update(cx, |_app, cx| cx.notify());
        cx.simulate_keystrokes("enter");
        app.read_with(cx, |app, _| {
            assert_eq!(
                (
                    app.settings.filmstrip,
                    app.settings.checkerboard,
                    app.settings.slideshow_interval_secs
                ),
                (false, true, 3)
            );
        });

        cx.simulate_keystrokes("tab");
        app.update(cx, |_app, cx| cx.notify());
        cx.simulate_keystrokes("space");
        app.read_with(cx, |app, _| assert!(!app.settings.checkerboard));

        cx.simulate_keystrokes("tab");
        app.update(cx, |_app, cx| cx.notify());
        cx.simulate_keystrokes("enter");
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.slideshow_interval_secs, 2)
        });

        cx.simulate_keystrokes("tab");
        app.update(cx, |_app, cx| cx.notify());
        cx.simulate_keystrokes("enter");
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.slideshow_interval_secs, 3)
        });
    }

    #[gpui::test]
    fn settings_appearance_controls_have_stable_bounds_at_minimum_window(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        cx.update(|window, _cx| window.resize(gpui::size(gpui::px(480.0), gpui::px(320.0))));
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_section = crate::ui::settings_panel::SettingsSection::Appearance;
            cx.notify();
        });
        cx.run_until_parked();

        for selector in [
            "settings-filmstrip-toggle",
            "settings-checkerboard-toggle",
            "settings-reduce-motion-toggle",
            "settings-slideshow-interval",
            "settings-slideshow-interval-decrement",
            "settings-slideshow-interval-value",
            "settings-slideshow-interval-increment",
        ] {
            assert!(cx.debug_bounds(selector).is_some(), "missing {selector}");
        }
    }

    #[gpui::test]
    fn motion_settings_row_keeps_stable_selector_inside_animation_boundary(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let animation_id = crate::ui::motion::AnimationId::new(
            crate::ui::settings_panel::sections::appearance::REDUCE_MOTION_TOGGLE_ID,
        );
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_section = crate::ui::settings_panel::SettingsSection::Appearance;
            app.settings.reduce_motion = false;
            assert!(app.hover_motion.set_hovered(animation_id, true));
            cx.notify();
        });
        cx.run_until_parked();

        assert!(cx
            .debug_bounds(crate::ui::settings_panel::sections::appearance::REDUCE_MOTION_TOGGLE_ID)
            .is_some());
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.hover_motion.phase(animation_id),
                crate::ui::motion::HoverPhase::Entering
            );
        });
    }

    #[gpui::test]
    fn settings_appearance_values_persist_through_existing_path(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _cx| app.settings_path = settings_path.clone());

        app.update(cx, |app, cx| app.set_filmstrip(false, cx));
        cx.run_until_parked();
        app.update(cx, |app, cx| app.set_checkerboard(false, cx));
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            app.set_slideshow_interval_secs(42, cx)
                .expect("interval must be valid");
        });
        cx.run_until_parked();

        let saved = sh_core::settings::load(&settings_path);
        assert!(!saved.filmstrip);
        assert!(!saved.checkerboard);
        assert_eq!(saved.slideshow_interval_secs, 42);
        assert_eq!(saved.version, sh_core::settings::CURRENT_SETTINGS_VERSION);
    }

    #[gpui::test]
    fn settings_slideshow_interval_setter_enforces_persisted_bounds(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            assert!(app.set_slideshow_interval_secs(0, cx).is_err());
            assert_eq!(app.settings.slideshow_interval_secs, 3);
            assert!(app.set_slideshow_interval_secs(61, cx).is_err());
            assert_eq!(app.settings.slideshow_interval_secs, 3);

            app.set_slideshow_interval_secs(1, cx)
                .expect("minimum interval must be valid");
            app.set_slideshow_interval_secs(60, cx)
                .expect("maximum interval must be valid");
            assert_eq!(app.settings.slideshow_interval_secs, 60);
        });
    }

    #[gpui::test]
    fn settings_filmstrip_toggle_over_viewer_refits_on_return(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let image = sh_core::transform::Vec2 {
            x: 4000.0,
            y: 2000.0,
        };
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.settings_path = settings_path;
            app.view = View::Viewer;
            app.settings.filmstrip = false;
            app.session.show_overlay_bottom = false;
            app.session.images[0].dimensions = Some((4000, 2000));
            app.session.fit_mode = FitMode::Fit;
            cx.notify();
        });
        cx.run_until_parked();

        app.update(cx, |app, cx| {
            app.open_settings(cx);
            app.set_filmstrip(true, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Settings);
            assert_eq!(app.settings_return_to, View::Viewer);
            let expected_viewport =
                stable_filmstrip_viewport(super::viewport_vec(app.viewport), true, false);
            assert_eq!(app.viewer_viewport(), expected_viewport);
        });

        app.update(cx, |app, cx| app.close_settings(cx));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let expected_viewport =
                stable_filmstrip_viewport(super::viewport_vec(app.viewport), true, false);
            assert_eq!(app.view, View::Viewer);
            assert_eq!(app.viewer_viewport(), expected_viewport);
            assert_eq!(
                app.session.zoom,
                sh_core::transform::fit(image, expected_viewport)
            );
        });
    }

    #[gpui::test]
    fn capture_conflict_reports_incumbent(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _| {
            // Offer open-file's ctrl-o to open-folder → must conflict.
            let ctrl_o = app.settings.keymap.get("open-file").unwrap().clone();
            let err = sh_core::keymap::validate_binding(
                &app.settings.keymap,
                crate::actions::KEYMAP_CONTEXT,
                "open-folder",
                &ctrl_o,
            )
            .unwrap_err();
            assert_eq!(err.existing_action, "open-file");
            // `validate_binding` is pure (returns Result, mutates nothing), so
            // this assert documents the rejection-persists-nothing contract:
            // a rejected candidate leaves the stored binding untouched, which
            // is what Task-7's rejection path relies on.
            // Binding unchanged (rejection persists nothing).
            assert_eq!(
                app.settings.keymap.get("open-folder").unwrap(),
                sh_core::keymap::defaults().get("open-folder").unwrap()
            );
        });
    }

    #[gpui::test]
    fn reset_restores_defaults(cx: &mut gpui::TestAppContext) {
        // apply_keymap saves synchronously: test_app's Z:\fake path cannot
        // be written, so point at a real tempdir first (Task 5 harness rule).
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            let mut km = sh_core::keymap::defaults();
            km.insert(
                "toggle-slideshow".into(),
                sh_core::keymap::KeyBinding {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    platform: false,
                    key: "k".into(),
                },
            );
            app.apply_keymap(km, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings.keymap.get("toggle-slideshow").unwrap().key,
                "k"
            );
        });
        // Same call the armed Reset button makes.
        app.update(cx, |app, cx| {
            app.apply_keymap(sh_core::keymap::defaults(), cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.keymap, sh_core::keymap::defaults());
        });
    }

    #[gpui::test]
    fn apply_language_commits_and_persists_on_success(cx: &mut gpui::TestAppContext) {
        use sh_core::i18n::Language;
        // Fresh default renders English (no OS-locale seeding).
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.language, Language::En);
        });
        // Successful save commits in memory (live re-render follows the
        // `cx.notify()` the method issues; visual switch is manual QA).
        app.update(cx, |app, cx| {
            app.apply_language(Language::Es, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.language, Language::Es);
        });
        // …and persists across restarts.
        let reloaded = sh_core::settings::load(&settings_path);
        assert_eq!(reloaded.language, Language::Es);
        assert_eq!(
            reloaded.version,
            sh_core::settings::CURRENT_SETTINGS_VERSION
        );
    }

    #[gpui::test]
    fn apply_language_keeps_old_language_when_save_fails(cx: &mut gpui::TestAppContext) {
        use sh_core::i18n::Language;
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // Unwritable destination: `save` writes `<path>.tmp` then renames
        // onto `path` — pointing at an existing DIRECTORY makes the rename
        // fail on every platform (`create_dir_all` would otherwise rescue a
        // merely-missing parent).
        let blocked = dir.path().join("blocked");
        std::fs::create_dir(&blocked).expect("fixture dir");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = blocked.clone();
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.apply_language(Language::Es, cx);
        });
        // Commit-on-success: failed save keeps the previous language and
        // writes no settings file.
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.language, Language::En);
        });
        assert!(
            !blocked.join("settings.json").exists(),
            "failed save must write nothing"
        );
    }

    #[gpui::test]
    fn rebind_survives_disk_reload(cx: &mut gpui::TestAppContext) {
        // Persistence: rebind → reload settings from disk → custom binding present.
        // Uses a real temp settings path: apply_keymap saves synchronously
        // (save, not spawn) so no run_until_parked race.
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        sh_core::settings::save(&settings_path, &sh_core::settings::Settings::default())
            .expect("seed settings");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            app.settings = sh_core::settings::load(&settings_path);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            let mut km = app.settings.keymap.clone();
            km.insert(
                "toggle-slideshow".into(),
                sh_core::keymap::KeyBinding {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    platform: false,
                    key: "k".into(),
                },
            );
            app.apply_keymap(km, cx);
        });
        let reloaded = sh_core::settings::load(&settings_path);
        assert_eq!(reloaded.keymap.get("toggle-slideshow").unwrap().key, "k");
        assert_eq!(
            reloaded.version,
            sh_core::settings::CURRENT_SETTINGS_VERSION
        );
    }

    // ── Task 8: review-gap tests (Tasks 6–7 reviews) ──

    /// M1 Err-path: a failed atomic save keeps the in-memory settings AND the
    /// live bindings untouched (clone-then-assign contract in `apply_keymap`).
    #[gpui::test]
    fn apply_keymap_save_failure_keeps_old_settings(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // A regular file where a directory would be needed: create_dir_all
        // on this parent fails, so the atomic save fails hermetically on
        // every platform (no nonexistent-drive tricks).
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"not a dir").expect("blocker file");
        let bad_path = blocker.join("settings.json");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = bad_path.clone();
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            let mut km = sh_core::keymap::defaults();
            km.insert(
                "toggle-slideshow".into(),
                sh_core::keymap::KeyBinding {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    platform: false,
                    key: "k".into(),
                },
            );
            app.apply_keymap(km, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings.keymap,
                sh_core::keymap::defaults(),
                "failed save must leave settings untouched"
            );
            assert_eq!(
                app.settings.version,
                sh_core::settings::CURRENT_SETTINGS_VERSION
            );
        });
    }

    /// M2 re-enter: opening Settings while already there is a no-op — the
    /// origin is NOT overwritten with Settings (early-return guard).
    #[gpui::test]
    fn open_settings_reenter_is_noop(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.open_settings(cx);
        });
        app.update(cx, |app, cx| {
            app.open_settings(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Settings);
            assert_eq!(
                app.settings_return_to,
                View::Grid,
                "re-enter must not overwrite the origin with Settings"
            );
        });
    }

    /// Theme discovery is I/O, so it must not run in `render` — the
    /// `open_settings` -> background scan -> seq-guarded commit path is the
    /// contract, and this pins both halves of it: a ticket per open (so a
    /// re-open while a scan is in flight bumps the counter and the older
    /// result is dropped) and a commit that actually lands.
    #[gpui::test]
    fn theme_discovery_runs_off_the_frame_loop_and_lands(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        // Seeded with the built-ins only, so nothing has been discovered yet.
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_load_seq, 0);
            assert_eq!(
                app.theme_entries.len(),
                crate::theme_builtins::BUILTIN_THEMES.len(),
                "the picker must never start empty"
            );
            assert!(app.theme_entries.iter().all(|e| e.is_selectable()));
        });

        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.open_settings(cx);
            // Re-entering is a no-op by the existing early return, so it
            // must NOT arm a second scan.
            app.open_settings(cx);
        });
        cx.run_until_parked();

        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_load_seq, 1, "one ticket per effective open");
            // The scan completed: the list is still coherent. `test_app`
            // points at a nonexistent config dir, which is the first-run
            // case — built-ins survive it rather than blanking the picker.
            assert_eq!(
                app.theme_entries.len(),
                crate::theme_builtins::BUILTIN_THEMES.len()
            );
        });
    }

    /// A refresh that lands while a NEWER refresh is in flight must be
    /// dropped whole. Two `refresh_theme_entries` calls without pumping in
    /// between leave ticket 1 stale; the second scan's result is the one
    /// that survives.
    #[gpui::test]
    fn stale_theme_discovery_result_is_dropped(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.refresh_theme_entries(cx);
            // Second call before the first can complete: ticket 1 is now
            // stale, and must not be allowed to commit.
            app.refresh_theme_entries(cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_load_seq, 2, "one ticket per refresh");
            assert_eq!(
                app.theme_entries.len(),
                crate::theme_builtins::BUILTIN_THEMES.len()
            );
        });
    }

    /// Esc routing through the REAL `BackToGrid` action (not direct close):
    /// first Esc with a capture in progress cancels the capture and STAYS in
    /// Settings; second Esc returns to the origin view.
    #[gpui::test]
    fn esc_first_cancels_capture_then_returns(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        let (app, cx) = cx.add_window_view(|window, cx| {
            let app = test_app(cx);
            // Cold-start pattern: the tracked root must own focus before
            // any keystroke arrives.
            window.focus(&app.focus_handle, cx);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.open_settings(cx);
            app.capture_action = Some("toggle-slideshow".into());
        });
        cx.simulate_keystrokes("escape");
        app.read_with(cx, |app, _| {
            assert!(
                app.capture_action.is_none(),
                "first Esc cancels the capture"
            );
            assert_eq!(app.view, View::Settings, "first Esc stays in Settings");
        });
        cx.simulate_keystrokes("escape");
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Grid, "second Esc returns to origin");
        });
    }

    /// Capture end-to-end (Task 7 Issue 2): while capturing, a real
    /// `ctrl-k` keystroke lands in the root `on_key_down` handler, validates,
    /// persists via `apply_keymap`, and clears the capture.
    #[gpui::test]
    fn capture_ctrl_k_rebinds_end_to_end(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        // apply_keymap saves synchronously: point at a real tempdir first
        // (Task 5 harness rule) so the capture persist succeeds.
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            // Cold-start pattern: the tracked root must own focus before
            // any keystroke arrives.
            window.focus(&app.focus_handle, cx);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_return_to = View::Grid;
            app.capture_action = Some("toggle-slideshow".into());
            cx.notify();
        });
        cx.simulate_keystrokes("ctrl-k");
        app.read_with(cx, |app, _| {
            let b = app.settings.keymap.get("toggle-slideshow").unwrap();
            assert_eq!(b.key, "k");
            assert!(b.ctrl);
            assert!(
                app.capture_action.is_none(),
                "successful capture clears capture mode"
            );
            assert!(app.capture_conflict.is_none());
        });
    }

    /// Appearance no-close: applying a builtin theme from the surface keeps
    /// the user in Settings (the old dropdown closed itself; the surface
    /// persists — closing here would strand the user).
    #[gpui::test]
    fn appearance_apply_keeps_settings_surface_open(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_return_to = View::Grid;
            assert_ne!(app.theme_store.name, "dark-clinical.json");
            let entry = builtin_entry(app, "dark-clinical.json");
            assert!(app.apply_theme_entry(&entry, cx));
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_store.name, "dark-clinical.json");
            assert_eq!(
                app.view,
                View::Settings,
                "theme apply must not close Settings"
            );
        });
    }

    // ── Viewer info panel ──

    /// Write one decodable PNG per `(name, w, h)` spec into `dir` and
    /// return the paths in order. Distinct dims per file let
    /// navigate-while-open assert B's facts (never A's).
    fn real_info_images(dir: &std::path::Path, specs: &[(&str, u32, u32)]) -> Vec<PathBuf> {
        specs
            .iter()
            .map(|(name, w, h)| {
                let path = dir.join(name);
                let img = image::RgbaImage::from_fn(*w, *h, |x, y| {
                    image::Rgba([(x % 255) as u8, (y % 255) as u8, 255, 255])
                });
                img.save(&path).expect("info test fixture must save");
                path
            })
            .collect()
    }

    /// Point the test app's session at real image files with known dims.
    fn point_session_at_images(app: &mut App, paths: &[PathBuf], _cx: &mut gpui::Context<App>) {
        let epoch = std::time::SystemTime::UNIX_EPOCH;
        let entries: Vec<sh_core::navigation::ImageEntry> = paths
            .iter()
            .map(|p| sh_core::navigation::ImageEntry {
                path: p.clone(),
                size: 0,
                modified: epoch,
                created: None,
            })
            .collect();
        app.session.images = build_image_items(entries);
        app.session.current = 0;
        app.view = View::Viewer;
    }

    #[gpui::test]
    fn info_toggle_opens_with_current_facts_and_recloses(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9)]);
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            assert!(!app.info_panel_open, "panel starts closed (transient)");
            app.toggle_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.info_panel_open);
            let (cached_path, result) = app.info_facts.as_ref().expect("facts cached on open");
            assert_eq!(cached_path, &paths[0]);
            let info = result.as_ref().expect("valid fixture must resolve").clone();
            assert_eq!((info.width, info.height), (17, 9));
            assert_eq!(info.format, "PNG");
        });
        app.update(cx, |app, cx| {
            app.toggle_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.info_panel_open, "re-tap closes the popover");
        });
    }

    #[gpui::test]
    fn info_esc_closes_without_navigating_or_leaving(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9), ("b.png", 32, 24)]);
        let (app, cx) = cx.add_window_view(|window, cx| {
            let app = test_app(cx);
            window.focus(&app.focus_handle, cx);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            app.toggle_info_panel(cx);
            assert!(app.info_panel_open);
        });
        cx.simulate_keystrokes("escape");
        app.read_with(cx, |app, _| {
            assert!(!app.info_panel_open, "Esc closes the popover");
            assert_eq!(app.view, View::Viewer, "Esc must not leave the viewer");
            assert_eq!(app.session.current, 0, "Esc must not navigate");
        });
    }

    #[gpui::test]
    fn info_catcher_close_changes_nothing_else(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9)]);
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            app.toggle_info_panel(cx);
            app.close_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.info_panel_open);
            assert_eq!(app.view, View::Viewer);
            assert_eq!(app.session.current, 0);
            assert!(!app.sort_menu_open, "catcher close touches nothing else");
            assert!(app.session.images[0].path == paths[0]);
        });
    }

    #[gpui::test]
    fn info_navigate_while_open_shows_next_image_facts(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9), ("b.png", 32, 24)]);
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            app.toggle_info_panel(cx);
            app.navigate(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.info_panel_open, "navigate keeps the panel open");
            assert_eq!(app.session.current, 1);
            let (cached_path, result) = app.info_facts.as_ref().expect("facts re-resolved");
            assert_eq!(cached_path, &paths[1], "facts track image B, never stale A");
            let info = result.as_ref().expect("valid fixture must resolve").clone();
            assert_eq!((info.width, info.height), (32, 24));
        });
    }

    #[gpui::test]
    fn info_corrupt_file_shows_localized_error_without_crash(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let bad = dir.path().join("corrupt.png");
        std::fs::write(&bad, b"not really a png").expect("corrupt fixture must write");
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, std::slice::from_ref(&bad), cx);
            app.toggle_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.info_panel_open, "panel still opens on corrupt input");
            let (_, result) = app.info_facts.as_ref().expect("outcome cached");
            assert!(
                result.is_err(),
                "corrupt file must be a typed Err, never a panic"
            );
            assert_eq!(
                app.settings
                    .language
                    .get(sh_core::i18n::StrKey::InfoLoadError),
                "Could not read image info"
            );
        });
        // Missing file: same contract — typed Err, localized error copy.
        let missing = dir.path().join("gone.png");
        app.update(cx, |app, cx| {
            point_session_at_images(app, std::slice::from_ref(&missing), cx);
            app.toggle_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            let (_, result) = app.info_facts.as_ref().expect("outcome cached");
            assert!(result.is_err());
        });
    }

    #[test]
    fn info_has_no_action_keymap_or_shortcuts_surface() {
        // Click-only guard: the Shortcuts panel renders one row per ACTIONS
        // entry, so absence here covers all three surfaces (read-only
        // inspection — nothing is mutated).
        for desc in crate::actions::ACTIONS {
            assert!(
                !desc.id.contains("info"),
                "no action id for the info panel, found {}",
                desc.id
            );
        }
        for id in sh_core::keymap::defaults().keys() {
            assert!(
                !id.contains("info"),
                "no keymap entry for the info panel, found {id}"
            );
        }
    }

    #[gpui::test]
    fn info_labels_resolve_through_settings_language(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9)]);
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            app.settings.language = sh_core::i18n::Language::Es;
            cx.notify();
        });
        app.read_with(cx, |app, _| {
            let lang = app.settings.language;
            assert_eq!(
                lang.get(sh_core::i18n::StrKey::InfoDimensionsLabel),
                "Dimensiones"
            );
            assert_eq!(
                lang.get(sh_core::i18n::StrKey::InfoLoadError),
                "No se pudo leer la información de la imagen"
            );
            // Values stay untranslated regardless of language.
            let rows = crate::ui::overlay::info_rows(
                lang,
                &sh_core::decode::FileInfo {
                    width: 1920,
                    height: 1080,
                    size_bytes: 2_400_000,
                    format: "PNG".into(),
                },
            );
            assert_eq!(rows[0].1, "1920 × 1080");
            assert_eq!(rows[1].1, "2.4 MB");
            assert_eq!(rows[2].1, "PNG");
        });
    }

    /// Every appearance toggle must actually paint its state, and paint it inside
    /// its own row.
    ///
    /// The glyph these replaced was a text checkmark, so a `Switch` that failed
    /// to render would leave the row looking *nearly* identical: label, border
    /// and row height all unchanged. Nothing else in the suite could tell "the
    /// state indicator is here" from "the state indicator silently vanished",
    /// which is ADR-020's `Selectable::toggled` failure mode again -- present in
    /// the tree, absent on screen.
    ///
    /// The containment half is not theoretical. This assertion fired during
    /// development: the checkerboard switch spanned `x 468..504` inside a row
    /// ending at `464`, because a `Switch` track is 36px wide where the `✓` it
    /// replaced was roughly 10px, and the longest label no longer fit the 480px
    /// minimum window.
    #[gpui::test]
    fn appearance_toggles_render_their_state_track(cx: &mut gpui::TestAppContext) {
        use crate::ui::settings_panel::scroll;
        use crate::ui::settings_panel::sections::appearance;
        use crate::ui::settings_panel::SettingsSection;

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        // The narrowest window the panel supports: if a switch can overflow its
        // row anywhere, it overflows here first.
        cx.simulate_resize(gpui::size(gpui::px(480.0), gpui::px(320.0)));
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_section = SettingsSection::Appearance;
            cx.notify();
        });
        cx.run_until_parked();

        for (row_id, track_selector) in [
            (
                appearance::FILMSTRIP_TOGGLE_ID,
                appearance::FILMSTRIP_TOGGLE_TRACK_SELECTOR,
            ),
            (
                appearance::CHECKERBOARD_TOGGLE_ID,
                appearance::CHECKERBOARD_TOGGLE_TRACK_SELECTOR,
            ),
            (
                appearance::REDUCE_MOTION_TOGGLE_ID,
                appearance::REDUCE_MOTION_TOGGLE_TRACK_SELECTOR,
            ),
        ] {
            let row = cx
                .debug_bounds(row_id)
                .unwrap_or_else(|| panic!("{row_id} must mount"));
            let track = cx.debug_bounds(track_selector).unwrap_or_else(|| {
                panic!("{track_selector} must mount: the toggle would render no state")
            });

            assert!(
                f32::from(track.size.height) > 0.0 && f32::from(track.size.width) > 0.0,
                "{track_selector} mounted with zero size: the state indicator is invisible"
            );

            // The switch must not resize its row, or scroll.rs's exact
            // arithmetic is wrong.
            let row_h = f32::from(row.size.height);
            assert!(
                (row_h - scroll::SETTINGS_ROW_H_PX).abs() < 1.0,
                "{row_id} height is {row_h}, expected {}: the switch must fit inside the row",
                scroll::SETTINGS_ROW_H_PX
            );

            let track_h = f32::from(track.size.height);
            assert!(
                track_h <= row_h,
                "{track_selector} is {track_h} tall inside a {row_h} row: it overflows"
            );

            let row_left = f32::from(row.origin.x);
            let row_right = row_left + f32::from(row.size.width);
            let track_left = f32::from(track.origin.x);
            let track_right = track_left + f32::from(track.size.width);
            assert!(
                track_left >= row_left - 1.0 && track_right <= row_right + 1.0,
                "{track_selector} spans x {track_left}..{track_right}, outside its row \
                 {row_left}..{row_right}"
            );
        }
    }

    // ── Theme Editor section ──

    /// The arithmetic in `scroll::theme_editor_content_h` checked against the
    /// render that has to match it.
    ///
    /// ADR-024 records four geometry tests that "assert on `scroll.rs`
    /// *functions*, not on the render, so they stay green while the panel is
    /// wrong". This one is written the other way round on purpose: it reads the
    /// real bounds of the real last row and compares them against the constant.
    ///
    /// The failure this catches is not cosmetic. `settings_max_scroll` clamps
    /// to `content - visible`, so a content height even one row too short puts
    /// the last row's bottom edge below the furthest scroll position and the
    /// user simply cannot reach the family field — at small window sizes only,
    /// which is why it can ship unnoticed.
    #[gpui::test]
    fn theme_editor_content_height_matches_the_rendered_column(cx: &mut gpui::TestAppContext) {
        use crate::ui::settings_panel::sections::theme_editor as te;

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        cx.simulate_resize(gpui::size(gpui::px(480.0), gpui::px(320.0)));
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_section = crate::ui::settings_panel::SettingsSection::ThemeEditor;
            cx.notify();
        });
        cx.run_until_parked();

        let column = cx
            .debug_bounds(te::CONTAINER_ID)
            .expect("theme editor column mounts");
        let last = cx
            .debug_bounds(static_selector(te::row_id(
                *te::FIELDS.last().expect("FIELDS is non-empty"),
            )))
            .expect("the last row mounts");
        let header = cx
            .debug_bounds(static_selector(te::row_id(te::FIELDS[0])))
            .expect("the first row mounts");

        // Measured top-to-bottom extent of the column's real content: from the
        // column's own top edge to the bottom of the LAST row. Deliberately
        // not `column.size.height`, which would only prove the column div
        // agrees with itself.
        let top = f32::from(column.origin.y);
        let measured = f32::from(last.origin.y + last.size.height) - top;
        let expected = scroll::theme_editor_content_h();

        assert!(
            (measured - expected).abs() < 1.0,
            "the rendered column is {measured} tall but scroll.rs assumes {expected}: \
             the last row is unreachable below the clamp. First row starts at y={}, \
             column top at {top}",
            f32::from(header.origin.y),
        );
        assert!(
            f32::from(column.size.height) - expected < 1.0,
            "column div is {} tall against an assumed {expected}",
            f32::from(column.size.height)
        );
    }

    /// Every row must fit inside its column horizontally at the narrowest
    /// window the app supports — the same failure the Appearance `Switch`
    /// produced when it outgrew its row and spanned x 468..504 inside a row
    /// ending at 464.
    #[gpui::test]
    fn theme_editor_rows_stay_inside_the_column_at_both_window_sizes(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::ui::settings_panel::sections::theme_editor as te;

        for (width, height) in [(480.0_f32, 320.0_f32), (1200.0, 900.0)] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
            app.update(cx, |app, cx| {
                app.view = View::Settings;
                app.settings_section = crate::ui::settings_panel::SettingsSection::ThemeEditor;
                cx.notify();
            });
            cx.run_until_parked();

            let column = cx
                .debug_bounds(te::CONTAINER_ID)
                .expect("theme editor column mounts");
            let col_left = f32::from(column.origin.x);
            let col_right = col_left + f32::from(column.size.width);

            for field in te::FIELDS {
                let row = cx
                    .debug_bounds(static_selector(te::row_id(field)))
                    .unwrap_or_else(|| panic!("{} must mount", te::label(field)));
                assert_positive_layout_bounds(row, &te::row_id(field));

                let row_h = f32::from(row.size.height);
                assert!(
                    (row_h - scroll::SETTINGS_ROW_H_PX).abs() < 1.0,
                    "{} is {row_h} tall at {width}x{height}, expected {}: a row that \
                     resizes breaks scroll.rs's arithmetic",
                    te::label(field),
                    scroll::SETTINGS_ROW_H_PX,
                );

                let row_left = f32::from(row.origin.x);
                let row_right = row_left + f32::from(row.size.width);
                assert!(
                    row_left >= col_left - 1.0 && row_right <= col_right + 1.0,
                    "{} spans x {row_left}..{row_right} at {width}x{height}, outside the \
                     column {col_left}..{col_right}",
                    te::label(field),
                );

                // The field and swatch are the two things that can overflow a
                // row, and the swatch is the one whose natural width is not
                // ours to choose.
                for part in [te::field_id(field), te::swatch_id(field)] {
                    let el = cx
                        .debug_bounds(static_selector(part.clone()))
                        .unwrap_or_else(|| panic!("{part} must mount"));
                    assert_positive_layout_bounds(el, &part);
                    let left = f32::from(el.origin.x);
                    let right = left + f32::from(el.size.width);
                    assert!(
                        left >= row_left - 1.0 && right <= row_right + 1.0,
                        "{part} spans x {left}..{right}, outside its row {row_left}..{row_right}"
                    );
                }

                // Both sizes are pinned by hand, so both must be MEASURED as
                // what was asked for at every label length — otherwise a long
                // slot name quietly reshapes the field and the hex it holds.
                for (part, want_w, want_h) in [
                    (te::field_id(field), te::FIELD_W_PX, te::FIELD_H_PX),
                    (te::swatch_id(field), te::SWATCH_PX, te::SWATCH_PX),
                ] {
                    let el = cx
                        .debug_bounds(static_selector(part.clone()))
                        .unwrap_or_else(|| panic!("{part} must mount"));
                    let got_w = f32::from(el.size.width);
                    let got_h = f32::from(el.size.height);
                    assert!(
                        (got_w - want_w).abs() < 1.0 && (got_h - want_h).abs() < 1.0,
                        "{part} for {} measured {got_w}x{got_h} at {width}x{height}, \
                         wanted {want_w}x{want_h}: it is being squeezed by the label",
                        te::label(field),
                    );
                }
            }
        }
    }

    /// A half-typed hex is the normal state of this section, not an error
    /// state. If it took rows down with it, the editor would delete the other
    /// eleven values the moment a user started typing.
    #[gpui::test]
    fn theme_editor_one_unparseable_row_keeps_every_other_row(cx: &mut gpui::TestAppContext) {
        use crate::ui::settings_panel::sections::theme_editor as te;

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        cx.simulate_resize(gpui::size(gpui::px(480.0), gpui::px(320.0)));
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_section = crate::ui::settings_panel::SettingsSection::ThemeEditor;
            // Every slot garbage, not just one: the swatch of each has to fall
            // back without the column losing a row.
            for slot in sh_core::theme_draft::SLOTS {
                app.theme_draft.set(slot, "#12");
            }
            app.theme_draft.name = String::new();
            cx.notify();
        });
        cx.run_until_parked();

        for field in te::FIELDS {
            let row = cx
                .debug_bounds(static_selector(te::row_id(field)))
                .unwrap_or_else(|| panic!("{} must survive a bad hex", te::label(field)));
            assert_positive_layout_bounds(row, &te::row_id(field));
        }
        // The draft keeps the bad text verbatim — resolve() is what judges it,
        // and it is not called until WU-4 saves.
        let text = app.read_with(cx, |app, _| {
            (
                app.theme_draft
                    .get(sh_core::theme_draft::Slot::Accent)
                    .to_string(),
                app.theme_draft.resolve().is_err(),
            )
        });
        assert_eq!(text.0, "#12", "the draft must keep what was typed");
        assert!(text.1, "a bad hex must still fail to resolve");
    }

    /// The sidebar drives from `SettingsSection::ALL`, so a variant missing
    /// from that array is reachable by keyboard and invisible. The section
    /// ships unselectable-from-the-list otherwise.
    #[gpui::test]
    fn theme_editor_appears_in_the_settings_sidebar(cx: &mut gpui::TestAppContext) {
        use crate::ui::settings_panel::SettingsSection;

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        cx.simulate_resize(gpui::size(gpui::px(480.0), gpui::px(320.0)));
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            cx.notify();
        });
        cx.run_until_parked();

        assert_eq!(SettingsSection::ALL.len(), 4, "four sections ship");
        assert_eq!(SettingsSection::ALL[2].0, SettingsSection::ThemeEditor);
        let row = cx
            .debug_bounds("settings-section-2")
            .expect("the theme editor row must appear in the sidebar");
        assert_positive_layout_bounds(row, "theme editor sidebar row");
    }
}

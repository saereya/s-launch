use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, CssProvider, Entry, EventControllerKey,
    Image, Label, ListBox, ListBoxRow, Orientation, Picture, PropagationPhase, ScrolledWindow,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::daemon::DaemonEvent;
use crate::plugins::{
    apps::AppsPlugin, commands::CommandsPlugin, emoji::EmojiPlugin, math::MathPlugin,
    power::PowerPlugin, Entry as AppEntry, EntryKind, Plugin,
};
use crate::search::Searcher;

fn load_user_css() -> String {
    let path = crate::config::style_path();
    match std::fs::read_to_string(&path) {
        Ok(css) => css,
        Err(_) => {
            tracing::warn!(
                "No stylesheet at {}; window will use system theme",
                path.display()
            );
            String::new()
        }
    }
}

// ── UI state ──────────────────────────────────────────────────────────────────

struct UiState {
    config: Config,
    searcher: Searcher,
    query: String,
    selected: usize,
    results: Vec<AppEntry>,
    /// Whether the user has moved the selection off the top row for the current
    /// query. Until they have, a result set arriving late re-selects the top
    /// row; afterwards it keeps the highlight on whatever they picked.
    selection_moved: bool,
}

impl UiState {
    /// Recompute results for a query the user just changed. The result set is
    /// semantically new, so the selection resets to the top.
    fn refresh_results(&mut self) {
        if self.query.starts_with('=') {
            let mut math_out = Vec::new();
            MathPlugin.query(&self.query, &mut math_out);
            self.results = math_out;
        } else if self.query.starts_with(':') {
            let mut emoji_out = Vec::new();
            EmojiPlugin.query(&self.query, &mut emoji_out);
            emoji_out.truncate(self.config.window.max_results);
            self.results = emoji_out;
        } else {
            self.searcher.update_pattern(&self.query);
            self.results = self.matcher_results();
        }
        self.selected = 0;
        self.selection_moved = false;
    }

    /// Re-poll the matcher for the *unchanged* query and adopt the result if it
    /// differs. Returns whether the list needs rebuilding.
    ///
    /// nucleo matches on background threads, so what `refresh_results` rendered
    /// may only be part of the match set. The matcher's notify callback calls
    /// this until it settles.
    fn resync_results(&mut self) -> bool {
        if self.query.starts_with('=') || self.query.starts_with(':') {
            return false; // prefix plugins bypass the matcher entirely
        }
        let fresh = self.matcher_results();
        if fresh == self.results {
            return false;
        }
        self.adopt_results(fresh);
        true
    }

    /// Recompute after the *entry list* changed under us (rescan or reload).
    /// The matcher was rebuilt, so the pattern has to be re-applied.
    ///
    /// Returns whether anything actually moved: a rescan that finds the same
    /// entries — the common case, since any write to a `$PATH` directory
    /// triggers one — must not rebuild the list while the window is open.
    fn refresh_after_entry_change(&mut self) -> bool {
        if self.query.starts_with('=') || self.query.starts_with(':') {
            return false;
        }
        self.searcher.update_pattern(&self.query);
        self.resync_results()
    }

    fn matcher_results(&mut self) -> Vec<AppEntry> {
        self.searcher
            .results(self.config.window.max_results)
            .into_iter()
            .map(|m| m.entry)
            .collect()
    }

    /// Swap in a new result list without yanking the highlight off the entry the
    /// user deliberately selected, as long as it survived into the new list.
    fn adopt_results(&mut self, fresh: Vec<AppEntry>) {
        let anchor = if self.selection_moved {
            self.results.get(self.selected).cloned()
        } else {
            None
        };
        self.results = fresh;
        self.selected = anchor
            .and_then(|a| self.results.iter().position(|e| *e == a))
            .unwrap_or(0);
    }

    fn launch_at(&self, index: usize) {
        if let Some(entry) = self.results.get(index) {
            match &entry.kind {
                EntryKind::App { .. } => {
                    AppsPlugin::new(self.config.plugins.terminal.clone()).launch(entry)
                }
                EntryKind::Command { .. } => {
                    CommandsPlugin::new(self.config.plugins.terminal.clone()).launch(entry)
                }
                EntryKind::MathResult { .. } => MathPlugin.launch(entry),
                EntryKind::EmojiResult { .. } => EmojiPlugin.launch(entry),
                EntryKind::Power { .. } => {
                    PowerPlugin::new(self.config.plugins.power_commands.clone()).launch(entry)
                }
            }
        }
    }
}

// ── Config-driven widget geometry ─────────────────────────────────────────────

/// The widgets whose appearance comes from `[window]`/`[input]` config.
///
/// Grouped so `apply_config` can run on reload as well as at build time.
/// Previously all of this was set inline while constructing the window, so
/// `slaunch reload` silently ignored every one of these settings — only
/// `max_results`, `terminal` and the plugin toggles actually took effect,
/// despite the docs promising config applies live.
struct Widgets {
    window: ApplicationWindow,
    content: GtkBox,
    scroll: ScrolledWindow,
    search_entry: Entry,
}

impl Widgets {
    fn apply_config(&self, config: &Config) {
        let w = &config.window;

        self.window.set_default_width(w.width as i32);

        // Anchors are set explicitly in every branch: reload can move the window
        // between positions, so "center" has to actively clear the edges rather
        // than rely on them never having been set.
        let (top, bottom) = match w.anchor.as_str() {
            "bottom" => (false, true),
            "center" => (false, false),
            _ => (true, false), // "top" (default)
        };
        self.window.set_anchor(Edge::Top, top);
        self.window.set_anchor(Edge::Bottom, bottom);
        if top {
            self.window.set_margin(Edge::Top, w.margin as i32);
        }
        if bottom {
            self.window.set_margin(Edge::Bottom, w.margin as i32);
        }

        let p = w.padding as i32;
        self.content.set_spacing(p);
        self.content.set_margin_start(p);
        self.content.set_margin_end(p);
        self.content.set_margin_top(p);
        self.content.set_margin_bottom(p);

        // Config clamping keeps this product inside i32.
        self.scroll
            .set_max_content_height((w.max_results * w.item_height as usize) as i32);

        self.search_entry
            .set_placeholder_text(Some(config.input.placeholder.as_str()));
    }
}

// ── Scroll helper ─────────────────────────────────────────────────────────────

fn scroll_to_row(scroll: &ScrolledWindow, row: &ListBoxRow) {
    let Some(list) = row.parent() else { return };
    let Some(bounds) = row.compute_bounds(&list) else {
        return;
    };

    let adj = scroll.vadjustment();
    let row_y = bounds.y() as f64;
    let row_h = bounds.height() as f64;
    let current = adj.value();
    let page = adj.page_size();

    if row_y < current {
        adj.set_value(row_y);
    } else if row_y + row_h > current + page {
        adj.set_value(row_y + row_h - page);
    }
}

// ── List helpers ──────────────────────────────────────────────────────────────

fn rebuild_list(list: &ListBox, state: &UiState) {
    list.remove_all();

    for entry in &state.results {
        let hbox = GtkBox::new(Orientation::Horizontal, 8);

        if let Some(icon_str) = &entry.icon {
            if std::path::Path::new(icon_str).is_absolute() {
                // Picture scales to fill its allocation; can_shrink prevents it from
                // expanding the row when the source image is larger than 32px.
                let pic = Picture::for_filename(icon_str.as_str());
                pic.set_can_shrink(true);
                pic.set_size_request(32, 32);
                pic.set_halign(Align::Center);
                pic.set_valign(Align::Center);
                hbox.append(&pic);
            } else {
                let img = Image::from_icon_name(icon_str);
                img.set_pixel_size(32);
                img.set_valign(Align::Center);
                hbox.append(&img);
            }
        }

        let vbox = GtkBox::new(Orientation::Vertical, 2);
        vbox.set_hexpand(true);
        vbox.set_valign(Align::Center);

        let name = Label::builder()
            .label(entry.name.as_str())
            .xalign(0.0)
            .hexpand(true)
            .build();
        vbox.append(&name);

        if let Some(desc) = &entry.description {
            let desc_lbl = Label::builder()
                .label(desc.as_str())
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["row-desc"])
                .build();
            vbox.append(&desc_lbl);
        }

        hbox.append(&vbox);

        let row = ListBoxRow::new();
        row.set_child(Some(&hbox));
        list.append(&row);
    }

    if !state.results.is_empty() {
        let idx = state.selected.min(state.results.len() - 1) as i32;
        list.select_row(list.row_at_index(idx).as_ref());
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Everything `build_ui` needs, handed over once when GTK activates.
///
/// Config and entries are passed in rather than read back out of `DaemonState`:
/// `build_ui` used to `try_read()` both and fall back to `Config::default()` plus
/// an empty list on failure, which meant a reload or a rescan holding the write
/// lock during startup produced an unstyled, empty launcher that only recovered
/// on the next reload. `run_daemon` already owns both values.
struct Startup {
    events: async_channel::Receiver<DaemonEvent>,
    config: Config,
    entries: Vec<AppEntry>,
}

pub fn run(
    config: Config,
    entries: Vec<AppEntry>,
    ipc_rx: mpsc::Receiver<DaemonEvent>,
) -> anyhow::Result<()> {
    gtk4::init().map_err(|e| anyhow::anyhow!("GTK init: {e}"))?;

    // Check before touching layer-shell: gtk4-layer-shell aborts the process
    // via g_error if the GDK backend isn't Wayland, which would kill the daemon
    // on a cryptic assertion instead of telling the user what's wrong.
    if !gtk4_layer_shell::is_supported() {
        anyhow::bail!(
            "the wlr-layer-shell protocol is not available on this display.\n\
             slaunch needs a Wayland compositor that implements \
             zwlr_layer_shell_v1 (Sway, Hyprland, river, ...).\n\
             GNOME/Mutter and X11 do not support it."
        );
    }

    // Bridge: tokio mpsc → async_channel (Sender is Send, safe from bg thread).
    // glib::MainContext::channel was removed in glib 0.18; async_channel is the
    // replacement that works with glib's spawn_local.
    let (sender, receiver) = async_channel::bounded::<DaemonEvent>(32);
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("IPC bridge runtime failed: {e}");
                return;
            }
        };
        rt.block_on(async move {
            let mut rx = ipc_rx;
            while let Some(ev) = rx.recv().await {
                if sender.send(ev).await.is_err() {
                    break;
                }
            }
        });
    });

    // NON_UNIQUE is deliberate. GApplication's default single-instance handling
    // registers the app id on the session bus, and a second process that finds
    // the name taken becomes a *remote* instance: it forwards `activate` to the
    // primary and `run()` returns immediately, so the daemon would exit
    // silently with status 0 after already binding its IPC socket — leaving a
    // stale socket and no explanation. slaunch does its own single-instance
    // check when it claims that socket (`daemon::bind`), which reports the
    // conflict properly, so GTK's version is redundant here as well as harmful.
    let app = Application::builder()
        .application_id("dev.slaunch")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    // Cell lets us take the startup payload exactly once inside the activate
    // closure without needing Rc (connect_activate doesn't require Send).
    let startup = Cell::new(Some(Startup {
        events: receiver,
        config,
        entries,
    }));

    app.connect_activate(move |app| {
        if let Some(startup) = startup.take() {
            build_ui(app, startup);
        }
    });

    app.run_with_args::<String>(&[]);
    Ok(())
}

// ── UI construction ───────────────────────────────────────────────────────────

fn build_ui(app: &Application, startup: Startup) {
    let Startup {
        events: rx,
        config,
        entries,
    } = startup;
    let app = app.clone();

    // CSS at highest priority — fully overrides the system theme for our window.
    let provider = CssProvider::new();
    provider.load_from_string(&load_user_css());
    let Some(display) = gtk4::gdk::Display::default() else {
        tracing::error!("No GTK display available");
        return;
    };
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );

    // Window
    let window = ApplicationWindow::builder().application(&app).build();
    window.add_css_class("slaunch");
    window.set_decorated(false);

    // Layer shell
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_namespace("slaunch");
    window.set_exclusive_zone(-1);

    // Widgets
    let search_entry = Entry::builder().hexpand(true).build();

    let list = ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);

    let scroll = ScrolledWindow::builder()
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .build();
    scroll.set_child(Some(&list));

    let content = GtkBox::new(Orientation::Vertical, 0);
    content.append(&search_entry);
    content.append(&scroll);
    window.set_child(Some(&content));

    let widgets = Widgets {
        window: window.clone(),
        content: content.clone(),
        scroll: scroll.clone(),
        search_entry: search_entry.clone(),
    };
    widgets.apply_config(&config);

    // Initial search state. The matcher runs on background threads and signals
    // completion through `notify`; this channel relays that onto the glib main
    // loop, where the list can actually be rebuilt. Capacity 1 with try_send
    // coalesces bursts — one pending wake-up is all we need.
    let (redraw_tx, redraw_rx) = async_channel::bounded::<()>(1);
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = redraw_tx.try_send(());
    });

    let mut searcher = Searcher::new(entries, notify);
    searcher.update_pattern("");
    let initial_results = searcher
        .results(config.window.max_results)
        .into_iter()
        .map(|m| m.entry)
        .collect();

    let state = Rc::new(RefCell::new(UiState {
        config,
        searcher,
        query: String::new(),
        selected: 0,
        results: initial_results,
        selection_moved: false,
    }));

    rebuild_list(&list, &state.borrow());

    // ── Search input ──────────────────────────────────────────────────────────
    {
        let list = list.clone();
        let state = state.clone();
        search_entry.connect_changed(move |entry| {
            {
                let mut s = state.borrow_mut();
                s.query = entry.text().to_string();
                s.refresh_results();
            }
            rebuild_list(&list, &state.borrow());
        });
    }

    // ── Keyboard navigation ───────────────────────────────────────────────────
    // Capture phase intercepts keys before Entry sees them, so arrow/enter/esc
    // always work even with focus on the text field.
    {
        let list = list.clone();
        let scroll = scroll.clone();
        let state = state.clone();
        let window_ref = window.clone();
        let entry_ref = search_entry.clone();

        let key_ctrl = EventControllerKey::new();
        key_ctrl.set_propagation_phase(PropagationPhase::Capture);
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            use gtk4::gdk::Key as K;

            // Hide before clearing, in both of these: set_text("") fires
            // connect_changed, which re-runs the query and rebuilds the list, so
            // clearing first left the window on screen for the duration of that
            // work rather than disappearing immediately.
            if key == K::Escape {
                window_ref.set_visible(false);
                entry_ref.set_text(""); // fires connect_changed → clears query + list
                return glib::Propagation::Stop;
            }

            if key == K::Return || key == K::KP_Enter {
                {
                    let s = state.borrow();
                    s.launch_at(s.selected);
                }
                window_ref.set_visible(false);
                entry_ref.set_text(""); // fires connect_changed
                return glib::Propagation::Stop;
            }

            if key == K::Down || key == K::Tab {
                {
                    let mut s = state.borrow_mut();
                    if !s.results.is_empty() {
                        s.selected = (s.selected + 1).min(s.results.len() - 1);
                        s.selection_moved = true;
                    }
                }
                let idx = state.borrow().selected as i32;
                list.select_row(list.row_at_index(idx).as_ref());
                if let Some(row) = list.row_at_index(idx) {
                    scroll_to_row(&scroll, &row);
                }
                return glib::Propagation::Stop;
            }

            // ISO_Left_Tab is the keyval compositors emit for Shift+Tab.
            if key == K::Up || key == K::ISO_Left_Tab {
                {
                    let mut s = state.borrow_mut();
                    if s.selected > 0 {
                        s.selected -= 1;
                        s.selection_moved = true;
                    }
                }
                let idx = state.borrow().selected as i32;
                list.select_row(list.row_at_index(idx).as_ref());
                if let Some(row) = list.row_at_index(idx) {
                    scroll_to_row(&scroll, &row);
                }
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
        });
        window.add_controller(key_ctrl);
    }

    // ── Mouse click ───────────────────────────────────────────────────────────
    {
        let state = state.clone();
        let window_ref = window.clone();
        let entry_ref = search_entry.clone();
        list.connect_row_activated(move |_, row| {
            {
                let s = state.borrow();
                s.launch_at(row.index() as usize);
            }
            window_ref.set_visible(false);
            entry_ref.set_text(""); // fires connect_changed
        });
    }

    // ── Persist across compositor dismissals ────────────────────────────────
    // The launcher is a long-lived daemon whose window is only ever hidden, not
    // destroyed. GtkApplication keeps app.run() alive only while it owns a
    // window, so if the compositor closes the layer surface (output hotplug,
    // monitor sleep/wake, reconfigure, dismiss gesture), GTK's default handler
    // would destroy the toplevel, drop the window count to zero, and return from
    // app.run() — silently killing the daemon. Intercept the request, hide
    // instead, and stop propagation so the default destroy handler never runs.
    {
        let entry_ref = search_entry.clone();
        window.connect_close_request(move |win| {
            win.set_visible(false);
            entry_ref.set_text(""); // fires connect_changed → clears query + list
            glib::Propagation::Stop
        });
    }

    // ── Late matcher results ─────────────────────────────────────────────────
    // Fuzzy matching is asynchronous, so the set rendered on a keystroke can be
    // partial. The matcher wakes us here when more has landed; without this the
    // list would keep showing truncated results until the next keystroke.
    {
        let list = list.clone();
        let state = state.clone();

        glib::MainContext::default().spawn_local(async move {
            while redraw_rx.recv().await.is_ok() {
                if state.borrow_mut().resync_results() {
                    rebuild_list(&list, &state.borrow());
                }
            }
        });
    }

    // ── IPC events on the glib main loop ─────────────────────────────────────
    // spawn_local runs this future on the glib event loop (main thread only),
    // so Rc<RefCell<...>> and GTK widgets can be captured without Send bounds.
    {
        let window_ref = window.clone();
        let entry_ref = search_entry.clone();
        let list = list.clone();
        let state = state.clone();

        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = rx.recv().await {
                match event {
                    DaemonEvent::Show => {
                        window_ref.present();
                        entry_ref.grab_focus();
                    }
                    DaemonEvent::Hide => {
                        window_ref.set_visible(false);
                        entry_ref.set_text(""); // fires connect_changed
                    }
                    DaemonEvent::ReloadConfig {
                        config: new_cfg,
                        entries,
                    } => {
                        let new_css = load_user_css();
                        {
                            let mut s = state.borrow_mut();
                            s.searcher.reload(entries);
                            s.config = *new_cfg;
                            s.refresh_results();
                        }
                        // Re-apply geometry and placeholder, not just the CSS:
                        // window config used to be frozen at build time.
                        widgets.apply_config(&state.borrow().config);
                        provider.load_from_string(&new_css);
                        rebuild_list(&list, &state.borrow());
                    }
                    DaemonEvent::EntriesUpdated(entries) => {
                        // A rescan fires on any write to a watched directory, so
                        // it usually finds nothing new. Only touch the list when
                        // the visible results actually changed — rebuilding it
                        // unconditionally reset the selection under a user who
                        // was mid-navigation in an open window.
                        let changed = {
                            let mut s = state.borrow_mut();
                            s.searcher.reload(entries);
                            s.refresh_after_entry_change()
                        };
                        if changed {
                            rebuild_list(&list, &state.borrow());
                        }
                    }
                    DaemonEvent::Quit => {
                        app.quit();
                    }
                }
            }
        });
    }
}

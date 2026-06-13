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
use crate::daemon::{DaemonEvent, DaemonState};
use crate::plugins::{
    apps::AppsPlugin, commands::CommandsPlugin, math::MathPlugin, Entry as AppEntry, EntryKind,
    Plugin,
};
use crate::search::Searcher;

fn load_user_css() -> String {
    let path = crate::config::style_path();
    match std::fs::read_to_string(&path) {
        Ok(css) => css,
        Err(_) => {
            tracing::warn!("No stylesheet at {}; window will use system theme", path.display());
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
    daemon_state: Arc<DaemonState>,
}

impl UiState {
    fn refresh_results(&mut self) {
        if self.query.starts_with('=') {
            let mut math_out = Vec::new();
            MathPlugin.query(&self.query, &mut math_out);
            self.results = math_out;
        } else {
            self.searcher.update_pattern(&self.query);
            self.results = self
                .searcher
                .results(self.config.window.max_results)
                .into_iter()
                .map(|m| m.entry)
                .collect();
        }
        self.selected = 0;
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
            }
        }
    }
}

// ── Scroll helper ─────────────────────────────────────────────────────────────

fn scroll_to_row(scroll: &ScrolledWindow, row: &ListBoxRow) {
    let Some(list) = row.parent() else { return };
    let Some(bounds) = row.compute_bounds(&list) else { return };

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
    // Compatible with all GTK4 versions (remove_all needs 4.12)
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

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

pub fn run(
    daemon_state: Arc<DaemonState>,
    ipc_rx: mpsc::Receiver<DaemonEvent>,
) -> anyhow::Result<()> {
    gtk4::init().map_err(|e| anyhow::anyhow!("GTK init: {e}"))?;

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

    let app = Application::builder()
        .application_id("dev.slaunch")
        .build();

    // Cell lets us take the receiver exactly once inside the activate closure
    // without needing Rc (connect_activate doesn't require Send).
    let receiver = Cell::new(Some(receiver));

    app.connect_activate({
        let daemon_state = daemon_state.clone();
        move |app| {
            if let Some(rx) = receiver.take() {
                build_ui(app, rx, daemon_state.clone());
            }
        }
    });

    app.run_with_args::<String>(&[]);
    Ok(())
}

// ── UI construction ───────────────────────────────────────────────────────────

fn build_ui(
    app: &Application,
    rx: async_channel::Receiver<DaemonEvent>,
    daemon_state: Arc<DaemonState>,
) {
    // try_read() is non-blocking; by the time activate fires, no writer is active.
    let config = daemon_state.config.try_read().map(|g| g.clone()).unwrap_or_default();
    let entries = daemon_state.entries.try_read().map(|g| g.clone()).unwrap_or_default();
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
    let window = ApplicationWindow::builder()
        .application(&app)
        .default_width(config.window.width as i32)
        .build();
    window.add_css_class("slaunch");
    window.set_decorated(false);

    // Layer shell
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_namespace("slaunch");
    window.set_exclusive_zone(-1);

    match config.window.anchor.as_str() {
        "bottom" => {
            window.set_anchor(Edge::Bottom, true);
            window.set_anchor(Edge::Top, false);
            window.set_margin(Edge::Bottom, config.window.margin as i32);
        }
        "center" => {} // no anchors → compositor centres the window
        _ => {
            // "top" (default)
            window.set_anchor(Edge::Top, true);
            window.set_anchor(Edge::Bottom, false);
            window.set_margin(Edge::Top, config.window.margin as i32);
        }
    }

    // Widgets
    let search_entry = Entry::builder()
        .placeholder_text(&config.input.placeholder)
        .hexpand(true)
        .build();

    let list = ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);

    let max_list_height =
        (config.window.max_results * config.window.item_height as usize) as i32;
    let scroll = ScrolledWindow::builder()
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(max_list_height)
        .build();
    scroll.set_child(Some(&list));

    let p = config.window.padding as i32;
    let content = GtkBox::new(Orientation::Vertical, p);
    content.set_margin_start(p);
    content.set_margin_end(p);
    content.set_margin_top(p);
    content.set_margin_bottom(p);
    content.append(&search_entry);
    content.append(&scroll);
    window.set_child(Some(&content));

    // Initial search state
    let mut searcher = Searcher::new(entries);
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
        daemon_state,
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

            if key == K::Escape {
                entry_ref.set_text(""); // fires connect_changed → clears query + list
                window_ref.set_visible(false);
                return glib::Propagation::Stop;
            }

            if key == K::Return {
                {
                    let s = state.borrow();
                    s.launch_at(s.selected);
                }
                entry_ref.set_text(""); // fires connect_changed
                window_ref.set_visible(false);
                return glib::Propagation::Stop;
            }

            if key == K::Down || key == K::Tab {
                {
                    let mut s = state.borrow_mut();
                    if !s.results.is_empty() {
                        s.selected = (s.selected + 1).min(s.results.len() - 1);
                    }
                }
                let idx = state.borrow().selected as i32;
                list.select_row(list.row_at_index(idx).as_ref());
                if let Some(row) = list.row_at_index(idx) {
                    scroll_to_row(&scroll, &row);
                }
                return glib::Propagation::Stop;
            }

            if key == K::Up {
                {
                    let mut s = state.borrow_mut();
                    if s.selected > 0 {
                        s.selected -= 1;
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
            entry_ref.set_text(""); // fires connect_changed
            window_ref.set_visible(false);
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
                        entry_ref.set_text(""); // fires connect_changed
                        window_ref.set_visible(false);
                    }
                    DaemonEvent::ReloadConfig(new_cfg) => {
                        let new_entries = state
                            .borrow()
                            .daemon_state
                            .entries
                            .try_read()
                            .map(|g| g.clone())
                            .unwrap_or_default();
                        let new_css = load_user_css();
                        {
                            let mut s = state.borrow_mut();
                            s.searcher.reload(new_entries);
                            s.config = *new_cfg;
                            s.refresh_results();
                        }
                        provider.load_from_string(&new_css);
                        rebuild_list(&list, &state.borrow());
                    }
                    DaemonEvent::Quit => {
                        app.quit();
                    }
                }
            }
        });
    }
}

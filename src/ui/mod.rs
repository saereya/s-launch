pub mod theme;

use std::sync::Arc;

use iced::{
    futures::SinkExt,
    keyboard::{self, key::Named},
    widget::{button, column, container, scrollable, text, text_input},
    Element, Length, Subscription, Task,
};
use iced_layershell::{
    reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings},
    settings::{LayerShellSettings, Settings, StartMode},
    to_layer_message, MultiApplication,
};
use tokio::sync::mpsc;

use crate::{
    config::Config,
    daemon::{DaemonEvent, DaemonState},
    plugins::{apps::AppsPlugin, commands::CommandsPlugin, Entry, EntryKind, Plugin},
    search::Searcher,
};

use theme::Theme;

// ── Window info type (required by MultiApplication) ───────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct WindowInfo;

// ── Messages ──────────────────────────────────────────────────────────────────
//
// The #[to_layer_message(multi, info_name = "WindowInfo")] macro appends layer
// shell action variants: NewLayerShell{settings,info}, RemoveWindow(Id),
// AnchorChange{id,anchor}, SizeChange{id,size}, etc.

#[to_layer_message(multi, info_name = "WindowInfo")]
#[derive(Debug, Clone)]
pub enum Message {
    QueryChanged(String),
    SelectionUp,
    SelectionDown,
    Activate,
    Hide,
    IpcEvent(DaemonEvent),
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct LauncherApp {
    config: Config,
    theme: Theme,
    searcher: Searcher,
    query: String,
    results: Vec<Entry>,
    selected: usize,
    window_id: Option<iced::window::Id>,
    ipc_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<DaemonEvent>>>,
    daemon_state: Arc<DaemonState>,
}

impl LauncherApp {
    fn refresh_results(&mut self) {
        self.searcher.update_pattern(&self.query);
        self.results = self
            .searcher
            .results(self.config.window.max_results)
            .into_iter()
            .map(|m| m.entry)
            .collect();
        self.selected = 0;
    }

    fn launch_selected(&self) {
        if let Some(entry) = self.results.get(self.selected) {
            match &entry.kind {
                EntryKind::App { .. } => AppsPlugin.launch(entry),
                EntryKind::Command { .. } => CommandsPlugin.launch(entry),
            }
        }
    }

    fn make_show_task(&mut self) -> Task<Message> {
        if self.window_id.is_some() {
            return Task::none();
        }
        let cfg = &self.config;
        let anchor = match cfg.window.anchor.as_str() {
            "bottom" => Anchor::Bottom | Anchor::Left | Anchor::Right,
            "center" => Anchor::empty(),
            _ => Anchor::Top | Anchor::Left | Anchor::Right,
        };

        let row_h = cfg.style.item_height as u32 + 4;
        let n = self.results.len().min(cfg.window.max_results) as u32;
        let height = 52 + n * row_h + cfg.style.padding as u32 * 2 + 8;

        let margin_top = cfg.window.margin as i32;
        let settings = NewLayerShellSettings {
            size: Some((cfg.window.width, height)),
            anchor,
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            margin: Some((margin_top, 0, margin_top, 0)),
            exclusive_zone: None,
            use_last_output: false,
            events_transparent: false,
        };

        Task::done(Message::NewLayerShell {
            settings,
            info: WindowInfo,
        })
    }

    fn make_hide_task(&mut self) -> Task<Message> {
        if let Some(id) = self.window_id.take() {
            self.query.clear();
            self.refresh_results();
            Task::done(Message::RemoveWindow(id))
        } else {
            Task::none()
        }
    }
}

// ── MultiApplication impl ─────────────────────────────────────────────────────

impl MultiApplication for LauncherApp {
    type Message = Message;
    type Theme = iced::Theme;
    type Executor = iced::executor::Default;
    type Flags = (Arc<DaemonState>, mpsc::Receiver<DaemonEvent>);
    type WindowInfo = WindowInfo;

    fn new((daemon_state, ipc_rx): Self::Flags) -> (Self, Task<Self::Message>) {
        let config = daemon_state
            .config
            .try_read()
            .map(|g| g.clone())
            .unwrap_or_default();
        let entries = daemon_state
            .entries
            .try_read()
            .map(|g| g.clone())
            .unwrap_or_default();

        let theme = Theme::from_style(&config.style);
        let mut searcher = Searcher::new(entries);
        searcher.update_pattern("");
        let results = searcher
            .results(config.window.max_results)
            .into_iter()
            .map(|m| m.entry)
            .collect();

        let app = LauncherApp {
            config,
            theme,
            searcher,
            query: String::new(),
            results,
            selected: 0,
            window_id: None,
            ipc_rx: Arc::new(tokio::sync::Mutex::new(ipc_rx)),
            daemon_state,
        };
        (app, Task::none())
    }

    fn namespace(&self) -> String {
        "slaunch".into()
    }

    fn id_info(&self, _id: iced::window::Id) -> Option<Self::WindowInfo> {
        Some(WindowInfo)
    }

    fn set_id_info(&mut self, id: iced::window::Id, _info: Self::WindowInfo) {
        self.window_id = Some(id);
    }

    fn remove_id(&mut self, _id: iced::window::Id) {
        self.window_id = None;
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::QueryChanged(q) => {
                self.query = q;
                self.refresh_results();
                Task::none()
            }

            Message::SelectionDown => {
                if !self.results.is_empty() {
                    self.selected = (self.selected + 1).min(self.results.len() - 1);
                }
                Task::none()
            }

            Message::SelectionUp => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                Task::none()
            }

            Message::Activate => {
                self.launch_selected();
                self.make_hide_task()
            }

            Message::Hide => self.make_hide_task(),

            Message::IpcEvent(event) => match event {
                DaemonEvent::Show => self.make_show_task(),

                DaemonEvent::Hide => self.make_hide_task(),

                DaemonEvent::ReloadConfig(new_cfg) => {
                    self.theme = Theme::from_style(&new_cfg.style);
                    let entries = self
                        .daemon_state
                        .entries
                        .try_read()
                        .map(|g| g.clone())
                        .unwrap_or_default();
                    self.searcher.reload(entries);
                    self.config = *new_cfg;
                    self.refresh_results();
                    Task::none()
                }

                DaemonEvent::Quit => iced::exit(),
            },

            // All layer shell action variants generated by the macro are
            // handled as TryInto conversions by the runtime — we don't match
            // them here, but we still need to cover the pattern to avoid
            // an exhaustive match warning. The `_` arm silences that.
            _ => Task::none(),
        }
    }

    fn view(&self, _window_id: iced::window::Id) -> Element<'_, Self::Message> {
        let t = &self.theme;
        let p = t.padding as u16;

        // ── Search input ──────────────────────────────────────────────────
        let input = text_input(&self.config.input.placeholder, &self.query)
            .on_input(Message::QueryChanged)
            .on_submit(Message::Activate)
            .size(t.font_size)
            .padding(p)
            .style(t.input_style());

        // ── Result rows ───────────────────────────────────────────────────
        let rows: Vec<Element<_>> = self
            .results
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let selected = i == self.selected;

                let name_line = text(&entry.name).size(t.font_size);
                let desc_str = entry.description.as_deref().unwrap_or("").to_string();
                let desc_line = text(desc_str).size(t.font_size * 0.8).color(iced::Color {
                    a: 0.6,
                    ..t.item_foreground
                });

                let content = column![name_line, desc_line].spacing(2);

                button(content)
                    .on_press(Message::Activate)
                    .width(Length::Fill)
                    .height(t.item_height)
                    .padding([4, p])
                    .style(t.item_style(selected))
                    .into()
            })
            .collect();

        let list = scrollable(column(rows).spacing(2).width(Length::Fill)).width(Length::Fill);

        // ── Outer window container ────────────────────────────────────────
        let win_style = t.window_container_style();
        container(column![input, list].spacing(p).padding(p))
            .width(Length::Fill)
            .style(move |_theme: &iced::Theme| win_style)
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let ipc_rx = self.ipc_rx.clone();

        let ipc_sub = Subscription::run_with_id(
            "ipc-events",
            iced::stream::channel(16, move |mut output| {
                let ipc_rx = ipc_rx.clone();
                async move {
                    let mut rx = ipc_rx.lock().await;
                    loop {
                        match rx.recv().await {
                            Some(event) => {
                                let _ = output.send(Message::IpcEvent(event)).await;
                            }
                            None => {
                                let _ = output
                                    .send(Message::IpcEvent(DaemonEvent::Quit))
                                    .await;
                                return;
                            }
                        }
                    }
                }
            }),
        );

        let keyboard_sub = keyboard::on_key_press(|key, _modifiers| match key {
            keyboard::Key::Named(Named::Escape) => Some(Message::Hide),
            keyboard::Key::Named(Named::ArrowDown) | keyboard::Key::Named(Named::Tab) => {
                Some(Message::SelectionDown)
            }
            keyboard::Key::Named(Named::ArrowUp) => Some(Message::SelectionUp),
            keyboard::Key::Named(Named::Enter) => Some(Message::Activate),
            _ => None,
        });

        Subscription::batch([ipc_sub, keyboard_sub])
    }
}

// ── Public launcher ───────────────────────────────────────────────────────────

pub fn run(
    daemon_state: Arc<DaemonState>,
    ipc_rx: mpsc::Receiver<DaemonEvent>,
) -> anyhow::Result<()> {
    let layer_settings = LayerShellSettings {
        start_mode: StartMode::Background,
        ..Default::default()
    };
    LauncherApp::run(Settings {
        flags: (daemon_state, ipc_rx),
        layer_settings,
        id: None,
        fonts: Vec::new(),
        default_font: iced::Font::default(),
        default_text_size: iced::Pixels(16.0),
        antialiasing: false,
        virtual_keyboard_support: None,
    })
    .map_err(|e| anyhow::anyhow!("Iced error: {e}"))
}

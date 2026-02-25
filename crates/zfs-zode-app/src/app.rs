use std::sync::Arc;

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use zfs_zode::{LogEvent, Zode};

use crate::components::title_bar_icon;
use crate::settings::Settings;
use crate::state::{AppState, Tab, MAX_LOG_ENTRIES};

pub(crate) struct ZodeApp {
    pub rt: Runtime,
    pub settings: Settings,
    pub zode: Option<Arc<Zode>>,
    pub shared: Arc<Mutex<AppState>>,
    pub tab: Tab,
    pub settings_error: Option<String>,
    pub shutdown_tx: Option<tokio::sync::mpsc::Sender<()>>,
    pub poller_handle: Option<tokio::task::JoinHandle<()>>,
    pub chat_state: Option<crate::state::ChatState>,
    pub identity_state: crate::state::IdentityState,
    pub visualization: crate::visualization::NetworkVisualization,
    icon_texture: Option<egui::TextureHandle>,
}

impl ZodeApp {
    pub fn new(rt: Runtime) -> Self {
        let mut app = Self {
            rt,
            settings: Settings::default(),
            zode: None,
            shared: Arc::new(Mutex::new(AppState::default())),
            tab: Tab::Status,
            settings_error: None,
            shutdown_tx: None,
            poller_handle: None,
            chat_state: None,
            identity_state: Default::default(),
            visualization: Default::default(),
            icon_texture: None,
        };
        app.boot_zode();
        app
    }

    fn icon_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.icon_texture
            .get_or_insert_with(|| {
                let png = include_bytes!("../assets/icon.png");
                let img = image::load_from_memory(png).expect("bad icon png");
                let rgba = img.to_rgba8();
                let pixels = rgba.as_flat_samples();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    pixels.as_slice(),
                );
                ctx.load_texture("app_icon", color_image, egui::TextureOptions::LINEAR)
            })
            .clone()
    }

    pub fn boot_zode(&mut self) {
        let config = match self.settings.build_config() {
            Ok(c) => c,
            Err(e) => {
                self.settings_error = Some(e);
                return;
            }
        };
        self.settings_error = None;
        self.stop_zode();

        let shared = Arc::new(Mutex::new(AppState::default()));
        self.shared = Arc::clone(&shared);

        let start_result = self.rt.block_on(async { Zode::start(config).await });
        match start_result {
            Ok(zode) => {
                let zode = Arc::new(zode);
                self.zode = Some(Arc::clone(&zode));
                let (stop_tx, stop_rx) = tokio::sync::mpsc::channel::<()>(1);
                self.shutdown_tx = Some(stop_tx);
                self.poller_handle =
                    Some(Self::spawn_status_poller(&self.rt, &zode, &shared, stop_rx));
                Self::spawn_log_listener(&self.rt, &zode, &shared);
            }
            Err(e) => {
                self.settings_error = Some(format!("Start failed: {e}"));
            }
        }
    }

    fn spawn_status_poller(
        rt: &Runtime,
        zode: &Arc<Zode>,
        shared: &Arc<Mutex<AppState>>,
        mut stop_rx: tokio::sync::mpsc::Receiver<()>,
    ) -> tokio::task::JoinHandle<()> {
        let bg_zode = Arc::clone(zode);
        let bg_shared = Arc::clone(shared);
        rt.spawn(async move {
            loop {
                tokio::select! {
                    _ = stop_rx.recv() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
                }
                let status = bg_zode.status();
                bg_shared.lock().await.status = Some(status);
            }
        })
    }

    fn spawn_log_listener(rt: &Runtime, zode: &Arc<Zode>, shared: &Arc<Mutex<AppState>>) {
        let log_shared = Arc::clone(shared);
        let mut event_rx = zode.subscribe_events();
        rt.spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        let line = event.to_string();
                        let mut state = log_shared.lock().await;
                        if let LogEvent::Started { ref listen_addr } = event {
                            state.listen_addr = Some(listen_addr.clone());
                        }
                        match &event {
                            LogEvent::PeerConnected(id) => {
                                state
                                    .peer_events
                                    .push_back(crate::state::PeerEvent::Connected(id.clone()));
                            }
                            LogEvent::PeerDisconnected(id) => {
                                state
                                    .peer_events
                                    .push_back(crate::state::PeerEvent::Disconnected(id.clone()));
                            }
                            LogEvent::PeerDiscovered(id) => {
                                state
                                    .peer_events
                                    .push_back(crate::state::PeerEvent::Discovered(id.clone()));
                            }
                            _ => {}
                        }
                        if state.log_entries.len() >= MAX_LOG_ENTRIES {
                            state.log_entries.pop_front();
                        }
                        state.log_entries.push_back(line);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        let mut state = log_shared.lock().await;
                        state
                            .log_entries
                            .push_back(format!("[WARN] lagged {n} events"));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub fn stop_zode(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.try_send(());
        }
        if let Some(ref zode) = self.zode {
            self.rt.block_on(zode.shutdown());
        }
        if let Some(handle) = self.poller_handle.take() {
            let _ = self.rt.block_on(handle);
        }
        self.zode = None;
    }

    fn handle_resize_edges(ctx: &egui::Context) -> bool {
        const BORDER: f32 = 6.0;
        let screen = ctx.screen_rect();
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return false;
        };

        let left = pos.x - screen.left() < BORDER;
        let right = screen.right() - pos.x < BORDER;
        let top = pos.y - screen.top() < BORDER;
        let bottom = screen.bottom() - pos.y < BORDER;

        use egui::viewport::ResizeDirection;
        let dir = match (left, right, top, bottom) {
            (true, _, true, _) => Some(ResizeDirection::NorthWest),
            (_, true, true, _) => Some(ResizeDirection::NorthEast),
            (true, _, _, true) => Some(ResizeDirection::SouthWest),
            (_, true, _, true) => Some(ResizeDirection::SouthEast),
            (true, _, _, _) => Some(ResizeDirection::West),
            (_, true, _, _) => Some(ResizeDirection::East),
            (_, _, true, _) => Some(ResizeDirection::North),
            (_, _, _, true) => Some(ResizeDirection::South),
            _ => None,
        };

        let Some(dir) = dir else { return false };

        let cursor = match dir {
            ResizeDirection::North | ResizeDirection::South => egui::CursorIcon::ResizeVertical,
            ResizeDirection::East | ResizeDirection::West => egui::CursorIcon::ResizeHorizontal,
            ResizeDirection::NorthWest | ResizeDirection::SouthEast => egui::CursorIcon::ResizeNwSe,
            ResizeDirection::NorthEast | ResizeDirection::SouthWest => egui::CursorIcon::ResizeNeSw,
        };
        ctx.set_cursor_icon(cursor);

        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }

        true
    }
}


impl ZodeApp {
    fn sync_visualization(&mut self, state: &crate::state::StateSnapshot) {
        for event in &state.peer_events {
            match event {
                crate::state::PeerEvent::Connected(id) => {
                    self.visualization.on_peer_connected(id);
                }
                crate::state::PeerEvent::Disconnected(id) => {
                    self.visualization.on_peer_disconnected(id);
                }
                crate::state::PeerEvent::Discovered(id) => {
                    self.visualization.on_peer_discovered(id);
                }
            }
        }
        if let Some(ref status) = state.status {
            self.visualization
                .reconcile(&status.zode_id, &status.connected_peers);
        }
    }

    fn render_title_bar(
        &mut self,
        ctx: &egui::Context,
        maximized: bool,
        on_resize_edge: bool,
    ) {
        egui::TopBottomPanel::top("tabs")
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::BLACK)
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                    .stroke(egui::Stroke::NONE),
            )
            .show(ctx, |ui| {
                let title_bar_rect = ui.max_rect();
                let title_resp = ui.interact(
                    title_bar_rect,
                    egui::Id::new("title_bar"),
                    egui::Sense::click_and_drag(),
                );
                if !on_resize_edge && title_resp.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if title_resp.double_clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }

                ui.visuals_mut().widgets.active = ui.visuals().widgets.hovered;
                ui.visuals_mut().selection.bg_fill = egui::Color32::TRANSPARENT;
                ui.visuals_mut().selection.stroke =
                    egui::Stroke::new(1.0, egui::Color32::WHITE);
                ui.visuals_mut().widgets.active.fg_stroke =
                    egui::Stroke::new(1.0, egui::Color32::WHITE);

                ui.horizontal(|ui| {
                    self.render_tab_buttons(ui);

                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| self.render_window_controls(ui),
                    );
                });

                Self::handle_title_bar_drag(ui, &title_resp, title_bar_rect, on_resize_edge);
            });
    }

    fn render_tab_buttons(&mut self, ui: &mut egui::Ui) {
        let tex = self.icon_texture(ui.ctx());
        ui.add(
            egui::Image::new(&tex)
                .fit_to_exact_size(egui::vec2(20.0, 20.0))
                .rounding(3.0),
        );
        ui.add_space(4.0);
        ui.selectable_value(&mut self.tab, Tab::Status, "ZODE");
        ui.selectable_value(&mut self.tab, Tab::Storage, "STORAGE");
        ui.selectable_value(&mut self.tab, Tab::Peers, "PEERS");
        ui.selectable_value(&mut self.tab, Tab::Log, "LOG");
        ui.selectable_value(&mut self.tab, Tab::Chat, "INTERLINK");
        ui.selectable_value(&mut self.tab, Tab::Info, "INFO");
    }

    fn render_window_controls(&mut self, ui: &mut egui::Ui) {
        if title_bar_icon(ui, egui_phosphor::regular::X, false).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        let max_icon = if maximized {
            egui_phosphor::regular::CORNERS_IN
        } else {
            egui_phosphor::regular::CORNERS_OUT
        };
        if title_bar_icon(ui, max_icon, false).clicked() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }

        if title_bar_icon(ui, egui_phosphor::regular::MINUS, false).clicked() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }

        ui.add_space(4.0);

        let is_settings = self.tab == Tab::Settings;
        if title_bar_icon(ui, egui_phosphor::regular::GEAR_SIX, is_settings).clicked() {
            self.tab = Tab::Settings;
        }

        let is_identity = self.tab == Tab::Identity;
        if title_bar_icon(ui, egui_phosphor::regular::USER_CIRCLE, is_identity).clicked() {
            self.tab = Tab::Identity;
        }

        let connected = self.zode.is_some();
        let dot_color = if connected {
            crate::components::colors::CONNECTED
        } else {
            crate::components::colors::DISCONNECTED
        };
        let status_label = if connected { "connected" } else { "stopped" };
        ui.monospace(egui::RichText::new(status_label).color(dot_color));
        let dot_radius = 3.5;
        let (dot_rect, _) = ui.allocate_exact_size(
            egui::vec2(dot_radius * 2.0 + 2.0, dot_radius * 2.0),
            egui::Sense::hover(),
        );
        ui.painter()
            .circle_filled(dot_rect.center(), dot_radius, dot_color);
    }

    /// Drag from any point in the title bar (including over buttons) to move
    /// the window. Raw pointer state bypasses widget hit-testing so a press
    /// that started on a tab button still initiates a drag once the pointer
    /// moves past a small threshold.
    fn handle_title_bar_drag(
        ui: &egui::Ui,
        title_resp: &egui::Response,
        title_bar_rect: egui::Rect,
        on_resize_edge: bool,
    ) {
        if on_resize_edge || title_resp.double_clicked() {
            return;
        }
        let drag = ui.input(|i| match (i.pointer.press_origin(), i.pointer.hover_pos()) {
            (Some(origin), Some(current)) => Some((origin, current)),
            _ => None,
        });
        if let Some((press_origin, current)) = drag {
            if title_bar_rect.contains(press_origin) && press_origin.distance(current) > 4.0 {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }
    }

    fn render_central_panel(
        &mut self,
        ctx: &egui::Context,
        state: &crate::state::StateSnapshot,
    ) {
        let central_frame = egui::Frame::default()
            .fill(egui::Color32::BLACK)
            .inner_margin(8.0);

        egui::CentralPanel::default()
            .frame(central_frame)
            .show(ctx, |ui| {
                if self.tab != Tab::Settings && self.tab != Tab::Identity && self.zode.is_none() {
                    let rect = ui.max_rect();
                    ui.vertical_centered(|ui| {
                        ui.add_space((rect.height() / 2.0 - 25.0).max(0.0));
                        ui.spinner();
                        ui.add_space(4.0);
                        ui.label("Zode is stopped. Go to Settings to start.");
                    });
                    return;
                }
                match self.tab {
                    Tab::Status => crate::render::render_status(self, ui, state),
                    Tab::Storage => crate::render_storage::render_storage(self, ui, state),
                    Tab::Peers => crate::render::render_peers(self, ui, state),
                    Tab::Log => crate::render::render_log(ui, state),
                    Tab::Chat => crate::chat::render_chat(self, ui),
                    Tab::Info => crate::render::render_info(self, ui, state),
                    Tab::Settings => crate::render::render_settings(self, ui),
                    Tab::Identity => crate::identity::render_identity(self, ui),
                }
            });
    }

    fn render_window_border(ctx: &egui::Context, maximized: bool) {
        if !maximized {
            let fg = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("window_border"),
            ));
            fg.rect_stroke(
                ctx.screen_rect(),
                0.0,
                egui::Stroke::new(1.0, crate::components::colors::BORDER),
            );
        }
    }
}

impl eframe::App for ZodeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let state = self
            .rt
            .block_on(async { self.shared.lock().await.snapshot() });

        self.sync_visualization(&state);

        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let on_resize_edge = if !maximized {
            Self::handle_resize_edges(ctx)
        } else {
            false
        };

        self.render_title_bar(ctx, maximized, on_resize_edge);
        self.render_central_panel(ctx, &state);
        Self::render_window_border(ctx, maximized);
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

use std::sync::Arc;

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use zode::{LogEvent, Zode};

use crate::components::title_bar_icon;
use crate::profile::{self, ProfileMeta};
use crate::settings::Settings;
use crate::state::{AppPhase, AppState, Tab, MAX_LOG_ENTRIES};

pub(crate) struct ZodeApp {
    pub rt: Runtime,
    pub settings: Settings,
    pub zode: Option<Arc<Zode>>,
    pub shared: Arc<Mutex<AppState>>,
    pub tab: Tab,
    pub prev_tab: Tab,
    pub settings_error: Option<String>,
    pub shutdown_tx: Option<tokio::sync::mpsc::Sender<()>>,
    pub poller_handle: Option<tokio::task::JoinHandle<()>>,
    pub peer_persist_tx: Option<tokio::sync::mpsc::Sender<()>>,
    pub peer_persist_handle: Option<tokio::task::JoinHandle<()>>,
    pub interlink_state: Option<crate::state::InterlinkState>,
    pub identity_state: crate::state::IdentityState,
    pub visualization: crate::visualization::NetworkVisualization,
    icon_texture: Option<egui::TextureHandle>,
    pub phase: AppPhase,
    pub profiles: Vec<ProfileMeta>,
    pub unlock_password: String,
    pub unlock_error: Option<String>,
    pub confirm_delete_profile: Option<String>,
    pub reveal_start: Option<f64>,
    pub status_first_seen: Option<f64>,
    pub log_scroll_to_bottom: bool,
    pub active_profile_id: Option<String>,
    pub session_password: Option<String>,
}

impl ZodeApp {
    pub fn new(rt: Runtime) -> Self {
        let base = profile::base_dir();
        let profiles = profile::list_profiles(&base);

        let phase = if profiles.is_empty() {
            AppPhase::Setup
        } else if profiles.len() == 1 {
            AppPhase::Unlock {
                profile_id: profiles[0].id.clone(),
            }
        } else {
            AppPhase::ProfileSelect
        };

        let settings = Settings::default();

        let app = Self {
            rt,
            settings,
            zode: None,
            shared: Arc::new(Mutex::new(AppState::default())),
            tab: Tab::Status,
            prev_tab: Tab::Status,
            settings_error: None,
            shutdown_tx: None,
            poller_handle: None,
            peer_persist_tx: None,
            peer_persist_handle: None,
            interlink_state: None,
            identity_state: Default::default(),
            visualization: Default::default(),
            icon_texture: None,
            phase: phase.clone(),
            profiles,
            unlock_password: String::new(),
            unlock_error: None,
            confirm_delete_profile: None,
            reveal_start: None,
            status_first_seen: None,
            log_scroll_to_bottom: false,
            active_profile_id: None,
            session_password: None,
        };

        app
    }

    /// Returns the path where settings should be persisted for the current
    /// session (per-profile if a profile is active, global otherwise).
    fn settings_file_path(&self) -> std::path::PathBuf {
        let base = profile::base_dir();
        if let Some(ref id) = self.active_profile_id {
            profile::settings_path_for_profile(&base, id)
        } else {
            profile::global_settings_path(&base)
        }
    }

    /// Path where the peer cache is stored (next to the settings file).
    fn peer_cache_path(&self) -> std::path::PathBuf {
        self.settings_file_path().with_file_name("peer_cache.json")
    }

    /// Merge previously cached peers into a network config's bootstrap list.
    /// Sanitizes historical addresses to remove duplicate `/p2p/` segments.
    fn merge_peer_cache(&self, config: &mut zode::ZodeConfig) {
        let path = self.peer_cache_path();
        let cached = crate::settings::load_peer_cache(&path);
        let total = cached.len();
        let mut parsed = 0usize;
        let mut failed = 0usize;
        for s in &cached {
            let stripped = grid_net::strip_zx_multiaddr(s);
            match stripped.parse::<grid_net::Multiaddr>() {
                Ok(addr) => {
                    let sanitized = grid_net::sanitize_dial_addr(&addr);
                    parsed += 1;
                    if !config.network.bootstrap_peers.contains(&sanitized) {
                        config.network.bootstrap_peers.push(sanitized);
                    }
                }
                Err(e) => {
                    failed += 1;
                    tracing::warn!(
                        raw = %s,
                        error = %e,
                        "peer cache entry failed to parse"
                    );
                }
            }
        }
        tracing::info!(
            path = %path.display(),
            total,
            parsed,
            failed,
            bootstrap_total = config.network.bootstrap_peers.len(),
            "peer cache merged"
        );
    }

    /// Persist current settings to disk.
    pub(crate) fn save_settings(&self) {
        let path = self.settings_file_path();
        if let Err(e) = self.settings.save_to(&path) {
            tracing::warn!("failed to save settings: {e}");
        }
    }

    pub(crate) fn icon_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
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

    pub(crate) fn attempt_unlock(&mut self, profile_id: &str) {
        let base = profile::base_dir();
        match profile::unlock_profile(&base, profile_id, &self.unlock_password) {
            Ok(plaintext) => {
                self.unlock_error = None;
                self.active_profile_id = Some(profile_id.to_string());
                self.session_password = Some(self.unlock_password.clone());
                self.unlock_password.clear();

                let settings_path = profile::settings_path_for_profile(&base, profile_id);
                self.settings = Settings::load_from(&settings_path);

                let shares: Vec<zid::ShamirShare> = plaintext
                    .shares
                    .iter()
                    .filter_map(|h| zid::ShamirShare::from_hex(h).ok())
                    .collect();
                self.identity_state.shares = shares;
                self.identity_state.identity_id = plaintext.identity_id;

                let libp2p_keypair =
                    grid_net::Keypair::from_protobuf_encoding(&plaintext.libp2p_keypair).ok();

                let caps = zid::MachineKeyCapabilities::from_bits_truncate(plaintext.capabilities);
                let mk_result = std::thread::Builder::new()
                    .name("vault-derive".into())
                    .stack_size(8 * 1024 * 1024)
                    .spawn({
                        let shares = self.identity_state.shares.clone();
                        let identity_id = plaintext.identity_id;
                        let machine_id = plaintext.machine_id;
                        let epoch = plaintext.epoch;
                        move || {
                            zid::derive_machine_keypair_from_shares(
                                &shares,
                                zid::IdentityId::new(identity_id),
                                zid::MachineId::new(machine_id),
                                epoch,
                                caps,
                            )
                        }
                    })
                    .expect("spawn derive thread")
                    .join()
                    .expect("derive thread panicked");

                if let Ok(kp) = mk_result {
                    let pk = kp.public_key();
                    let did = zid::ed25519_to_did_key(&pk.ed25519_bytes());
                    self.identity_state.did = Some(did.clone());
                    self.identity_state
                        .machine_keys
                        .push(crate::state::DerivedMachineKey {
                            machine_id: plaintext.machine_id,
                            epoch: plaintext.epoch,
                            capabilities: caps,
                            did,
                            public_key: pk,
                            keypair: std::sync::Arc::new(kp),
                        });
                }

                let data_dir = profile::data_dir_for_profile(&base, profile_id);
                self.settings.data_dir = data_dir.to_string_lossy().to_string();

                if let Some(kp) = libp2p_keypair {
                    self.boot_zode_with_keypair(Some(kp));
                } else {
                    self.boot_zode();
                }
                self.phase = AppPhase::Revealing;
                self.reveal_start = None;
            }
            Err(e) => {
                self.unlock_error = Some(e.to_string());
                self.phase = AppPhase::Unlock {
                    profile_id: profile_id.to_string(),
                };
            }
        }
    }

    pub(crate) fn boot_zode_with_keypair(&mut self, keypair: Option<grid_net::Keypair>) {
        let config = match self.settings.build_config() {
            Ok(mut c) => {
                if let Some(kp) = keypair {
                    c.network = c.network.with_keypair(kp);
                }
                self.merge_peer_cache(&mut c);
                c
            }
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
                self.settings.data_dir = zode.data_dir().to_string_lossy().to_string();
                let zode = Arc::new(zode);
                self.zode = Some(Arc::clone(&zode));
                let (stop_tx, stop_rx) = tokio::sync::mpsc::channel::<()>(1);
                self.shutdown_tx = Some(stop_tx);
                self.poller_handle =
                    Some(Self::spawn_status_poller(&self.rt, &zode, &shared, stop_rx));
                Self::spawn_log_listener(&self.rt, &zode, &shared);

                let (persist_tx, persist_rx) = tokio::sync::mpsc::channel::<()>(1);
                self.peer_persist_tx = Some(persist_tx);
                self.peer_persist_handle = Some(Self::spawn_peer_persister(
                    &self.rt,
                    &zode,
                    self.peer_cache_path(),
                    persist_rx,
                ));
            }
            Err(e) => {
                self.settings_error = Some(format!("Start failed: {e}"));
            }
        }
    }

    pub fn boot_zode(&mut self) {
        let config = match self.settings.build_config() {
            Ok(mut c) => {
                self.merge_peer_cache(&mut c);
                c
            }
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
                self.settings.data_dir = zode.data_dir().to_string_lossy().to_string();
                let zode = Arc::new(zode);
                self.zode = Some(Arc::clone(&zode));
                let (stop_tx, stop_rx) = tokio::sync::mpsc::channel::<()>(1);
                self.shutdown_tx = Some(stop_tx);
                self.poller_handle =
                    Some(Self::spawn_status_poller(&self.rt, &zode, &shared, stop_rx));
                Self::spawn_log_listener(&self.rt, &zode, &shared);

                let (persist_tx, persist_rx) = tokio::sync::mpsc::channel::<()>(1);
                self.peer_persist_tx = Some(persist_tx);
                self.peer_persist_handle = Some(Self::spawn_peer_persister(
                    &self.rt,
                    &zode,
                    self.peer_cache_path(),
                    persist_rx,
                ));
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

    fn spawn_peer_persister(
        rt: &Runtime,
        zode: &Arc<Zode>,
        cache_path: std::path::PathBuf,
        mut stop_rx: tokio::sync::mpsc::Receiver<()>,
    ) -> tokio::task::JoinHandle<()> {
        let bg_zode = Arc::clone(zode);
        rt.spawn(async move {
            const PERSIST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
            loop {
                tokio::select! {
                    _ = stop_rx.recv() => return,
                    _ = tokio::time::sleep(PERSIST_INTERVAL) => {}
                }
                let addrs = bg_zode.peer_multiaddrs().await;
                if addrs.is_empty() {
                    tracing::debug!("peer cache tick: no peers to persist");
                    continue;
                }
                let sample: Vec<_> = addrs.iter().take(5).collect();
                tracing::info!(
                    count = addrs.len(),
                    sample = ?sample,
                    path = %cache_path.display(),
                    "persisting peer cache"
                );
                if let Err(e) = crate::settings::save_peer_cache(&cache_path, &addrs) {
                    tracing::warn!("failed to persist peer cache: {e}");
                }
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
        if let Some(tx) = self.peer_persist_tx.take() {
            let _ = tx.try_send(());
        }
        if let Some(ref zode) = self.zode {
            self.rt.block_on(zode.shutdown());
        }
        if let Some(handle) = self.poller_handle.take() {
            let _ = self.rt.block_on(handle);
        }
        if let Some(handle) = self.peer_persist_handle.take() {
            let _ = self.rt.block_on(handle);
        }

        if let Some(ref zode) = self.zode {
            let addrs = self.rt.block_on(zode.peer_multiaddrs());
            let sample: Vec<_> = addrs.iter().take(5).collect();
            tracing::info!(
                count = addrs.len(),
                sample = ?sample,
                "shutdown: saving peer cache"
            );
            self.settings.remember_peers(&addrs);
            if let Err(e) = crate::settings::save_peer_cache(&self.peer_cache_path(), &addrs) {
                tracing::warn!("failed to persist peer cache on shutdown: {e}");
            }
        }
        self.save_settings();

        self.zode = None;
        self.interlink_state = None;
    }

    pub(crate) fn do_delete_profile(&mut self, profile_id: &str) {
        let base = profile::base_dir();
        if let Err(e) = profile::delete_profile(&base, profile_id) {
            self.unlock_error = Some(e.to_string());
            return;
        }
        self.profiles.retain(|p| p.id != profile_id);
        self.confirm_delete_profile = None;
        self.unlock_password.clear();
        self.unlock_error = None;

        self.phase = if self.profiles.is_empty() {
            self.identity_state = Default::default();
            AppPhase::Setup
        } else if self.profiles.len() == 1 {
            AppPhase::Unlock {
                profile_id: self.profiles[0].id.clone(),
            }
        } else {
            AppPhase::ProfileSelect
        };
    }

    pub(crate) fn lock_session(&mut self) {
        self.stop_zode();
        self.settings = Settings::default();
        self.unlock_password.clear();
        self.unlock_error = None;
        self.confirm_delete_profile = None;
        self.active_profile_id = None;
        self.session_password = None;
        self.status_first_seen = None;
        self.identity_state = Default::default();
        self.shared =
            std::sync::Arc::new(tokio::sync::Mutex::new(crate::state::AppState::default()));
        self.tab = Tab::Status;

        self.phase = if self.profiles.is_empty() {
            self.identity_state = Default::default();
            AppPhase::Setup
        } else if self.profiles.len() > 1 {
            AppPhase::ProfileSelect
        } else {
            AppPhase::Unlock {
                profile_id: self.profiles[0].id.clone(),
            }
        };
    }

    fn handle_resize_edges(ctx: &egui::Context) -> bool {
        const BORDER: f32 = 6.0;
        let screen = ctx.viewport_rect();
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

    fn render_title_bar(&mut self, ctx: &egui::Context, maximized: bool, on_resize_edge: bool) {
        egui::TopBottomPanel::top("tabs")
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::BLACK)
                    .inner_margin(egui::Margin::symmetric(12, 8))
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
                ui.visuals_mut().selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
                ui.visuals_mut().widgets.active.fg_stroke =
                    egui::Stroke::new(1.0, egui::Color32::WHITE);

                ui.horizontal(|ui| {
                    self.render_tab_buttons(ui);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.render_window_controls(ui)
                    });
                });

                Self::handle_title_bar_drag(ui, &title_resp, title_bar_rect, on_resize_edge);
            });
    }

    fn render_tab_buttons(&mut self, ui: &mut egui::Ui) {
        let tex = self.icon_texture(ui.ctx());
        ui.add(
            egui::Image::new(&tex)
                .fit_to_exact_size(egui::vec2(20.0, 20.0))
                .corner_radius(3.0),
        );
        ui.add_space(4.0);
        ui.selectable_value(&mut self.tab, Tab::Status, "ZODE");
        ui.selectable_value(&mut self.tab, Tab::Storage, "STORAGE");
        ui.selectable_value(&mut self.tab, Tab::Peers, "ZODES");
        ui.selectable_value(&mut self.tab, Tab::Log, "LOG");
        ui.selectable_value(&mut self.tab, Tab::Interlink, "INTERLINK");
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

        if !self.profiles.is_empty() {
            if title_bar_icon(ui, egui_phosphor::regular::LOCK, false)
                .on_hover_text("Lock & switch profile")
                .clicked()
            {
                self.lock_session();
            }
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
        let drag = ui.input(
            |i| match (i.pointer.press_origin(), i.pointer.hover_pos()) {
                (Some(origin), Some(current)) => Some((origin, current)),
                _ => None,
            },
        );
        if let Some((press_origin, current)) = drag {
            if title_bar_rect.contains(press_origin) && press_origin.distance(current) > 4.0 {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }
    }

    fn render_pre_auth_title_bar(
        &mut self,
        ctx: &egui::Context,
        maximized: bool,
        on_resize_edge: bool,
    ) {
        egui::TopBottomPanel::top("pre_auth_title")
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::BLACK)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .stroke(egui::Stroke::NONE),
            )
            .show(ctx, |ui| {
                let title_bar_rect = ui.max_rect();
                let title_resp = ui.interact(
                    title_bar_rect,
                    egui::Id::new("pre_auth_title_bar"),
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
                ui.visuals_mut().selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
                ui.visuals_mut().widgets.active.fg_stroke =
                    egui::Stroke::new(1.0, egui::Color32::WHITE);

                ui.horizontal(|ui| {
                    let tex = self.icon_texture(ui.ctx());
                    ui.add(
                        egui::Image::new(&tex)
                            .fit_to_exact_size(egui::vec2(20.0, 20.0))
                            .corner_radius(3.0),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("ZODE")
                            .strong()
                            .size(11.0)
                            .color(egui::Color32::from_rgb(140, 140, 145)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if title_bar_icon(ui, egui_phosphor::regular::X, false).clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
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
                    });
                });

                Self::handle_title_bar_drag(ui, &title_resp, title_bar_rect, on_resize_edge);
            });
    }

    fn render_central_panel(&mut self, ctx: &egui::Context, state: &crate::state::StateSnapshot) {
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
                        ui.label("ZODE is stopped. Go to Settings to start.");
                    });
                    return;
                }
                if self.tab == Tab::Log && self.prev_tab != Tab::Log {
                    self.log_scroll_to_bottom = true;
                }
                if self.tab == Tab::Interlink && self.prev_tab != Tab::Interlink {
                    if let Some(ref mut il) = self.interlink_state {
                        il.focus_compose = true;
                    }
                }
                self.prev_tab = self.tab;

                match self.tab {
                    Tab::Status => crate::render::render_status(self, ui, state),
                    Tab::Storage => crate::render_storage::render_storage(self, ui, state),
                    Tab::Peers => crate::render::render_peers(self, ui, state),
                    Tab::Log => crate::render::render_log(self, ui, state),
                    Tab::Interlink => crate::interlink::render_interlink(self, ui),
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
                ctx.viewport_rect(),
                0.0,
                egui::Stroke::new(1.0, crate::components::colors::BORDER),
                egui::StrokeKind::Outside,
            );
        }
    }
}

impl eframe::App for ZodeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let on_resize_edge = if !maximized {
            Self::handle_resize_edges(ctx)
        } else {
            false
        };

        match self.phase.clone() {
            AppPhase::Setup => {
                self.render_pre_auth_title_bar(ctx, maximized, on_resize_edge);
                self.render_setup_screen(ctx);
                Self::render_window_border(ctx, maximized);
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            AppPhase::ProfileSelect => {
                self.render_pre_auth_title_bar(ctx, maximized, on_resize_edge);
                self.render_profile_select(ctx);
                Self::render_window_border(ctx, maximized);
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            AppPhase::Unlock { profile_id } => {
                self.render_pre_auth_title_bar(ctx, maximized, on_resize_edge);
                self.render_unlock_screen(ctx, &profile_id);
                Self::render_window_border(ctx, maximized);
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            AppPhase::Revealing | AppPhase::Running => {}
        }

        let state = self
            .rt
            .block_on(async { self.shared.lock().await.snapshot() });

        self.sync_visualization(&state);

        self.render_title_bar(ctx, maximized, on_resize_edge);
        self.render_central_panel(ctx, &state);
        Self::render_window_border(ctx, maximized);

        if matches!(self.phase, AppPhase::Revealing) {
            let start = *self
                .reveal_start
                .get_or_insert_with(|| ctx.input(|i| i.time));
            let now = ctx.input(|i| i.time);
            let t = ((now - start) / Self::REVEAL_DURATION).clamp(0.0, 1.0) as f32;
            if t < 1.0 {
                Self::render_reveal_overlay(ctx, t);
                ctx.request_repaint();
            } else {
                self.phase = AppPhase::Running;
                self.reveal_start = None;
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

impl ZodeApp {
    const REVEAL_DURATION: f64 = 0.75;

    fn render_reveal_overlay(ctx: &egui::Context, progress: f32) {
        fn ease_out_cubic(t: f32) -> f32 {
            1.0 - (1.0 - t).powi(3)
        }

        let eased = ease_out_cubic(progress);
        let screen = ctx.viewport_rect();
        let center_x = screen.center().x;
        let half_w = screen.width() / 2.0;
        let offset = half_w * eased;

        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("reveal_overlay"),
        ));

        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(screen.left() - offset, screen.top()),
                egui::pos2(center_x - offset, screen.bottom()),
            ),
            0.0,
            egui::Color32::BLACK,
        );

        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(center_x + offset, screen.top()),
                egui::pos2(screen.right() + offset, screen.bottom()),
            ),
            0.0,
            egui::Color32::BLACK,
        );

        let glow_strength = (1.0 - eased).powi(2);
        if glow_strength > 0.01 {
            let left_edge = center_x - offset;
            let right_edge = center_x + offset;
            let edge_alpha = (glow_strength * 200.0) as u8;

            let edge_color = egui::Color32::from_rgba_unmultiplied(46, 230, 176, edge_alpha);
            painter.line_segment(
                [
                    egui::pos2(left_edge, screen.top()),
                    egui::pos2(left_edge, screen.bottom()),
                ],
                egui::Stroke::new(1.5, edge_color),
            );
            painter.line_segment(
                [
                    egui::pos2(right_edge, screen.top()),
                    egui::pos2(right_edge, screen.bottom()),
                ],
                egui::Stroke::new(1.5, edge_color),
            );

            for i in 1..=6u8 {
                let falloff = 1.0 - (i as f32 / 7.0);
                let a = (edge_alpha as f32 * falloff * 0.35) as u8;
                let w = i as f32 * 2.5;
                let c = egui::Color32::from_rgba_unmultiplied(46, 230, 176, a);
                painter.line_segment(
                    [
                        egui::pos2(left_edge - w, screen.top()),
                        egui::pos2(left_edge - w, screen.bottom()),
                    ],
                    egui::Stroke::new(2.0, c),
                );
                painter.line_segment(
                    [
                        egui::pos2(right_edge + w, screen.top()),
                        egui::pos2(right_edge + w, screen.bottom()),
                    ],
                    egui::Stroke::new(2.0, c),
                );
            }
        }
    }

    fn render_profile_select(&mut self, ctx: &egui::Context) {
        let tex = self.icon_texture(ctx);
        let frame = egui::Frame::default()
            .fill(egui::Color32::BLACK)
            .inner_margin(32.0);

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let rect = ui.max_rect();
            ui.vertical_centered(|ui| {
                let content_height = 200.0;
                ui.add_space(((rect.height() - content_height) / 2.0).max(20.0));

                ui.add(
                    egui::Image::new(&tex)
                        .fit_to_exact_size(egui::vec2(56.0, 56.0))
                        .corner_radius(8.0),
                );
                ui.add_space(16.0);

                ui.label(
                    egui::RichText::new("SELECT PROFILE")
                        .strong()
                        .size(12.0)
                        .color(egui::Color32::from_rgb(140, 140, 145)),
                );
                ui.add_space(16.0);

                let profiles = self.profiles.clone();
                for p in &profiles {
                    let btn =
                        egui::Button::new(egui::RichText::new(&p.name).monospace().size(12.0))
                            .min_size(egui::vec2(230.0, 36.0));

                    if ui.add(btn).clicked() {
                        self.phase = AppPhase::Unlock {
                            profile_id: p.id.clone(),
                        };
                    }
                    ui.add_space(4.0);
                }

            });
        });
    }

    fn render_unlock_screen(&mut self, ctx: &egui::Context, profile_id: &str) {
        let profile_id = profile_id.to_string();
        let profile_name = self
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| profile_id.clone());

        let tex = self.icon_texture(ctx);
        let frame = egui::Frame::default()
            .fill(egui::Color32::BLACK)
            .inner_margin(32.0);

        let mut do_unlock = false;

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let rect = ui.max_rect();
            ui.vertical_centered(|ui| {
                let content_height = 220.0;
                ui.add_space(((rect.height() - content_height) / 2.0).max(20.0));

                ui.add(
                    egui::Image::new(&tex)
                        .fit_to_exact_size(egui::vec2(56.0, 56.0))
                        .corner_radius(8.0),
                );

                ui.add_space(16.0);

                ui.label(
                    egui::RichText::new(&profile_name)
                        .size(13.0)
                        .color(egui::Color32::from_rgb(160, 160, 165)),
                );

                ui.add_space(20.0);

                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.unlock_password)
                        .password(true)
                        .desired_width(280.0)
                        .hint_text("Enter your password")
                        .margin(egui::Margin::symmetric(8, 6)),
                );
                if self.unlock_password.is_empty() && !resp.has_focus() {
                    resp.request_focus();
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_unlock = true;
                }

                ui.add_space(8.0);

                if crate::components::action_button(ui, "Unlock") {
                    do_unlock = true;
                }

                ui.add_space(24.0);

                if self.profiles.len() > 1 {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Back to profiles")
                                    .size(11.0)
                                    .color(egui::Color32::from_rgb(100, 100, 108)),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.unlock_password.clear();
                        self.unlock_error = None;
                        self.phase = AppPhase::ProfileSelect;
                    }
                    ui.add_space(4.0);
                }

            });

            if let Some(ref err) = self.unlock_error {
                let err_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left(), rect.bottom() - 28.0),
                    egui::vec2(rect.width(), 20.0),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(err_rect), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.colored_label(crate::components::colors::ERROR, err);
                    });
                });
            }
        });

        if do_unlock {
            self.attempt_unlock(&profile_id);
        }
    }
}

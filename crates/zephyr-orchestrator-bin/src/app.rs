use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

use crate::components::tokens::{colors, font_size, spacing};
use crate::components::{
    danger_button, status_bar_frame, title_bar_frame, title_bar_icon,
};
use crate::node_manager::{self, ManagedNode};
use crate::render_dashboard;
use crate::render_launch;
use crate::render_log;
use crate::render_nodes;
use crate::render_topology::TopologyVisualization;
use crate::state::{AppPhase, AppState, LogLevel, NetworkPreset, Tab};

pub(crate) struct OrchestratorApp {
    pub rt: Runtime,
    pub shared: Arc<Mutex<AppState>>,
    pub managed_nodes: Vec<ManagedNode>,
    pub poller_handles: Vec<tokio::task::JoinHandle<()>>,
    pub log_handles: Vec<tokio::task::JoinHandle<()>>,

    pub tab: Tab,
    pub phase: AppPhase,
    pub selected_preset: NetworkPreset,
    pub launching: bool,
    pub launch_error: Option<String>,
    pub launch_instant: Option<Instant>,

    pub topology: TopologyVisualization,
    pub icon_texture: Option<egui::TextureHandle>,
    pub log_level_filter: Option<LogLevel>,

    #[allow(dead_code)]
    pub auto_traffic: bool,
    #[allow(dead_code)]
    pub traffic_rate: f32,
}

impl OrchestratorApp {
    pub fn new(rt: Runtime) -> Self {
        Self {
            rt,
            shared: Arc::new(Mutex::new(AppState::default())),
            managed_nodes: Vec::new(),
            poller_handles: Vec::new(),
            log_handles: Vec::new(),

            tab: Tab::Dashboard,
            phase: AppPhase::Launch,
            selected_preset: NetworkPreset::Standard,
            launching: false,
            launch_error: None,
            launch_instant: None,

            topology: TopologyVisualization::default(),
            icon_texture: None,
            log_level_filter: None,

            auto_traffic: false,
            traffic_rate: 1.0,
        }
    }

    pub fn do_launch(&mut self) {
        self.launching = true;
        self.launch_error = None;
        let nodes = node_manager::launch_network(&self.selected_preset, &self.rt, &self.shared);
        if nodes.is_empty() {
            self.launch_error = Some("Failed to start any nodes".to_string());
            self.launching = false;
            return;
        }

        let pollers =
            node_manager::spawn_status_pollers(&nodes, Arc::clone(&self.shared), &self.rt);
        let listeners =
            node_manager::spawn_log_listeners(&nodes, Arc::clone(&self.shared), &self.rt);

        self.managed_nodes = nodes;
        self.poller_handles = pollers;
        self.log_handles = listeners;
        self.phase = AppPhase::Running;
        self.launching = false;
        self.launch_instant = Some(Instant::now());
    }

    fn do_shutdown(&mut self) {
        self.phase = AppPhase::ShuttingDown;
        for h in self.poller_handles.drain(..) {
            h.abort();
        }
        for h in self.log_handles.drain(..) {
            h.abort();
        }
        node_manager::shutdown_all(&self.managed_nodes, &self.rt);
        self.managed_nodes.clear();

        self.rt.block_on(async {
            let mut state = self.shared.lock().await;
            state.nodes.clear();
            state.log_entries.clear();
        });

        self.phase = AppPhase::Launch;
        self.launch_instant = None;
    }

    fn icon_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.icon_texture
            .get_or_insert_with(|| {
                let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
                    .expect("bad icon png")
                    .to_rgba8();
                let size = [img.width() as _, img.height() as _];
                let pixels = img.into_raw();
                ctx.load_texture(
                    "app_icon",
                    egui::ColorImage::from_rgba_unmultiplied(size, &pixels),
                    egui::TextureOptions::LINEAR,
                )
            })
            .clone()
    }
}

impl eframe::App for OrchestratorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let _ = self.icon_texture(ctx);

        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let on_resize_edge = if !maximized {
            Self::handle_resize_edges(ctx)
        } else {
            false
        };

        let state_snapshot = self.rt.block_on(async {
            let s = self.shared.lock().await;
            snapshot(&s)
        });

        if self.phase == AppPhase::Running {
            self.topology.reconcile(&state_snapshot);
        }

        self.render_title_bar(ctx, maximized, on_resize_edge);

        if self.phase == AppPhase::Running {
            egui::TopBottomPanel::bottom("status_bar")
                .frame(status_bar_frame())
                .show(ctx, |ui| {
                    self.render_status_bar(ui, &state_snapshot);
                });
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(colors::PANEL_BG)
                    .inner_margin(0.0),
            )
            .show(ctx, |ui| match self.phase {
                AppPhase::Launch => {
                    render_launch::render_launch_screen(self, ui);
                }
                AppPhase::Running => {
                    self.render_running(ui, &state_snapshot);
                }
                AppPhase::ShuttingDown => {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 2.0 - 20.0);
                        ui.spinner();
                        ui.label("Shutting down...");
                    });
                }
            });

        if self.phase == AppPhase::Running {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        render_window_border(ctx);
    }
}

impl OrchestratorApp {
    fn render_title_bar(
        &mut self,
        ctx: &egui::Context,
        maximized: bool,
        on_resize_edge: bool,
    ) {
        egui::TopBottomPanel::top("title_bar")
            .frame(title_bar_frame())
            .show(ctx, |ui| {
                let title_bar_rect = ui.max_rect();
                let title_resp = ui.interact(
                    title_bar_rect,
                    egui::Id::new("title_bar"),
                    egui::Sense::click_and_drag(),
                );
                if !on_resize_edge
                    && title_resp.drag_started_by(egui::PointerButton::Primary)
                {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::StartDrag);
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
                    ui.label(
                        egui::RichText::new("ZEPHYR ORCHESTRATOR")
                            .strong()
                            .size(font_size::BODY)
                            .color(colors::TEXT_HEADING),
                    );

                    ui.add_space(spacing::XXL);

                    if self.phase == AppPhase::Running {
                        for &tab in Tab::ALL {
                            let active = self.tab == tab;
                            if title_bar_icon(ui, tab.icon(), active).clicked() {
                                self.tab = tab;
                            }
                        }
                    }

                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if title_bar_icon(ui, egui_phosphor::regular::X, false).clicked()
                            {
                                if self.phase == AppPhase::Running {
                                    self.do_shutdown();
                                }
                                ui.ctx()
                                    .send_viewport_cmd(egui::ViewportCommand::Close);
                            }

                            let max_icon = if maximized {
                                egui_phosphor::regular::CORNERS_IN
                            } else {
                                egui_phosphor::regular::CORNERS_OUT
                            };
                            if title_bar_icon(ui, max_icon, false).clicked() {
                                ui.ctx().send_viewport_cmd(
                                    egui::ViewportCommand::Maximized(!maximized),
                                );
                            }

                            if title_bar_icon(
                                ui,
                                egui_phosphor::regular::MINUS,
                                false,
                            )
                            .clicked()
                            {
                                ui.ctx().send_viewport_cmd(
                                    egui::ViewportCommand::Minimized(true),
                                );
                            }

                            ui.add_space(spacing::SM);

                            if self.phase == AppPhase::Running {
                                if danger_button(ui, "Shutdown") {
                                    self.do_shutdown();
                                }
                                ui.add_space(spacing::MD);
                            }
                        },
                    );
                });

                Self::handle_title_bar_drag(
                    ui,
                    &title_resp,
                    title_bar_rect,
                    on_resize_edge,
                );
            });
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
            ResizeDirection::NorthWest | ResizeDirection::SouthEast => {
                egui::CursorIcon::ResizeNwSe
            }
            ResizeDirection::NorthEast | ResizeDirection::SouthWest => {
                egui::CursorIcon::ResizeNeSw
            }
        };
        ctx.set_cursor_icon(cursor);

        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }

        true
    }

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
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }
    }

    fn render_running(&mut self, ui: &mut egui::Ui, state: &AppState) {
        match self.tab {
            Tab::Dashboard => {
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::default()
                            .fill(colors::PANEL_BG)
                            .inner_margin(spacing::LG),
                    )
                    .show_inside(ui, |ui| {
                        render_dashboard::render_dashboard(self, ui, state);
                    });
            }
            Tab::Nodes => {
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::default()
                            .fill(colors::PANEL_BG)
                            .inner_margin(spacing::LG),
                    )
                    .show_inside(ui, |ui| {
                        render_nodes::render_nodes(ui, state);
                    });
            }
            Tab::Topology => {
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::default()
                            .fill(colors::PANEL_BG)
                            .inner_margin(0.0),
                    )
                    .show_inside(ui, |ui| {
                        self.topology.render(ui);
                    });
            }
            Tab::Log => {
                render_log::render_log(ui, state, &mut self.log_level_filter);
            }
        }
    }

    fn render_status_bar(&self, ui: &mut egui::Ui, state: &AppState) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{} nodes", state.nodes.len()))
                    .size(font_size::SMALL)
                    .color(colors::TEXT_MUTED),
            );
            ui.separator();
            ui.label(
                egui::RichText::new(format!("{} peers", state.network.total_peers))
                    .size(font_size::SMALL)
                    .color(colors::TEXT_MUTED),
            );
            ui.separator();
            ui.label(
                egui::RichText::new(format!("Epoch {}", state.network.current_epoch))
                    .size(font_size::SMALL)
                    .color(colors::TEXT_MUTED),
            );
            ui.separator();
            ui.label(
                egui::RichText::new(format!("{} zones", state.network.total_zones))
                    .size(font_size::SMALL)
                    .color(colors::TEXT_MUTED),
            );

            if let Some(started) = self.launch_instant {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(crate::helpers::format_uptime(
                            started.elapsed().as_secs(),
                        ))
                        .size(font_size::SMALL)
                        .color(colors::CONNECTED),
                    );
                });
            }
        });
    }
}

fn snapshot(state: &AppState) -> AppState {
    AppState {
        phase: state.phase,
        nodes: state
            .nodes
            .iter()
            .map(|n| crate::state::NodeState {
                node_id: n.node_id,
                zode_id: n.zode_id.clone(),
                status: n.status.clone(),
                assigned_zones: n.assigned_zones.clone(),
                is_leader_in: n.is_leader_in.clone(),
                mempool_sizes: n.mempool_sizes.clone(),
                last_update: n.last_update,
            })
            .collect(),
        network: crate::state::NetworkSnapshot {
            total_zones: state.network.total_zones,
            current_epoch: state.network.current_epoch,
            epoch_progress_pct: state.network.epoch_progress_pct,
            zone_heads: state.network.zone_heads.clone(),
            certificates_produced: state.network.certificates_produced,
            spends_processed: state.network.spends_processed,
            total_peers: state.network.total_peers,
        },
        log_entries: state
            .log_entries
            .iter()
            .map(|e| crate::state::AggregatedLogEntry {
                node_id: e.node_id,
                line: e.line.clone(),
                level: e.level,
                timestamp: e.timestamp,
            })
            .collect(),
        launch_start: state.launch_start,
        auto_traffic: state.auto_traffic,
        traffic_rate: state.traffic_rate,
        traffic_stats: crate::state::TrafficStats {
            total_submitted: state.traffic_stats.total_submitted,
            recent: state
                .traffic_stats
                .recent
                .iter()
                .map(|r| crate::state::RecentTransaction {
                    nullifier_hex: r.nullifier_hex.clone(),
                    zone_id: r.zone_id,
                    timestamp: r.timestamp,
                })
                .collect(),
        },
    }
}

fn render_window_border(ctx: &egui::Context) {
    #[allow(deprecated)]
    let rect = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("window_border"),
    ));
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, colors::BORDER_DIM),
        egui::StrokeKind::Inside,
    );
}

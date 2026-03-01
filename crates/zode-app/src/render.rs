use eframe::egui;

use crate::app::ZodeApp;
use crate::components::{
    action_button, action_panel, colors, copy_button, editable_list, error_label, field_label,
    hint_label, info_grid, kv_row, kv_row_copyable, muted_label, section, text_input,
};
use crate::helpers::format_bytes;
use crate::state::StateSnapshot;

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub(crate) fn render_settings(app: &mut ZodeApp, ui: &mut egui::Ui) {
    if let Some(ref err) = app.settings_error {
        error_label(ui, err);
    }

    let running = app.zode.is_some();
    let mut do_boot = false;
    let mut do_stop = false;

    action_panel(ui, "settings_actions_panel", |ui| {
        if running {
            if action_button(ui, "STOP ZODE") {
                do_stop = true;
            }
            ui.add_space(12.0);
        }
        let label = if running {
            "RESTART ZODE"
        } else {
            "START ZODE"
        };
        if action_button(ui, label) {
            do_boot = true;
        }
    });

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            render_settings_general(app, ui, running);
            render_bootstrap_peers(app, ui);
            render_relay_peers(app, ui);
            render_default_programs(app, ui);
            render_topics(app, ui);
            render_discovery_settings(app, ui);
            render_rpc_settings(app, ui);
            render_known_peers(app, ui);
        });

    if do_boot {
        app.save_settings();
        app.boot_zode();
    }
    if do_stop {
        app.stop_zode();
    }
}

fn render_settings_general(app: &mut ZodeApp, ui: &mut egui::Ui, running: bool) {
    section(ui, "General", |ui| {
        let msg = if running {
            "The ZODE is running. Edit settings below and click Restart to apply."
        } else {
            "The ZODE is stopped. Edit settings and click Start."
        };
        hint_label(ui, msg);
        ui.add_space(8.0);

        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                field_label(ui, "Data Directory");
                ui.add(text_input(&mut app.settings.data_dir, 400.0));
                ui.end_row();
                field_label(ui, "Listen Address");
                ui.add(text_input(&mut app.settings.listen_addr, 400.0));
                ui.end_row();
            });
    });
}

fn render_bootstrap_peers(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "Bootstrap Peers", |ui| {
        hint_label(
            ui,
            "Multiaddrs of other ZODE nodes to connect to on startup.",
        );
        ui.add_space(8.0);
        editable_list(
            ui,
            &mut app.settings.bootstrap_peers,
            &mut app.settings.bootstrap_input,
            360.0,
        );
    });
}

fn render_relay_peers(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "Relay", |ui| {
        hint_label(
            ui,
            "Relay-first connectivity for NAT-restricted nodes. Add public relay peers and enable relay transport.",
        );
        ui.add_space(8.0);
        ui.checkbox(&mut app.settings.enable_relay, "Enable Relay Transport");
        ui.add_space(8.0);
        editable_list(
            ui,
            &mut app.settings.relay_peers,
            &mut app.settings.relay_input,
            360.0,
        );
    });
}

fn render_default_programs(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "Default Programs", |ui| {
        hint_label(
            ui,
            "Standard programs the ZODE subscribes to. Toggle off to skip.",
        );
        ui.add_space(8.0);
        ui.checkbox(&mut app.settings.enable_zid, "ZID (Zero Identity)");
        ui.checkbox(&mut app.settings.enable_interlink, "Interlink");
    });
}

fn render_topics(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "Additional Programs", |ui| {
        hint_label(ui, "64-character hex program IDs for non-default programs.");
        ui.add_space(8.0);
        editable_list(
            ui,
            &mut app.settings.topics,
            &mut app.settings.topic_input,
            360.0,
        );
    });
}

fn render_discovery_settings(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "Discovery", |ui| {
        hint_label(
            ui,
            "Automatic zode discovery via Kademlia DHT. Nodes find each other transitively.",
        );
        ui.add_space(8.0);
        ui.checkbox(&mut app.settings.enable_kademlia, "Enable DHT Discovery");

        if app.settings.enable_kademlia {
            ui.indent("kad_settings", |ui| {
                ui.checkbox(
                    &mut app.settings.kademlia_server_mode,
                    "Server mode (respond to DHT queries from other zodes)",
                );
                ui.horizontal(|ui| {
                    field_label(ui, "Random walk interval (seconds)");
                    ui.add(
                        egui::DragValue::new(&mut app.settings.random_walk_interval_secs)
                            .range(5..=300)
                            .speed(1),
                    );
                });
            });
        }
    });
}

fn render_rpc_settings(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "RPC Server", |ui| {
        hint_label(ui, "JSON-RPC HTTP server for external app access.");
        ui.add_space(8.0);
        ui.checkbox(&mut app.settings.enable_rpc, "Enable RPC Server");

        if app.settings.enable_rpc {
            ui.indent("rpc_settings", |ui| {
                egui::Grid::new("rpc_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        field_label(ui, "Bind Address");
                        ui.add(text_input(&mut app.settings.rpc_bind_addr, 300.0));
                        ui.end_row();
                        field_label(ui, "API Key (empty = open)");
                        ui.add(text_input(&mut app.settings.rpc_api_key, 300.0));
                        ui.end_row();
                    });
            });
        }
    });
}

fn render_known_peers(app: &mut ZodeApp, ui: &mut egui::Ui) {
    if app.settings.known_peers.is_empty() {
        return;
    }
    section(ui, "Known Peers (auto-saved)", |ui| {
        hint_label(
            ui,
            "Peers remembered from previous sessions. They are added to bootstrap on startup.",
        );
        ui.add_space(8.0);
        let mut to_remove = Vec::new();
        for (i, peer) in app.settings.known_peers.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.monospace(peer);
                if ui
                    .small_button(egui_phosphor::regular::TRASH)
                    .on_hover_text("Remove")
                    .clicked()
                {
                    to_remove.push(i);
                }
            });
        }
        for i in to_remove.into_iter().rev() {
            app.settings.known_peers.remove(i);
        }
        ui.add_space(4.0);
        if action_button(ui, "Clear All") {
            app.settings.known_peers.clear();
        }
    });
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

const STATUS_FADE_DURATION: f64 = 0.5;

pub(crate) fn render_status(app: &mut ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let full_rect = ui.max_rect();
    let now = ui.input(|i| i.time);

    if state.status.is_some() && app.status_first_seen.is_none() {
        app.status_first_seen = Some(now);
    }

    let Some(ref status) = state.status else {
        egui::Frame::default()
            .fill(colors::SURFACE)
            .corner_radius(0.0)
            .inner_margin(0.0)
            .outer_margin(egui::Margin::symmetric(1, 0))
            .stroke(egui::Stroke::new(1.0, colors::BORDER))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(ui.available_height());
                app.visualization.render(ui);
            });
        return;
    };

    egui::TopBottomPanel::bottom("status_sections_panel")
        .resizable(false)
        .frame(
            egui::Frame::default()
                .fill(egui::Color32::BLACK)
                .inner_margin(egui::Margin {
                    left: 1,
                    right: 1,
                    top: 8,
                    bottom: 0,
                }),
        )
        .show_inside(ui, |ui| {
            render_zode_status(ui, status, state);
            render_storage_status(ui, status);
            render_metrics_status(ui, &status.metrics);
            render_rpc_status(ui, status);
        });

    egui::Frame::default()
        .fill(colors::SURFACE)
        .corner_radius(0.0)
        .inner_margin(0.0)
        .outer_margin(egui::Margin::symmetric(1, 0))
        .stroke(egui::Stroke::new(1.0, colors::BORDER))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height());
            app.visualization.render(ui);
        });

    let fade_t = app
        .status_first_seen
        .map(|start| ((now - start) / STATUS_FADE_DURATION).clamp(0.0, 1.0) as f32)
        .unwrap_or(1.0);

    if fade_t < 1.0 {
        let alpha = ((1.0 - fade_t) * 255.0) as u8;
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("status_fade"),
        ));
        painter.rect_filled(full_rect, 0.0, egui::Color32::from_black_alpha(alpha));
        ui.ctx().request_repaint();
    }
}

fn render_zode_status(ui: &mut egui::Ui, status: &zode::ZodeStatus, state: &StateSnapshot) {
    let full_addr = state
        .listen_addr
        .as_ref()
        .map(|listen| format!("{listen}/p2p/{}", status.zode_id));

    section(ui, "ZODE", |ui| {
        info_grid(ui, "status_grid", |ui| {
            kv_row_copyable(ui, "ZODE ID", &status.zode_id);

            field_label(ui, "Address");
            ui.horizontal(|ui| {
                if let Some(ref addr) = full_addr {
                    ui.monospace(addr);
                    copy_button(ui, addr);
                } else {
                    muted_label(ui, "resolving...");
                }
            });
            ui.end_row();

            kv_row(
                ui,
                if status.peer_count == 1 {
                    "Peer"
                } else {
                    "Peers"
                },
                &format!("{}", status.peer_count),
            );
            kv_row(ui, "Programs", &format!("{}", status.topics.len()));
        });
    });
}

fn render_storage_status(ui: &mut egui::Ui, status: &zode::ZodeStatus) {
    section(ui, "Storage", |ui| {
        info_grid(ui, "storage_grid", |ui| {
            kv_row(ui, "DB Size", &format_bytes(status.metrics.db_size_bytes));
            kv_row(
                ui,
                "Sectors stored",
                &format!("{}", status.metrics.sectors_stored_total),
            );
        });
    });
}

fn render_metrics_status(ui: &mut egui::Ui, m: &zode::MetricsSnapshot) {
    section(ui, "Metrics", |ui| {
        info_grid(ui, "metrics_grid", |ui| {
            kv_row(ui, "Sectors stored", &format!("{}", m.sectors_stored_total));
            kv_row(
                ui,
                "Rejections",
                &format!(
                    "{} (policy: {}, limit: {})",
                    m.store_rejections_total, m.policy_rejections, m.limit_rejections,
                ),
            );
        });
    });
}

fn render_rpc_status(ui: &mut egui::Ui, status: &zode::ZodeStatus) {
    section(ui, "RPC", |ui| {
        info_grid(ui, "rpc_status_grid", |ui| {
            if status.rpc_enabled {
                kv_row(
                    ui,
                    "Status",
                    &format!(
                        "Listening on {}",
                        status.rpc_addr.as_deref().unwrap_or("...")
                    ),
                );
                kv_row(
                    ui,
                    "Auth",
                    if status.rpc_auth_required {
                        "API key required"
                    } else {
                        "Open"
                    },
                );
                kv_row(
                    ui,
                    "Requests",
                    &format!("{}", status.metrics.rpc_requests_total),
                );
            } else {
                kv_row(ui, "Status", "Disabled");
            }
        });
    });
}

// ---------------------------------------------------------------------------
// Peers
// ---------------------------------------------------------------------------

pub(crate) fn render_peers(_app: &ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let Some(ref status) = state.status else {
        ui.vertical_centered(|ui| {
            let avail = ui.available_height();
            ui.add_space((avail / 2.0 - 20.0).max(0.0));
            ui.spinner();
            ui.label("Loading...");
        });
        return;
    };

    section(ui, "Connected Zodes", |ui| {
        ui.set_min_height(ui.available_height());
        info_grid(ui, "peer_info", |ui| {
            kv_row(ui, "Local ZODE", &status.zode_id);
            kv_row(ui, "Connected", &format!("{}", status.peer_count));
        });

        ui.add_space(4.0);
        hint_label(
            ui,
            "ZODE discovery via GossipSub / bootstrap peers / Kademlia DHT.",
        );
        ui.add_space(8.0);

        if status.connected_peers.is_empty() {
            muted_label(ui, "No connected ZODEs.");
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    for peer in &status.connected_peers {
                        ui.horizontal(|ui| {
                            ui.monospace(peer);
                            copy_button(ui, peer);
                        });
                    }
                });
        }
    });
}

// ---------------------------------------------------------------------------
// Log
// ---------------------------------------------------------------------------

pub(crate) fn render_log(app: &mut ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let should_scroll = app.log_scroll_to_bottom;
    app.log_scroll_to_bottom = false;

    section(
        ui,
        &format!("Live Log ({})", state.log_entries.len()),
        |ui| {
            ui.set_min_height(ui.available_height());
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for entry in &state.log_entries {
                        let color = log_entry_color(entry);
                        ui.label(egui::RichText::new(entry).monospace().color(color));
                    }
                    if should_scroll {
                        ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                    }
                });
        },
    );
}

fn log_entry_color(entry: &str) -> egui::Color32 {
    use zode::LogLevel;
    match LogLevel::from_log_line(entry) {
        LogLevel::Reject => egui::Color32::from_rgb(255, 100, 100),
        LogLevel::Gossip => egui::Color32::from_rgb(100, 200, 255),
        LogLevel::Discovery => egui::Color32::from_rgb(100, 150, 255),
        LogLevel::PeerConnect => colors::CONNECTED,
        LogLevel::PeerDisconnect => egui::Color32::from_rgb(255, 255, 100),
        LogLevel::Relay => egui::Color32::from_rgb(180, 130, 255),
        LogLevel::DialError => egui::Color32::from_rgb(255, 140, 60),
        LogLevel::Rpc => egui::Color32::from_rgb(100, 220, 220),
        LogLevel::Shutdown => egui::Color32::from_rgb(200, 100, 255),
        LogLevel::Normal => egui::Color32::from_rgb(200, 200, 200),
    }
}

// ---------------------------------------------------------------------------
// Info
// ---------------------------------------------------------------------------

pub(crate) fn render_info(_app: &ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let Some(ref status) = state.status else {
        ui.vertical_centered(|ui| {
            let avail = ui.available_height();
            ui.add_space((avail / 2.0 - 20.0).max(0.0));
            ui.spinner();
            ui.label("Loading...");
        });
        return;
    };

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            section(ui, "ZODE Info", |ui| {
                info_grid(ui, "info_grid", |ui| {
                    kv_row(ui, "ZODE ID", &status.zode_id);
                    kv_row(ui, "DB Size", &format_bytes(status.metrics.db_size_bytes));
                    kv_row(
                        ui,
                        "Sectors Stored",
                        &format!("{}", status.metrics.sectors_stored_total),
                    );
                });
            });

            section(ui, "RPC", |ui| {
                info_grid(ui, "info_rpc_grid", |ui| {
                    if status.rpc_enabled {
                        let addr = status.rpc_addr.as_deref().unwrap_or("...");
                        let auth = if status.rpc_auth_required {
                            "key"
                        } else {
                            "open"
                        };
                        kv_row(ui, "RPC", &format!("{addr} (auth: {auth})"));
                    } else {
                        kv_row(ui, "RPC", "Disabled");
                    }
                });
            });

            section(ui, "Subscribed Programs", |ui| {
                for topic in &status.topics {
                    ui.monospace(format!("  {topic}"));
                }
            });

            muted_label(ui, &format!("zode-app v{}", env!("CARGO_PKG_VERSION")));
        });
}

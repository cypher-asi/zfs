use eframe::egui;

use crate::app::ZodeApp;
use crate::components::{
    action_button, action_panel, colors, copy_button, editable_list, error_label, field_label,
    form_grid, hint_label, info_grid, kv_row, kv_row_copyable, loading_state, muted_label,
    section, text_input,
};
use crate::components::tokens::{font_size, spacing};
use crate::helpers::format_bytes;
use crate::state::{SettingsSection, StateSnapshot};

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub(crate) fn render_settings(app: &mut ZodeApp, ui: &mut egui::Ui) {
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

    let nav_resp = egui::SidePanel::left("settings_nav")
        .exact_width(168.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::default()
                .fill(colors::PANEL_BG)
                .inner_margin(egui::Margin {
                    left: 0,
                    right: spacing::MD as i8,
                    top: spacing::LG as i8,
                    bottom: spacing::LG as i8,
                })
                .outer_margin(egui::Margin {
                    left: 0,
                    right: spacing::MD as i8,
                    top: 0,
                    bottom: spacing::MD as i8,
                }),
        )
        .show_inside(ui, |ui| {
            ui.set_min_height(ui.available_height());
            ui.set_width(ui.available_width());

            let panel_left = ui.cursor().min.x;
            let mut row_positions: Vec<(SettingsSection, f32, f32)> = Vec::new();

            for sec in SettingsSection::ALL {
                let (clicked, row_y, row_h) = settings_nav_item(ui, app.settings_section, sec);
                row_positions.push((sec, row_y, row_h));
                if clicked {
                    app.settings_section = sec;
                }
            }

            if let Some(&(_, target_y, h)) = row_positions
                .iter()
                .find(|(s, _, _)| *s == app.settings_section)
            {
                let anim_y = ui.ctx().animate_value_with_time(
                    egui::Id::new("settings_nav_indicator_y"),
                    target_y,
                    0.15,
                );
                let indicator = egui::Rect::from_min_size(
                    egui::pos2(panel_left, anim_y),
                    egui::vec2(2.0, h),
                );
                ui.painter()
                    .rect_filled(indicator, 0.0, egui::Color32::WHITE);
            }
        });

    let nav_rect = nav_resp.response.rect;
    let border_rect = egui::Rect::from_min_max(
        nav_rect.min,
        egui::pos2(nav_rect.max.x - spacing::MD, nav_rect.max.y - spacing::MD),
    );
    let border_stroke = egui::Stroke::new(1.0, colors::BORDER);
    ui.painter().rect_stroke(border_rect, 0.0, border_stroke, egui::StrokeKind::Inside);

    if let Some(ref err) = app.settings_error {
        egui::TopBottomPanel::bottom("settings_error_panel")
            .resizable(false)
            .frame(
                egui::Frame::default()
                    .fill(colors::PANEL_BG)
                    .inner_margin(egui::Margin {
                        left: spacing::MD as i8,
                        right: spacing::MD as i8,
                        top: spacing::SM as i8,
                        bottom: spacing::SM as i8,
                    }),
            )
            .show_inside(ui, |ui| {
                error_label(ui, err);
            });
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            match app.settings_section {
                SettingsSection::General => render_settings_general(app, ui, running),
                SettingsSection::Peers => render_peers_settings(app, ui),
                SettingsSection::Relay => render_relay_peers(app, ui),
                SettingsSection::DefaultPrograms => render_default_programs(app, ui),
                SettingsSection::AdditionalPrograms => render_topics(app, ui),
                SettingsSection::Discovery => render_discovery_settings(app, ui),
                SettingsSection::RpcServer => render_rpc_settings(app, ui),
            }
        });

    if do_boot {
        app.save_settings();
        app.boot_zode();
    }
    if do_stop {
        app.stop_zode();
    }
}

/// Renders a nav label and returns `(clicked, row_y, row_height)`.
/// The active indicator bar is drawn separately so it can be animated.
fn settings_nav_item(
    ui: &mut egui::Ui,
    current: SettingsSection,
    target: SettingsSection,
) -> (bool, f32, f32) {
    let active = current == target;
    let label_text = target.label().to_uppercase();

    let row_height = ui.spacing().interact_size.y;
    let row_width = ui.available_width();
    let (row_id, row_rect) = ui.allocate_space(egui::vec2(row_width, row_height));
    let response = ui.interact(row_rect, row_id, egui::Sense::click());

    let text_color = if active || response.hovered() {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_gray(180)
    };

    let text_pos = egui::pos2(row_rect.min.x + spacing::LG, row_rect.center().y);
    ui.painter().text(
        text_pos,
        egui::Align2::LEFT_CENTER,
        &label_text,
        egui::FontId::proportional(font_size::BODY),
        text_color,
    );

    (response.clicked(), row_rect.min.y, row_height)
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

        form_grid(ui, "settings_grid", |ui| {
                field_label(ui, "Data Directory");
                ui.add(text_input(&mut app.settings.data_dir, 400.0));
                ui.end_row();
                field_label(ui, "Listen Address");
                ui.add(text_input(&mut app.settings.listen_addr, 400.0));
                ui.end_row();
        });
    });
}

fn render_peers_settings(app: &mut ZodeApp, ui: &mut egui::Ui) {
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

    if !app.settings.known_peers.is_empty() {
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
                form_grid(ui, "rpc_grid", |ui| {
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
                .fill(colors::PANEL_BG)
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
        loading_state(ui);
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
        LogLevel::Reject => colors::LOG_REJECT,
        LogLevel::Gossip => colors::LOG_GOSSIP,
        LogLevel::Discovery => colors::LOG_DISCOVERY,
        LogLevel::PeerConnect => colors::CONNECTED,
        LogLevel::PeerDisconnect => colors::LOG_PEER_DISCONNECT,
        LogLevel::Relay => colors::LOG_RELAY,
        LogLevel::DialError => colors::LOG_DIAL_ERROR,
        LogLevel::Rpc => colors::LOG_RPC,
        LogLevel::Shutdown => colors::LOG_SHUTDOWN,
        LogLevel::Normal => colors::LOG_NORMAL,
    }
}

// ---------------------------------------------------------------------------
// Info
// ---------------------------------------------------------------------------

pub(crate) fn render_info(_app: &ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let Some(ref status) = state.status else {
        loading_state(ui);
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

            muted_label(ui, &format!("zode-bin v{}", env!("CARGO_PKG_VERSION")));
        });
}

use std::collections::HashMap;

use eframe::egui;
use zfs_core::Cid;
use zfs_storage::{BlockStore, HeadStore, ProgramIndex};

use crate::app::ZodeApp;
use crate::components::{
    action_button, action_panel, colors, copy_button, editable_list, error_label, field_label,
    hint_label, info_grid, kv_row, kv_row_copyable, muted_label, section, text_input,
};
use crate::helpers::{format_bytes, format_timestamp_ms};
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
        let label = if running { "RESTART ZODE" } else { "START ZODE" };
        if action_button(ui, label) {
            do_boot = true;
        }
    });

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            render_settings_general(app, ui, running);
            render_bootstrap_peers(app, ui);
            render_default_programs(app, ui);
            render_topics(app, ui);
            render_discovery_settings(app, ui);
        });

    if do_boot {
        app.boot_zode();
    }
    if do_stop {
        app.stop_zode();
    }
}

fn render_settings_general(app: &mut ZodeApp, ui: &mut egui::Ui, running: bool) {
    section(ui, "General", |ui| {
        let msg = if running {
            "The Zode is running. Edit settings below and click Restart to apply."
        } else {
            "The Zode is stopped. Edit settings and click Start."
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
        hint_label(ui, "Multiaddrs of other Zode nodes to connect to on startup.");
        ui.add_space(8.0);
        editable_list(
            ui,
            &mut app.settings.bootstrap_peers,
            &mut app.settings.bootstrap_input,
            360.0,
        );
    });
}

fn render_default_programs(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "Default Programs", |ui| {
        hint_label(ui, "Standard programs the Zode subscribes to. Toggle off to skip.");
        ui.add_space(8.0);
        ui.checkbox(&mut app.settings.enable_zid, "ZID (Zero Identity)");
        ui.checkbox(&mut app.settings.enable_zchat, "Interlink");
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
            "Automatic peer discovery via Kademlia DHT. Nodes find each other transitively.",
        );
        ui.add_space(8.0);
        ui.checkbox(&mut app.settings.enable_kademlia, "Enable DHT Discovery");

        if app.settings.enable_kademlia {
            ui.indent("kad_settings", |ui| {
                ui.checkbox(
                    &mut app.settings.kademlia_server_mode,
                    "Server mode (respond to DHT queries from other peers)",
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


// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

pub(crate) fn render_status(app: &mut ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let Some(ref status) = state.status else {
        ui.spinner();
        ui.label(if app.zode.is_some() {
            "Starting Zode..."
        } else {
            "Zode is stopped. Go to Settings to start."
        });
        return;
    };

    egui::TopBottomPanel::bottom("status_sections_panel")
        .resizable(false)
        .frame(
            egui::Frame::default()
                .fill(egui::Color32::BLACK)
                .inner_margin(egui::Margin {
                    left: 1.0,
                    right: 1.0,
                    top: 8.0,
                    bottom: 0.0,
                }),
        )
        .show_inside(ui, |ui| {
            render_zode_status(ui, status, state);
            render_storage_status(ui, status);
            render_metrics_status(ui, &status.metrics);
        });

    egui::Frame::default()
        .fill(colors::SURFACE)
        .rounding(0.0)
        .inner_margin(0.0)
        .outer_margin(egui::Margin::symmetric(1.0, 0.0))
        .stroke(egui::Stroke::new(1.0, colors::BORDER))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height());
            app.visualization.render(ui);
        });
}

fn render_zode_status(
    ui: &mut egui::Ui,
    status: &zfs_zode::ZodeStatus,
    state: &StateSnapshot,
) {
    let full_addr = state
        .listen_addr
        .as_ref()
        .map(|listen| format!("{listen}/p2p/{}", status.zode_id));

    section(ui, "Zode", |ui| {
        info_grid(ui, "status_grid", |ui| {
            kv_row_copyable(ui, "Zode ID", &status.zode_id);

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

            kv_row(ui, "Peers", &format!("{}", status.peer_count));
            kv_row(ui, "Programs", &format!("{}", status.topics.len()));
        });
    });
}

fn render_storage_status(ui: &mut egui::Ui, status: &zfs_zode::ZodeStatus) {
    section(ui, "Storage", |ui| {
        info_grid(ui, "storage_grid", |ui| {
            kv_row(ui, "DB Size", &format_bytes(status.storage.db_size_bytes));
            kv_row(ui, "Blocks", &format!("{}", status.storage.block_count));
            kv_row(ui, "Heads", &format!("{}", status.storage.head_count));
            kv_row(ui, "Programs", &format!("{}", status.storage.program_count));
        });
    });
}

fn render_metrics_status(ui: &mut egui::Ui, m: &zfs_zode::MetricsSnapshot) {
    section(ui, "Metrics", |ui| {
        info_grid(ui, "metrics_grid", |ui| {
            kv_row(ui, "Blocks stored", &format!("{}", m.blocks_stored_total));
            kv_row(
                ui,
                "Rejections",
                &format!(
                    "{} (policy: {}, proof: {}, limit: {})",
                    m.store_rejections_total,
                    m.policy_rejections,
                    m.proof_rejections,
                    m.limit_rejections,
                ),
            );
        });
    });
}

// ---------------------------------------------------------------------------
// Storage (program browser)
// ---------------------------------------------------------------------------

pub(crate) fn render_storage(app: &ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let Some(ref status) = state.status else {
        ui.spinner();
        return;
    };

    let Some(ref zode) = app.zode else {
        return;
    };

    let head_by_cid: HashMap<Cid, zfs_core::Head> = zode
        .storage()
        .list_all_heads()
        .unwrap_or_default()
        .into_iter()
        .map(|h| (h.cid, h))
        .collect();

    let known_programs: HashMap<zfs_core::ProgramId, &str> =
        zfs_programs::default_program_ids()
            .into_iter()
            .map(|(name, pid)| (pid, name))
            .collect();

    section(ui, "Program Storage", |ui| {
        ui.set_min_height(ui.available_height());
        if status.topics.is_empty() {
            muted_label(ui, "No subscribed programs.");
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for topic in &status.topics {
                    let Some(hex) = topic.strip_prefix("prog/") else {
                        continue;
                    };
                    let Ok(pid) = zfs_core::ProgramId::from_hex(hex) else {
                        continue;
                    };
                    let label = match known_programs.get(&pid) {
                        Some(name) => format!(
                            "Program: {} [{}]",
                            &hex[..16.min(hex.len())],
                            name
                        ),
                        None => format!("Program: {}", &hex[..16.min(hex.len())]),
                    };
                    ui.collapsing(label, |ui| {
                        let cids = zode.storage().list_cids(&pid).unwrap_or_default();
                        if cids.is_empty() {
                            muted_label(ui, "No CIDs stored.");
                        } else {
                            for cid in &cids {
                                render_cid_entry(ui, zode, cid, head_by_cid.get(cid));
                            }
                        }
                    });
                }
            });
    });
}

fn render_cid_entry(
    ui: &mut egui::Ui,
    zode: &zfs_zode::Zode,
    cid: &Cid,
    head: Option<&zfs_core::Head>,
) {
    let cid_hex = cid.to_hex();
    let short = &cid_hex[..16.min(cid_hex.len())];

    let label = if head.is_some() {
        format!("  {cid_hex}  [HEAD]")
    } else {
        format!("  {cid_hex}")
    };
    let header_id = format!("cid_{cid_hex}");

    egui::CollapsingHeader::new(egui::RichText::new(label).monospace())
        .id_salt(&header_id)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                field_label(ui, "CID");
                ui.monospace(&cid_hex);
                copy_button(ui, &cid_hex);
            });

            if let Some(head) = head {
                render_head_metadata(ui, head, short);
            }

            match zode.storage().get(cid) {
                Ok(Some(data)) => render_block_content(ui, &data, short),
                Ok(None) => {
                    ui.colored_label(colors::WARN, "Block not found in local store.");
                }
                Err(e) => {
                    ui.colored_label(colors::ERROR, format!("Read error: {e}"));
                }
            }
        });
}

fn render_head_metadata(ui: &mut egui::Ui, head: &zfs_core::Head, short_cid: &str) {
    ui.add_space(4.0);
    egui::CollapsingHeader::new(
        egui::RichText::new("Head (stored metadata)")
            .strong()
            .color(egui::Color32::from_rgb(255, 180, 100)),
    )
    .id_salt(format!("head_{short_cid}"))
    .default_open(true)
    .show(ui, |ui| {
        info_grid(ui, &format!("head_grid_{short_cid}"), |ui| {
            let sid_hex = head.sector_id.to_hex();
            kv_row_copyable(ui, "Sector ID", &sid_hex);
            kv_row(ui, "Version", &format!("{}", head.version));
            kv_row(
                ui,
                "Timestamp",
                &format!(
                    "{} ({}ms)",
                    format_timestamp_ms(head.timestamp_ms),
                    head.timestamp_ms
                ),
            );

            field_label(ui, "Program ID");
            ui.monospace(head.program_id.to_hex());
            ui.end_row();

            field_label(ui, "Prev Head CID");
            if let Some(ref prev) = head.prev_head_cid {
                ui.monospace(prev.to_hex());
            } else {
                muted_label(ui, "(none — first version)");
            }
            ui.end_row();

            field_label(ui, "Signature");
            if head.signature.is_some() {
                ui.label("Present (Ed25519 + ML-DSA-65)");
            } else {
                muted_label(ui, "(none)");
            }
            ui.end_row();
        });
    });
    ui.add_space(4.0);
}

const HEX_PREVIEW_BYTES: usize = 256;

fn render_block_content(ui: &mut egui::Ui, data: &[u8], short_cid: &str) {
    ui.horizontal(|ui| {
        field_label(ui, "Size");
        ui.label(format!(
            "{} ({})",
            format_bytes(data.len() as u64),
            data.len()
        ));
    });

    if let Ok(text) = std::str::from_utf8(data) {
        hint_label(ui, "Content appears to be valid UTF-8.");
        ui.add_space(4.0);
        let preview = if text.len() > 2048 {
            format!("{}...", &text[..2048])
        } else {
            text.to_string()
        };
        egui::CollapsingHeader::new("Text preview")
            .id_salt(format!("txt_{short_cid}"))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.monospace(&preview);
                    });
            });
    } else {
        hint_label(ui, "Content is binary / encrypted ciphertext.");
    }

    ui.add_space(4.0);
    egui::CollapsingHeader::new("Hex dump")
        .id_salt(format!("hex_{short_cid}"))
        .show(ui, |ui| {
            let slice = &data[..data.len().min(HEX_PREVIEW_BYTES)];
            let hex_lines = format_hex_dump(slice);
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    ui.monospace(&hex_lines);
                });
            if data.len() > HEX_PREVIEW_BYTES {
                muted_label(
                    ui,
                    &format!(
                        "... showing first {} of {} bytes",
                        HEX_PREVIEW_BYTES,
                        data.len()
                    ),
                );
            }
        });
}

fn format_hex_dump(data: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        let offset = i * 16;
        out.push_str(&format!("{offset:08x}  "));

        for (j, byte) in chunk.iter().enumerate() {
            out.push_str(&format!("{byte:02x} "));
            if j == 7 {
                out.push(' ');
            }
        }
        let padding = 16 - chunk.len();
        for j in 0..padding {
            out.push_str("   ");
            if chunk.len() + j == 7 {
                out.push(' ');
            }
        }

        out.push_str(" |");
        for &b in chunk {
            let c = if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            };
            out.push(c);
        }
        out.push_str("|\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Peers
// ---------------------------------------------------------------------------

pub(crate) fn render_peers(_app: &ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let Some(ref status) = state.status else {
        ui.spinner();
        return;
    };

    section(ui, "Connected Peers", |ui| {
        ui.set_min_height(ui.available_height());
        info_grid(ui, "peer_info", |ui| {
            kv_row(ui, "Local Zode", &status.zode_id);
            kv_row(ui, "Connected", &format!("{}", status.peer_count));
        });

        ui.add_space(4.0);
        hint_label(ui, "Peer discovery via GossipSub / bootstrap peers / Kademlia DHT.");
        ui.add_space(8.0);

        if status.connected_peers.is_empty() {
            muted_label(ui, "No connected Zodes.");
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

pub(crate) fn render_log(ui: &mut egui::Ui, state: &StateSnapshot) {
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
                });
        },
    );
}

fn log_entry_color(entry: &str) -> egui::Color32 {
    if entry.starts_with("[STORE REJECT") {
        egui::Color32::from_rgb(255, 100, 100)
    } else if entry.starts_with("[GOSSIP") {
        egui::Color32::from_rgb(100, 200, 255)
    } else if entry.starts_with("[DHT") {
        egui::Color32::from_rgb(100, 150, 255)
    } else if entry.starts_with("[PEER+") {
        egui::Color32::from_rgb(100, 255, 100)
    } else if entry.starts_with("[PEER-") {
        egui::Color32::from_rgb(255, 255, 100)
    } else if entry.starts_with("[SHUTDOWN") {
        egui::Color32::from_rgb(200, 100, 255)
    } else {
        egui::Color32::from_rgb(200, 200, 200)
    }
}

// ---------------------------------------------------------------------------
// Info
// ---------------------------------------------------------------------------

pub(crate) fn render_info(_app: &ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let Some(ref status) = state.status else {
        ui.spinner();
        return;
    };

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            section(ui, "Zode Info", |ui| {
                info_grid(ui, "info_grid", |ui| {
                    kv_row(ui, "Zode ID", &status.zode_id);
                    kv_row(ui, "DB Size", &format_bytes(status.storage.db_size_bytes));
                    kv_row(ui, "Block Count", &format!("{}", status.storage.block_count));
                    kv_row(ui, "Head Count", &format!("{}", status.storage.head_count));
                    kv_row(
                        ui,
                        "Program Count",
                        &format!("{}", status.storage.program_count),
                    );
                });
            });

            section(ui, "Subscribed Programs", |ui| {
                for topic in &status.topics {
                    ui.monospace(format!("  {topic}"));
                }
            });

            muted_label(ui, &format!("zfs-zode-app v{}", env!("CARGO_PKG_VERSION")));
        });
}

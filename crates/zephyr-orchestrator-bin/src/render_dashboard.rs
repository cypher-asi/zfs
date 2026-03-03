use std::time::Instant;

use eframe::egui;

use crate::app::OrchestratorApp;
use crate::components::tokens::{self, colors, font_size, spacing};
use crate::components::labels::field_label;
use crate::components::section;
use crate::helpers::{format_uptime, node_color};
use crate::state::AppState;

pub(crate) fn render_dashboard(app: &mut OrchestratorApp, ui: &mut egui::Ui, state: &AppState) {
    egui::ScrollArea::vertical()
        .id_salt("dashboard_scroll")
        .show(ui, |ui| {
            render_stats_bar(ui, state, app.launch_instant);
            ui.add_space(spacing::MD);
            render_traffic_controls(app, ui, state);
            ui.add_space(spacing::MD);
            render_zone_grid(ui, state);
            ui.add_space(spacing::MD);
            render_epoch_timeline(ui, state);
            ui.add_space(spacing::MD);
            render_activity_feed(ui, state);
        });
}

fn render_stats_bar(ui: &mut egui::Ui, state: &AppState, launch_instant: Option<Instant>) {
    section(ui, "Network Overview", |ui| {
        egui::Grid::new("dash_stats")
            .num_columns(8)
            .spacing([spacing::LG, spacing::XS])
            .show(ui, |ui| {
                field_label(ui, "Epoch");
                ui.label(format!("{}", state.network.current_epoch));
                field_label(ui, "Zones");
                ui.label(format!("{}", state.network.total_zones));
                field_label(ui, "Active Validators");
                ui.label(format!("{}", state.nodes.len()));
                field_label(ui, "Connected Peers");
                ui.label(format!("{}", state.network.total_peers));
                ui.end_row();

                field_label(ui, "Certificates");
                ui.label(format!("{}", state.network.certificates_produced));
                field_label(ui, "Spends Processed");
                ui.label(format!("{}", state.network.spends_processed));
                field_label(ui, "Tx Submitted");
                ui.label(format!("{}", state.traffic_stats.total_submitted));
                if let Some(started) = launch_instant {
                    field_label(ui, "Uptime");
                    ui.label(format_uptime(started.elapsed().as_secs()));
                }
                ui.end_row();
            });
    });
}

fn render_traffic_controls(app: &mut OrchestratorApp, ui: &mut egui::Ui, _state: &AppState) {
    section(ui, "Traffic Generator", |ui| {
        ui.horizontal(|ui| {
            let mut enabled = app.auto_traffic;
            if ui.checkbox(&mut enabled, "Auto-traffic").changed() {
                app.auto_traffic = enabled;
                app.sync_traffic_to_shared();
            }

            ui.add_space(spacing::LG);

            ui.label(
                egui::RichText::new("Rate (tx/s):")
                    .size(font_size::SMALL)
                    .color(colors::TEXT_SECONDARY),
            );

            let mut rate = app.traffic_rate;
            let slider = egui::Slider::new(&mut rate, 0.1..=50.0)
                .logarithmic(true)
                .text("tx/s");
            if ui.add(slider).changed() {
                app.traffic_rate = rate;
                app.sync_traffic_to_shared();
            }
        });
    });
}

fn render_zone_grid(ui: &mut egui::Ui, state: &AppState) {
    if state.network.total_zones == 0 {
        return;
    }

    section(ui, "Zone Grid", |ui| {
        let avail_w = ui.available_width();
        let card_w = 200.0_f32;
        let cols = ((avail_w + spacing::MD) / (card_w + spacing::MD))
            .floor()
            .max(1.0) as u32;

        for row_start in (0..state.network.total_zones).step_by(cols as usize) {
            ui.horizontal(|ui| {
                for zone_id in row_start..state.network.total_zones.min(row_start + cols) {
                    render_zone_card(ui, zone_id, state, card_w);
                    ui.add_space(spacing::MD);
                }
            });
            ui.add_space(spacing::MD);
        }
    });
}

fn render_zone_card(ui: &mut egui::Ui, zone_id: u32, state: &AppState, card_w: f32) {
    let card_h = 100.0;
    let (rect, _resp) = ui.allocate_exact_size(egui::vec2(card_w, card_h), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect(
        rect,
        0.0,
        colors::SURFACE_DARK,
        egui::Stroke::new(tokens::STROKE_DEFAULT, colors::BORDER),
        egui::StrokeKind::Inside,
    );

    let pad = spacing::MD;
    let inner = rect.shrink(pad);

    painter.text(
        inner.left_top(),
        egui::Align2::LEFT_TOP,
        format!("Zone {zone_id}"),
        egui::FontId::proportional(font_size::SUBTITLE),
        colors::TEXT_HEADING,
    );

    if let Some(head) = state.network.zone_heads.get(&zone_id) {
        let hex = hex::encode(&head[..4]);
        painter.text(
            egui::pos2(inner.left(), inner.top() + 18.0),
            egui::Align2::LEFT_TOP,
            format!("head: {hex}..."),
            egui::FontId::proportional(font_size::SMALL),
            colors::TEXT_MUTED,
        );
    }

    let mut dot_x = inner.left();
    let dot_y = inner.top() + 40.0;
    for ns in &state.nodes {
        if ns.assigned_zones.contains(&zone_id) {
            let color = node_color(ns.node_id);
            painter.circle_filled(egui::pos2(dot_x + 4.0, dot_y + 4.0), 4.0, color);
            dot_x += 12.0;
        }
    }

    let mempool: usize = state
        .nodes
        .iter()
        .filter_map(|ns| ns.mempool_sizes.get(&zone_id))
        .sum();
    painter.text(
        egui::pos2(inner.left(), inner.bottom() - 14.0),
        egui::Align2::LEFT_TOP,
        format!("mempool: {mempool}"),
        egui::FontId::proportional(font_size::SMALL),
        colors::TEXT_SECONDARY,
    );
}

fn render_epoch_timeline(ui: &mut egui::Ui, state: &AppState) {
    section(ui, "Epoch Timeline", |ui| {
        let avail = ui.available_width();
        let bar_h = 20.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(avail, bar_h), egui::Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 2.0, colors::SURFACE_DARK);

        let pct = state.network.epoch_progress_pct.clamp(0.0, 1.0);
        let fill_w = rect.width() * pct;
        let fill_rect = egui::Rect::from_min_size(rect.left_top(), egui::vec2(fill_w, bar_h));
        painter.rect_filled(fill_rect, 2.0, colors::ACCENT.linear_multiply(0.6));

        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            format!(
                "Epoch {} — {:.0}%",
                state.network.current_epoch,
                pct * 100.0
            ),
            egui::FontId::proportional(font_size::SMALL),
            egui::Color32::WHITE,
        );
    });
}

fn render_activity_feed(ui: &mut egui::Ui, state: &AppState) {
    if state.network.total_zones == 0 {
        return;
    }

    section(ui, "Activity Feed", |ui| {
        let avail_w = ui.available_width();
        let col_w =
            (avail_w / state.network.total_zones.max(1) as f32 - spacing::SM).clamp(180.0, 320.0);

        ui.horizontal_top(|ui| {
            for zone_id in 0..state.network.total_zones {
                render_zone_activity_column(ui, zone_id, state, col_w);
                if zone_id + 1 < state.network.total_zones {
                    ui.add_space(spacing::SM);
                }
            }
        });
    });
}

fn render_zone_activity_column(
    ui: &mut egui::Ui,
    zone_id: u32,
    state: &AppState,
    col_w: f32,
) {
    ui.vertical(|ui| {
        ui.set_width(col_w);

        ui.label(
            egui::RichText::new(format!("Zone {zone_id}"))
                .size(font_size::SUBTITLE)
                .color(colors::TEXT_HEADING),
        );

        ui.add_space(spacing::XS);

        egui::Frame::NONE
            .inner_margin(egui::Margin::same(spacing::SM as i8))
            .fill(colors::SURFACE_DARK)
            .stroke(egui::Stroke::new(tokens::STROKE_DEFAULT, colors::BORDER))
            .corner_radius(4.0)
            .show(ui, |ui| {
                ui.set_width(col_w - spacing::SM * 2.0);

                let zone_blocks: Vec<_> = state
                    .recent_blocks
                    .iter()
                    .filter(|b| b.zone_id == zone_id)
                    .collect();

                if zone_blocks.is_empty() {
                    ui.add_space(spacing::MD);
                    ui.label(
                        egui::RichText::new("awaiting blocks...")
                            .size(font_size::SMALL)
                            .color(colors::TEXT_MUTED)
                            .italics(),
                    );
                } else {
                    for block in zone_blocks.iter().rev().take(20) {
                        render_block_entry(ui, block);
                        ui.add_space(spacing::XS);
                    }
                }
            });
    });
}

fn render_block_entry(ui: &mut egui::Ui, block: &crate::state::RecentBlock) {
    let age = block.timestamp.elapsed();
    let age_str = if age.as_secs() < 60 {
        format!("{}s ago", age.as_secs())
    } else {
        format!("{}m ago", age.as_secs() / 60)
    };

    egui::Frame::NONE
        .inner_margin(egui::Margin::same(spacing::SM as i8))
        .fill(colors::SURFACE_RAISED)
        .corner_radius(3.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Block #{}", block.height))
                        .size(font_size::SMALL)
                        .color(colors::ACCENT)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(&age_str)
                            .size(font_size::TINY)
                            .color(colors::TEXT_MUTED),
                    );
                });
            });

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{:.16}...", &block.block_hash_hex))
                        .size(font_size::TINY)
                        .color(colors::TEXT_MUTED),
                );
            });

            if block.tx_nullifiers.is_empty() {
                ui.label(
                    egui::RichText::new("  (empty block)")
                        .size(font_size::TINY)
                        .color(colors::TEXT_MUTED)
                        .italics(),
                );
            } else {
                for nf in &block.tx_nullifiers {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("  tx ")
                                .size(font_size::TINY)
                                .color(colors::TEXT_MUTED),
                        );
                        ui.label(
                            egui::RichText::new(nf)
                                .size(font_size::TINY)
                                .color(colors::TEXT_SECONDARY),
                        );
                    });
                }
            }
        });
}

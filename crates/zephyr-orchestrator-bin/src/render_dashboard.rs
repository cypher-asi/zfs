use std::collections::VecDeque;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::app::OrchestratorApp;
use crate::components::tokens::{self, colors, font_size, spacing};
use crate::components::labels::field_label;
use crate::components::{section, section_heading_with_right};
use crate::helpers::{fmt_float_comma, fmt_int_comma, format_uptime, node_color};
use crate::state::{AppState, RecentBlock};

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
            render_activity_feed(app, ui, state);
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
                ui.label(fmt_int_comma(state.network.certificates_produced));
                field_label(ui, "Spends Processed");
                ui.label(fmt_int_comma(state.network.spends_processed));
                field_label(ui, "Actual TPS");
                ui.label(
                    egui::RichText::new(fmt_float_comma(state.network.actual_tps, 0))
                        .color(colors::ACCENT)
                        .strong(),
                );
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
            let slider = egui::Slider::new(&mut rate, 0.1..=1_000_000.0)
                .logarithmic(true)
                .text("tx/s");
            if ui.add(slider).changed() {
                app.traffic_rate = rate;
                app.sync_traffic_to_shared();
            }

            ui.add_space(spacing::LG);

            ui.label(
                egui::RichText::new(format!("Actual: {} tx/s", fmt_float_comma(_state.network.actual_tps, 0)))
                    .size(font_size::SMALL)
                    .color(colors::ACCENT)
                    .strong(),
            );

            if _state.traffic_stats.backpressure_active {
                ui.add_space(spacing::LG);
                ui.label(
                    egui::RichText::new(format!(
                        "{} BACKPRESSURE",
                        egui_phosphor::regular::WARNING
                    ))
                    .size(font_size::SMALL)
                    .color(egui::Color32::from_rgb(255, 165, 0))
                    .strong(),
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                const PRESETS: [f32; 10] = [1_000.0, 2_500.0, 5_000.0, 10_000.0, 15_000.0, 20_000.0, 50_000.0, 100_000.0, 500_000.0, 1_000_000.0];
                for &preset in PRESETS.iter().rev() {
                    let label = if preset == 2_500.0 {
                        "2.5k".to_string()
                    } else if preset >= 1_000_000.0 {
                        format!("{}m", preset as u32 / 1_000_000)
                    } else if preset >= 1_000.0 {
                        format!("{}k", preset as u32 / 1_000)
                    } else {
                        format!("{}", preset as u32)
                    };
                    let active = (app.traffic_rate - preset).abs() < 0.5;
                    let size = egui::vec2(tokens::WIDGET_HEIGHT, tokens::WIDGET_HEIGHT);

                    let wv = &mut ui.visuals_mut().widgets;
                    let saved = (
                        wv.inactive.weak_bg_fill,
                        wv.hovered.weak_bg_fill,
                        wv.active.weak_bg_fill,
                    );
                    wv.inactive.weak_bg_fill =
                        if active { colors::SURFACE_RAISED } else { egui::Color32::BLACK };
                    wv.hovered.weak_bg_fill = colors::SURFACE_INTERACTIVE;
                    wv.active.weak_bg_fill = colors::BORDER;

                    let btn = egui::Button::new(
                        egui::RichText::new(&label)
                            .size(font_size::TINY)
                            .color(if active { colors::ACCENT } else { egui::Color32::WHITE }),
                    )
                    .stroke(tokens::default_stroke())
                    .corner_radius(0.0)
                    .min_size(size);

                    if ui.add(btn).clicked() {
                        app.traffic_rate = preset;
                        app.sync_traffic_to_shared();
                    }

                    let wv = &mut ui.visuals_mut().widgets;
                    wv.inactive.weak_bg_fill = saved.0;
                    wv.hovered.weak_bg_fill = saved.1;
                    wv.active.weak_bg_fill = saved.2;
                }
            });
        });

        ui.add_space(spacing::SM);

        ui.horizontal(|ui| {
            let tps_per_zone =
                app.max_block_size as f64 * (1000.0 / app.round_interval_ms as f64);
            let total_zones = _state.network.total_zones.max(1);
            let total_tps = tps_per_zone * total_zones as f64;

            ui.label(
                egui::RichText::new(format!(
                    "Block size: {}  |  Round: {} ms  |  Ceiling: {} tx/s ({}/zone \u{00d7} {} zones)",
                    fmt_int_comma(app.max_block_size as u64),
                    fmt_int_comma(app.round_interval_ms),
                    fmt_float_comma(total_tps, 0),
                    fmt_float_comma(tps_per_zone, 0),
                    total_zones,
                ))
                .size(font_size::SMALL)
                .color(colors::TEXT_MUTED),
            );
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

    let height = state.network.zone_heights.get(&zone_id).copied().unwrap_or(0);
    painter.text(
        egui::pos2(inner.right(), inner.top()),
        egui::Align2::RIGHT_TOP,
        format!("#{height}"),
        egui::FontId::proportional(font_size::SUBTITLE),
        if height > 0 { colors::ACCENT } else { colors::TEXT_MUTED },
    );

    let consecutive_timeouts = state.network.zone_consecutive_timeouts.get(&zone_id).copied().unwrap_or(0);
    if consecutive_timeouts > 0 {
        let stall_color = egui::Color32::from_rgb(255, 165, 0);
        painter.text(
            egui::pos2(inner.right(), inner.top() + 18.0),
            egui::Align2::RIGHT_TOP,
            format!("stall \u{00d7}{consecutive_timeouts}"),
            egui::FontId::proportional(font_size::SMALL),
            stall_color,
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

    let node_count = state
        .nodes
        .iter()
        .filter(|ns| ns.mempool_sizes.contains_key(&zone_id))
        .count()
        .max(1);
    let mempool_total: usize = state
        .nodes
        .iter()
        .filter_map(|ns| ns.mempool_sizes.get(&zone_id))
        .sum();
    let mempool_avg = mempool_total / node_count;
    painter.text(
        egui::pos2(inner.left(), inner.bottom() - 14.0),
        egui::Align2::LEFT_TOP,
        format!("mempool: {mempool_avg}"),
        egui::FontId::proportional(font_size::SMALL),
        colors::TEXT_SECONDARY,
    );
}

fn render_epoch_timeline(ui: &mut egui::Ui, state: &AppState) {
    let pct = state.network.epoch_progress_pct.clamp(0.0, 1.0);
    let subtitle = format!(
        "Epoch {} \u{2014} {:.0}%",
        state.network.current_epoch,
        pct * 100.0,
    );

    let max = ui.available_rect_before_wrap();
    let prev_clip = ui.clip_rect();
    ui.set_clip_rect(prev_clip.intersect(egui::Rect::from_x_y_ranges(
        max.left()..=max.right(),
        prev_clip.top()..=prev_clip.bottom(),
    )));

    let mut prepared = egui::Frame::default()
        .fill(colors::SURFACE)
        .corner_radius(0.0)
        .inner_margin(spacing::XL)
        .outer_margin(egui::Margin::symmetric(1, 0))
        .stroke(tokens::border_stroke())
        .begin(ui);

    {
        let ui = &mut prepared.content_ui;
        ui.set_width(ui.available_width());
        section_heading_with_right(ui, "Epoch Timeline", &subtitle);
        ui.add_space(10.0);

        let avail = ui.available_width();
        let bar_h = 6.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(avail, bar_h), egui::Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 0.0, colors::SURFACE_DARK);

        let fill_w = rect.width() * pct;
        let fill_rect = egui::Rect::from_min_size(rect.left_top(), egui::vec2(fill_w, bar_h));
        painter.rect_filled(fill_rect, 0.0, colors::ACCENT.linear_multiply(0.6));
    }

    let resp = prepared.end(ui);

    let border_rect = egui::Rect::from_min_max(
        egui::pos2(max.left() + 1.0, resp.rect.top()),
        egui::pos2(max.right() - 1.0, resp.rect.bottom()),
    );
    ui.painter().rect_stroke(
        border_rect,
        0.0,
        tokens::border_stroke(),
        egui::StrokeKind::Inside,
    );

    ui.set_clip_rect(prev_clip);
    ui.add_space(spacing::MD);
}

/// Delay before auto-resuming after the pointer leaves the feed area,
/// so brief pointer-out during scrolling doesn't flicker.
const FEED_UNPAUSE_DELAY: Duration = Duration::from_secs(1);

fn render_activity_feed(app: &mut OrchestratorApp, ui: &mut egui::Ui, state: &AppState) {
    if state.network.total_zones == 0 {
        return;
    }

    section(ui, "Activity Feed", |ui| {
        if app.activity_feed_paused {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(egui_phosphor::regular::PAUSE)
                        .size(font_size::SMALL)
                        .color(colors::ACCENT),
                );
                ui.label(
                    egui::RichText::new("Paused \u{2014} move cursor away to resume")
                        .size(font_size::TINY)
                        .color(colors::TEXT_MUTED),
                );
            });
            ui.add_space(spacing::XS);
        }

        let avail_w = ui.available_width();
        let col_w =
            (avail_w / state.network.total_zones.max(1) as f32 - spacing::SM).clamp(180.0, 320.0);

        let feed_rect = {
            let blocks: &VecDeque<RecentBlock> = if app.activity_feed_paused {
                &app.frozen_blocks
            } else {
                &state.recent_blocks
            };

            let inner = ui.horizontal_top(|ui| {
                for zone_id in 0..state.network.total_zones {
                    render_zone_activity_column(ui, zone_id, blocks, col_w, state);
                    if zone_id + 1 < state.network.total_zones {
                        ui.add_space(spacing::SM);
                    }
                }
            });
            inner.response.rect
        };

        let pointer_in_feed = ui.input(|i| {
            i.pointer
                .hover_pos()
                .map_or(false, |pos| feed_rect.contains(pos))
        });

        if pointer_in_feed {
            if !app.activity_feed_paused {
                app.activity_feed_paused = true;
                app.frozen_blocks = state.recent_blocks.clone();
            }
            app.feed_hover_lost = None;
        } else if app.activity_feed_paused {
            let lost = app.feed_hover_lost.get_or_insert_with(Instant::now);
            if lost.elapsed() >= FEED_UNPAUSE_DELAY {
                app.activity_feed_paused = false;
                app.frozen_blocks.clear();
                app.feed_hover_lost = None;
            }
        }
    });
}

fn render_zone_activity_column(
    ui: &mut egui::Ui,
    zone_id: u32,
    blocks: &VecDeque<RecentBlock>,
    col_w: f32,
    state: &AppState,
) {
    ui.vertical(|ui| {
        ui.set_width(col_w);

        let height = state.network.zone_heights.get(&zone_id).copied().unwrap_or(0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("ZONE {zone_id}"))
                    .size(font_size::SUBTITLE)
                    .color(colors::TEXT_HEADING),
            );
            ui.label(
                egui::RichText::new(format!("#{height}"))
                    .size(font_size::SMALL)
                    .color(if height > 0 { colors::ACCENT } else { colors::TEXT_MUTED }),
            );
        });

        ui.add_space(spacing::XS);

        egui::Frame::NONE
            .inner_margin(egui::Margin::same(spacing::SM as i8))
            .fill(colors::SURFACE_DARK)
            .stroke(egui::Stroke::new(tokens::STROKE_DEFAULT, colors::BORDER))
            .corner_radius(0.0)
            .show(ui, |ui| {
                ui.set_width(col_w - spacing::SM * 2.0);

                let zone_blocks: Vec<_> = blocks
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

            if block.tx_count == 0 {
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
                let hidden = block.tx_count.saturating_sub(block.tx_nullifiers.len());
                if hidden > 0 {
                    ui.label(
                        egui::RichText::new(format!("  +{hidden} more"))
                            .size(font_size::TINY)
                            .color(colors::TEXT_MUTED)
                            .italics(),
                    );
                }
            }
        });
}

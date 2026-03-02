use std::time::Instant;

use eframe::egui;

use crate::app::OrchestratorApp;
use crate::components::tokens::{self, colors, font_size, spacing};
use crate::components::{info_grid, kv_row, section};
use crate::helpers::{format_uptime, node_color};
use crate::state::AppState;

pub(crate) fn render_dashboard(app: &OrchestratorApp, ui: &mut egui::Ui, state: &AppState) {
    egui::ScrollArea::vertical()
        .id_salt("dashboard_scroll")
        .show(ui, |ui| {
            render_stats_bar(ui, state, app.launch_instant);
            ui.add_space(spacing::MD);
            render_zone_grid(ui, state);
            ui.add_space(spacing::MD);
            render_epoch_timeline(ui, state);
        });
}

fn render_stats_bar(ui: &mut egui::Ui, state: &AppState, launch_instant: Option<Instant>) {
    section(ui, "Network Overview", |ui| {
        info_grid(ui, "dash_stats", |ui| {
            kv_row(ui, "Epoch", &format!("{}", state.network.current_epoch));
            kv_row(ui, "Zones", &format!("{}", state.network.total_zones));
            kv_row(ui, "Active Validators", &format!("{}", state.nodes.len()));
            kv_row(
                ui,
                "Connected Peers",
                &format!("{}", state.network.total_peers),
            );
            kv_row(
                ui,
                "Certificates",
                &format!("{}", state.network.certificates_produced),
            );
            kv_row(
                ui,
                "Spends Processed",
                &format!("{}", state.network.spends_processed),
            );
            if let Some(started) = launch_instant {
                kv_row(ui, "Uptime", &format_uptime(started.elapsed().as_secs()));
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

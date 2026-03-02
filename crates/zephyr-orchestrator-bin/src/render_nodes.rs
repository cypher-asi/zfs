use eframe::egui;

use crate::components::section;
use crate::components::tokens::{self, colors, font_size, spacing};
use crate::helpers::{node_color, shorten_zid};
use crate::state::AppState;

const CARD_WIDTH: f32 = 260.0;
const CARD_HEIGHT: f32 = 140.0;

pub(crate) fn render_nodes(ui: &mut egui::Ui, state: &AppState) {
    egui::ScrollArea::vertical()
        .id_salt("nodes_scroll")
        .show(ui, |ui| {
            section(ui, "Validators", |ui| {
                if state.nodes.is_empty() {
                    ui.label("No nodes running.");
                    return;
                }

                let avail_w = ui.available_width();
                let cols = ((avail_w + spacing::MD) / (CARD_WIDTH + spacing::MD))
                    .floor()
                    .max(1.0) as usize;

                for row in state.nodes.chunks(cols) {
                    ui.horizontal(|ui| {
                        for ns in row {
                            render_node_card(ui, ns);
                            ui.add_space(spacing::MD);
                        }
                    });
                    ui.add_space(spacing::MD);
                }
            });
        });
}

fn render_node_card(ui: &mut egui::Ui, ns: &crate::state::NodeState) {
    let (rect, _resp) =
        ui.allocate_exact_size(egui::vec2(CARD_WIDTH, CARD_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let node_col = node_color(ns.node_id);
    painter.rect(
        rect,
        0.0,
        colors::SURFACE_DARK,
        egui::Stroke::new(tokens::STROKE_DEFAULT, colors::BORDER),
        egui::StrokeKind::Inside,
    );

    let pad = spacing::LG;
    let inner = rect.shrink(pad);

    let label = if ns.zode_id.is_empty() {
        format!("Node {}", ns.node_id)
    } else {
        shorten_zid(&ns.zode_id, 6)
    };

    painter.text(
        inner.left_top(),
        egui::Align2::LEFT_TOP,
        &label,
        egui::FontId::proportional(font_size::SUBTITLE),
        colors::TEXT_HEADING,
    );

    painter.circle_filled(
        egui::pos2(inner.right() - 6.0, inner.top() + 6.0),
        5.0,
        node_col,
    );

    let is_connected = ns.status.as_ref().is_some_and(|s| s.peer_count > 0);
    let status_label = if is_connected {
        "CONNECTED"
    } else {
        "STARTING"
    };
    let status_color = if is_connected {
        colors::CONNECTED
    } else {
        colors::WARN
    };

    painter.text(
        egui::pos2(inner.left(), inner.top() + 20.0),
        egui::Align2::LEFT_TOP,
        status_label,
        egui::FontId::proportional(font_size::SMALL),
        status_color,
    );

    let peer_count = ns.status.as_ref().map(|s| s.peer_count).unwrap_or(0);
    painter.text(
        egui::pos2(inner.left(), inner.top() + 36.0),
        egui::Align2::LEFT_TOP,
        format!("Peers: {peer_count}"),
        egui::FontId::proportional(font_size::ACTION),
        colors::TEXT_SECONDARY,
    );

    if !ns.assigned_zones.is_empty() {
        let mut chip_x = inner.left();
        let chip_y = inner.top() + 56.0;
        for &zone_id in &ns.assigned_zones {
            let label = format!("Z{zone_id}");
            let galley = painter.layout_no_wrap(
                label,
                egui::FontId::proportional(font_size::SMALL),
                egui::Color32::WHITE,
            );
            let chip_w = galley.size().x + 8.0;
            let chip_rect =
                egui::Rect::from_min_size(egui::pos2(chip_x, chip_y), egui::vec2(chip_w, 16.0));
            painter.rect_filled(chip_rect, 2.0, node_col.linear_multiply(0.3));
            painter.galley(
                egui::pos2(chip_x + 4.0, chip_y + 1.0),
                galley,
                egui::Color32::WHITE,
            );
            chip_x += chip_w + 4.0;
            if chip_x > inner.right() - 20.0 {
                break;
            }
        }
    }

    let mempool_total: usize = ns.mempool_sizes.values().sum();
    painter.text(
        egui::pos2(inner.left(), inner.bottom() - 14.0),
        egui::Align2::LEFT_TOP,
        format!("Mempool: {mempool_total}"),
        egui::FontId::proportional(font_size::SMALL),
        colors::TEXT_SECONDARY,
    );
}

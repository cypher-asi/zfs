use eframe::egui;

use crate::app::ZodeApp;
use crate::components::{colors, loading_state, muted_label, section};
use crate::components::tokens::{self, font_size, spacing};

const CARD_WIDTH: f32 = 260.0;
const CARD_HEIGHT: f32 = 100.0;
const CHECKBOX_SIZE: f32 = 18.0;

pub(crate) fn render_services(app: &ZodeApp, ui: &mut egui::Ui) {
    let Some(ref zode) = app.zode else {
        loading_state(ui);
        return;
    };

    section(ui, "Factory Services", |ui| {
        let registry = zode.service_registry();
        let Ok(registry) = registry.try_lock() else {
            muted_label(ui, "Loading services…");
            return;
        };

        let services = registry.list_services();
        if services.is_empty() {
            muted_label(ui, "No services registered.");
            ui.add_space(spacing::SM);
            muted_label(ui, "Services will appear here when enabled.");
            return;
        }

        let avail_w = ui.available_width();
        let cols = ((avail_w + spacing::MD) / (CARD_WIDTH + spacing::MD))
            .floor()
            .max(1.0) as usize;

        for row in services.chunks(cols) {
            ui.horizontal(|ui| {
                for svc in row {
                    service_card(ui, svc);
                    ui.add_space(spacing::MD);
                }
            });
            ui.add_space(spacing::MD);
        }
    });
}

fn service_card(ui: &mut egui::Ui, svc: &grid_service::ServiceInfo) {
    let id_hex = svc.id.to_hex();
    let short_id = &id_hex[..8.min(id_hex.len())];

    let border_color = if svc.running {
        colors::CONNECTED
    } else {
        colors::BORDER
    };

    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(CARD_WIDTH, CARD_HEIGHT),
        egui::Sense::click(),
    );

    let painter = ui.painter_at(rect);

    painter.rect(
        rect,
        0.0,
        colors::SURFACE_DARK,
        egui::Stroke::new(tokens::STROKE_DEFAULT, border_color),
        egui::StrokeKind::Inside,
    );

    if resp.hovered() {
        painter.rect(
            rect,
            0.0,
            egui::Color32::from_white_alpha(4),
            egui::Stroke::NONE,
            egui::StrokeKind::Inside,
        );
    }

    let pad = spacing::LG;
    let inner = rect.shrink(pad);

    let cb_rect = egui::Rect::from_min_size(
        egui::pos2(inner.right() - CHECKBOX_SIZE, inner.top()),
        egui::vec2(CHECKBOX_SIZE, CHECKBOX_SIZE),
    );

    let cb_border = if svc.running {
        colors::CONNECTED
    } else {
        colors::BORDER_SUBTLE
    };
    painter.rect(
        cb_rect,
        2.0,
        if svc.running {
            colors::CONNECTED
        } else {
            egui::Color32::TRANSPARENT
        },
        egui::Stroke::new(tokens::STROKE_DEFAULT, cb_border),
        egui::StrokeKind::Inside,
    );

    if svc.running {
        let cx = cb_rect.center().x;
        let cy = cb_rect.center().y;
        let points = [
            egui::pos2(cx - 4.0, cy),
            egui::pos2(cx - 1.0, cy + 3.0),
            egui::pos2(cx + 4.5, cy - 3.5),
        ];
        painter.line_segment(
            [points[0], points[1]],
            egui::Stroke::new(2.0, colors::SURFACE_DARK),
        );
        painter.line_segment(
            [points[1], points[2]],
            egui::Stroke::new(2.0, colors::SURFACE_DARK),
        );
    }

    let name_rect = egui::Rect::from_min_max(
        inner.left_top(),
        egui::pos2(cb_rect.left() - spacing::SM, inner.top() + 16.0),
    );
    painter.text(
        name_rect.left_top(),
        egui::Align2::LEFT_TOP,
        &svc.descriptor.name,
        egui::FontId::proportional(font_size::SUBTITLE),
        colors::TEXT_HEADING,
    );

    painter.text(
        egui::pos2(inner.left(), inner.top() + 20.0),
        egui::Align2::LEFT_TOP,
        format!("v{}", svc.descriptor.version),
        egui::FontId::proportional(font_size::SMALL),
        colors::TEXT_MUTED,
    );

    let required = svc.descriptor.required_programs.len();
    let owned = svc.descriptor.owned_programs.len();
    let detail = format!("{short_id}…  ·  {required} req, {owned} owned");
    painter.text(
        egui::pos2(inner.left(), inner.bottom() - 14.0),
        egui::Align2::LEFT_TOP,
        detail,
        egui::FontId::proportional(font_size::SMALL),
        colors::TEXT_SECONDARY,
    );

    let status_text = if svc.running { "RUNNING" } else { "STOPPED" };
    let status_color = if svc.running {
        colors::CONNECTED
    } else {
        colors::TEXT_MUTED
    };
    painter.text(
        egui::pos2(inner.right(), inner.bottom() - 14.0),
        egui::Align2::RIGHT_TOP,
        status_text,
        egui::FontId::proportional(font_size::SMALL),
        status_color,
    );
}

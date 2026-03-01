use eframe::egui;

use super::tokens::colors;

/// Centered spinner + "Loading…" label, vertically positioned in the middle.
pub(crate) fn loading_state(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        let avail = ui.available_height();
        ui.add_space((avail / 2.0 - 20.0).max(0.0));
        ui.spinner();
        ui.label("Loading...");
    });
}

/// Checkmark icon for verified signature status.
pub(crate) fn verified_icon(ui: &mut egui::Ui) {
    let size = 14.0;
    let (resp, painter) = ui.allocate_painter(egui::Vec2::splat(size), egui::Sense::hover());
    let c = resp.rect.center();
    painter.add(egui::Shape::line(
        vec![
            c + egui::vec2(-3.5, 0.5),
            c + egui::vec2(-1.0, 3.0),
            c + egui::vec2(4.5, -3.5),
        ],
        egui::Stroke::new(2.0, colors::CONNECTED),
    ));
}

/// X icon for failed signature status.
pub(crate) fn failed_icon(ui: &mut egui::Ui) {
    let size = 14.0;
    let (resp, painter) = ui.allocate_painter(egui::Vec2::splat(size), egui::Sense::hover());
    let c = resp.rect.center();
    let stroke = egui::Stroke::new(2.0, colors::ERROR);
    painter.line_segment(
        [c + egui::vec2(-3.0, -3.0), c + egui::vec2(3.0, 3.0)],
        stroke,
    );
    painter.line_segment(
        [c + egui::vec2(3.0, -3.0), c + egui::vec2(-3.0, 3.0)],
        stroke,
    );
}

/// Colored dot indicator for connection status.
pub(crate) fn status_dot(ui: &mut egui::Ui, connected: bool) {
    let color = if connected {
        colors::CONNECTED
    } else {
        colors::DISCONNECTED
    };
    let label = if connected { "connected" } else { "stopped" };
    ui.monospace(egui::RichText::new(label).color(color));

    let dot_radius = 3.5;
    let (dot_rect, _) = ui.allocate_exact_size(
        egui::vec2(dot_radius * 2.0 + 2.0, dot_radius * 2.0),
        egui::Sense::hover(),
    );
    ui.painter().circle_filled(dot_rect.center(), dot_radius, color);
}

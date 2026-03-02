use eframe::egui;

use super::tokens::colors;

pub(crate) fn loading_state(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        let avail = ui.available_height();
        ui.add_space((avail / 2.0 - 20.0).max(0.0));
        ui.spinner();
        ui.label("Loading...");
    });
}

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
    ui.painter()
        .circle_filled(dot_rect.center(), dot_radius, color);
}

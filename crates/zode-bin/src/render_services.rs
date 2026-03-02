use eframe::egui;

use crate::app::ZodeApp;
use crate::components::{colors, info_grid, kv_row, loading_state, muted_label, section};
use crate::components::tokens::spacing;

pub(crate) fn render_services(app: &ZodeApp, ui: &mut egui::Ui) {
    let Some(ref zode) = app.zode else {
        loading_state(ui);
        return;
    };

    section(ui, "Services", |ui| {
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

        for svc in &services {
            let id_hex = svc.id.to_hex();
            let short_id = &id_hex[..8.min(id_hex.len())];

            ui.horizontal(|ui| {
                let status_color = if svc.running {
                    colors::ACCENT
                } else {
                    colors::TEXT_MUTED
                };
                let status_label = if svc.running { "Running" } else { "Stopped" };

                ui.colored_label(
                    colors::TEXT_HEADING,
                    egui::RichText::new(&svc.descriptor.name).strong(),
                );
                ui.label(
                    egui::RichText::new(format!("v{}", svc.descriptor.version))
                        .color(colors::TEXT_MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(status_color, format!("● {status_label}"));
                });
            });

            let grid_id: &str = &format!("svc_info_{short_id}");
            info_grid(ui, grid_id, |ui| {
                kv_row(ui, "Service ID", &format!("{short_id}…"));

                let required = svc.descriptor.required_programs.len();
                let owned = svc.descriptor.owned_programs.len();
                kv_row(
                    ui,
                    "Programs",
                    &format!("{required} required, {owned} owned"),
                );
            });

            if !svc.descriptor.required_programs.is_empty()
                || !svc.descriptor.owned_programs.is_empty()
            {
                ui.indent(format!("svc_programs_{short_id}"), |ui| {
                    for pid in &svc.descriptor.required_programs {
                        let pid_hex = pid.to_hex();
                        let short = &pid_hex[..8.min(pid_hex.len())];
                        muted_label(ui, &format!("↳ required: {short}…"));
                    }
                    for desc in &svc.descriptor.owned_programs {
                        muted_label(
                            ui,
                            &format!("↳ owned: {} v{}", desc.name, desc.version),
                        );
                    }
                });
            }

            ui.add_space(spacing::MD);
            ui.separator();
            ui.add_space(spacing::SM);
        }
    });
}

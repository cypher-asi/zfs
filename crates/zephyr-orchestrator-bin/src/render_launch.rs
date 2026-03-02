use eframe::egui;

use crate::app::OrchestratorApp;
use crate::components::action_button;
use crate::components::tokens::{self, colors, font_size, spacing};
use crate::state::NetworkPreset;

const CARD_WIDTH: f32 = 260.0;
const CARD_HEIGHT: f32 = 130.0;

pub(crate) fn render_launch_screen(app: &mut OrchestratorApp, ui: &mut egui::Ui) {
    let panel = ui.max_rect();
    let col_w = 600.0_f32.min(panel.width());
    let col = egui::Rect::from_center_size(panel.center(), egui::vec2(col_w, panel.height()));

    ui.scope_builder(egui::UiBuilder::new().max_rect(col), |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(((panel.height() - 480.0) / 2.0).max(20.0));

            if let Some(ref tex) = app.icon_texture {
                ui.add(
                    egui::Image::new(tex)
                        .fit_to_exact_size(egui::vec2(56.0, 56.0))
                        .corner_radius(8.0),
                );
                ui.add_space(spacing::XL);
            }

            ui.label(
                egui::RichText::new("ZEPHYR ORCHESTRATOR")
                    .strong()
                    .size(font_size::SUBTITLE)
                    .color(colors::TEXT_HEADING),
            );
            ui.add_space(spacing::SM);
            ui.label(
                egui::RichText::new("Launch a local Zephyr network for testing")
                    .size(font_size::ACTION)
                    .color(colors::TEXT_SECONDARY),
            );
            ui.add_space(spacing::XXXL);

            let presets = [
                NetworkPreset::Minimal,
                NetworkPreset::Standard,
                NetworkPreset::Large,
                NetworkPreset::Custom {
                    validators: 4,
                    zones: 3,
                    committee_size: 3,
                },
            ];

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                for preset in &presets {
                    if preset_card(ui, preset, &app.selected_preset) {
                        app.selected_preset = preset.clone();
                    }
                }
            });

            ui.add_space(spacing::MD);

            if matches!(app.selected_preset, NetworkPreset::Custom { .. }) {
                render_custom_inputs(ui, &mut app.selected_preset);
            } else {
                ui.allocate_space(egui::vec2(0.0, ui.spacing().interact_size.y));
            }
            ui.add_space(spacing::MD);

            ui.add_space(spacing::XXL);

            if app.launching {
                ui.spinner();
                ui.label("Launching network...");
            } else if action_button(ui, "Launch Network") {
                app.do_launch();
            }

            if let Some(ref err) = app.launch_error {
                ui.add_space(spacing::SM);
                ui.colored_label(colors::ERROR, err.as_str());
            }
        });
    });
}

fn preset_card(ui: &mut egui::Ui, preset: &NetworkPreset, selected: &NetworkPreset) -> bool {
    let is_selected = std::mem::discriminant(preset) == std::mem::discriminant(selected);
    let border_color = if is_selected {
        egui::Color32::WHITE
    } else {
        colors::BORDER
    };

    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(CARD_WIDTH, CARD_HEIGHT), egui::Sense::click());

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

    painter.text(
        inner.left_top(),
        egui::Align2::LEFT_TOP,
        preset.label(),
        egui::FontId::proportional(font_size::SUBTITLE),
        colors::TEXT_HEADING,
    );

    painter.text(
        egui::pos2(inner.left(), inner.top() + 22.0),
        egui::Align2::LEFT_TOP,
        preset.description(),
        egui::FontId::proportional(font_size::ACTION),
        colors::TEXT_SECONDARY,
    );

    painter.text(
        egui::pos2(inner.left(), inner.top() + 44.0),
        egui::Align2::LEFT_TOP,
        format!("Committee size: {}", preset.committee_size()),
        egui::FontId::proportional(font_size::SMALL),
        colors::TEXT_MUTED,
    );

    painter.text(
        egui::pos2(inner.left(), inner.bottom() - 14.0),
        egui::Align2::LEFT_TOP,
        format!(
            "{} nodes  \u{00B7}  {} zones",
            preset.validators(),
            preset.zones()
        ),
        egui::FontId::proportional(font_size::SMALL),
        colors::TEXT_SECONDARY,
    );

    resp.clicked()
}

fn render_custom_inputs(ui: &mut egui::Ui, preset: &mut NetworkPreset) {
    if let NetworkPreset::Custom {
        validators,
        zones,
        committee_size,
    } = preset
    {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spacing::SM;

            ui.label("VALIDATORS");
            ui.add(egui::DragValue::new(validators).range(2..=20));

            ui.add_space(spacing::MD);
            ui.label("ZONES");
            let mut z_val = *zones as i32;
            ui.add(egui::DragValue::new(&mut z_val).range(1..=16));
            *zones = z_val.max(1) as u32;

            ui.add_space(spacing::MD);
            ui.label("COMMITTEE SIZE");
            ui.add(egui::DragValue::new(committee_size).range(1..=*validators));
        });
    }
}

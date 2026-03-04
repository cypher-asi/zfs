use eframe::egui;

use crate::app::OrchestratorApp;
use crate::components::action_button;
use crate::components::tokens::{self, colors, font_size, spacing};
use crate::helpers::fmt_float_comma;
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

            render_throughput_inputs(ui, &mut app.max_block_size, &mut app.round_interval_ms);
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

fn render_throughput_inputs(ui: &mut egui::Ui, max_block_size: &mut usize, round_interval_ms: &mut u64) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing::SM;

        ui.label(
            egui::RichText::new("MAX BLOCK SIZE")
                .size(font_size::SMALL)
                .color(colors::TEXT_SECONDARY),
        );
        let mut bs = *max_block_size as i32;
        int_input(ui, "max_block_size", &mut bs, 1..=4096);
        *max_block_size = bs.max(1) as usize;

        ui.add_space(spacing::MD);

        ui.label(
            egui::RichText::new("ROUND INTERVAL (ms)")
                .size(font_size::SMALL)
                .color(colors::TEXT_SECONDARY),
        );
        let mut ri = *round_interval_ms as i32;
        int_input(ui, "round_interval_ms", &mut ri, 10..=5000);
        *round_interval_ms = ri.max(10) as u64;

        ui.add_space(spacing::MD);

        let tps_per_zone = *max_block_size as f64 * (1000.0 / *round_interval_ms as f64);
        ui.label(
            egui::RichText::new(format!("{} tx/s per zone", fmt_float_comma(tps_per_zone, 0)))
                .size(font_size::SMALL)
                .color(colors::ACCENT),
        );
    });
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
            let mut v = *validators as i32;
            int_input(ui, "validators", &mut v, 2..=100);
            *validators = v.max(2) as usize;

            ui.add_space(spacing::MD);
            ui.label("ZONES");
            let mut z = *zones as i32;
            int_input(ui, "zones", &mut z, 1..=16);
            *zones = z.max(1) as u32;

            ui.add_space(spacing::MD);
            ui.label("COMMITTEE SIZE");
            let mut c = *committee_size as i32;
            int_input(ui, "committee_size", &mut c, 1..=(*validators as i32));
            *committee_size = c.max(1) as usize;
        });
    }
}

fn int_input(ui: &mut egui::Ui, salt: &str, value: &mut i32, range: std::ops::RangeInclusive<i32>) {
    let id = ui.id().with(salt);
    let mut buf: String = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| value.to_string());

    let resp = ui.add(
        egui::TextEdit::singleline(&mut buf)
            .desired_width(44.0)
            .horizontal_align(egui::Align::Center),
    );

    if let Ok(n) = buf.parse::<i32>() {
        *value = n.clamp(*range.start(), *range.end());
    }

    if !resp.has_focus() {
        buf = value.to_string();
    }

    ui.data_mut(|d| d.insert_temp(id, buf));
}

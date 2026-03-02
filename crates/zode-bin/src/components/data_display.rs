use eframe::egui;

use super::buttons::{copy_button, square_icon_button};
use super::labels::field_label;
use super::tokens::{spacing, WIDGET_HEIGHT};

/// Two-column key–value grid with uniform spacing.
pub(crate) fn info_grid(ui: &mut egui::Ui, id: &str, add_rows: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([spacing::LG, spacing::XS])
        .show(ui, add_rows);
}

/// Simple text key-value row inside an `info_grid`.
pub(crate) fn kv_row(ui: &mut egui::Ui, key: &str, value: &str) {
    field_label(ui, key);
    ui.label(value);
    ui.end_row();
}

/// Key-value row with monospace text and a COPY button.
pub(crate) fn kv_row_copyable(ui: &mut egui::Ui, key: &str, value: &str) {
    field_label(ui, key);
    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(egui::RichText::new(value).monospace())
                .truncate()
                .wrap_mode(egui::TextWrapMode::Truncate),
        );
        copy_button(ui, value);
    });
    ui.end_row();
}

/// Stacked field: label on top, wrapping monospace value + copy button below.
/// Not grid-based, so the value gets the full available width to wrap within.
pub(crate) fn kv_stacked_copyable(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        field_label(ui, key);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            copy_button(ui, value);
        });
    });
    ui.add(
        egui::Label::new(egui::RichText::new(value).monospace())
            .wrap_mode(egui::TextWrapMode::Wrap),
    );
    ui.add_space(spacing::SM);
}

/// Text input + ADD button + removable monospace items.
pub(crate) fn editable_list(
    ui: &mut egui::Ui,
    items: &mut Vec<String>,
    input: &mut String,
    _input_width: f32,
) {
    let btn_w = WIDGET_HEIGHT + ui.spacing().item_spacing.x;

    ui.horizontal(|ui| {
        let w = (ui.available_width() - btn_w).max(80.0);
        let resp = ui.add(super::inputs::text_input(input, w));
        if (square_icon_button(ui, egui_phosphor::regular::PLUS)
            || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
            && !input.trim().is_empty()
        {
            items.push(input.trim().to_string());
            input.clear();
        }
    });

    let mut remove_idx = None;
    for (i, item) in items.iter().enumerate() {
        ui.horizontal(|ui| {
            let label_w = ui.available_width() - btn_w;
            ui.add_sized(
                [label_w, WIDGET_HEIGHT],
                egui::Label::new(egui::RichText::new(item).monospace()).truncate(),
            )
            .on_hover_text(item);
            if square_icon_button(ui, egui_phosphor::regular::X) {
                remove_idx = Some(i);
            }
        });
    }
    if let Some(idx) = remove_idx {
        items.remove(idx);
    }
}

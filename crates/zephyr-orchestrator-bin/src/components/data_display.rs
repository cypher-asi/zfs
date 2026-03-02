use eframe::egui;

use super::buttons::copy_button;
use super::labels::field_label;
use super::tokens::spacing;

pub(crate) fn info_grid(ui: &mut egui::Ui, id: &str, add_rows: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([spacing::LG, spacing::XS])
        .show(ui, add_rows);
}

pub(crate) fn kv_row(ui: &mut egui::Ui, key: &str, value: &str) {
    field_label(ui, key);
    ui.label(value);
    ui.end_row();
}

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

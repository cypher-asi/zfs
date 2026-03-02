use eframe::egui;

use super::tokens::{colors, spacing};

pub(crate) fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).strong());
}

pub(crate) fn hint_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak());
}

pub(crate) fn muted_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(colors::TEXT_MUTED));
}

pub(crate) fn error_label(ui: &mut egui::Ui, text: &str) {
    ui.colored_label(colors::ERROR, text);
    ui.add_space(spacing::SM);
}

pub(crate) fn status_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak().italics());
}

pub(crate) fn warn_label(ui: &mut egui::Ui, text: &str) {
    ui.colored_label(colors::WARN, text);
}

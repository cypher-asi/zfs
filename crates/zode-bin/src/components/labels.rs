use eframe::egui;

use super::tokens::{colors, spacing};

/// ALL-CAPS bold field label (e.g. grid keys).
pub(crate) fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).strong());
}

/// Subdued descriptive text shown beneath section headings.
pub(crate) fn hint_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak());
}

/// Subdued placeholder / empty-state text (uses TEXT_MUTED color).
pub(crate) fn muted_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(colors::TEXT_MUTED));
}

/// Red error message with spacing below.
pub(crate) fn error_label(ui: &mut egui::Ui, text: &str) {
    ui.colored_label(colors::ERROR, text);
    ui.add_space(spacing::SM);
}

/// Subdued status / info message (weak + italics).
pub(crate) fn status_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak().italics());
}

/// Warning-colored label.
pub(crate) fn warn_label(ui: &mut egui::Ui, text: &str) {
    ui.colored_label(colors::WARN, text);
}

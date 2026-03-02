use eframe::egui;

use super::tokens::WIDGET_HEIGHT;

pub(crate) fn text_input(buf: &mut String, width: f32) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(buf)
        .desired_width(width)
        .margin(egui::Margin::symmetric(4, 0))
        .vertical_align(egui::Align::Center)
        .min_size(egui::vec2(0.0, WIDGET_HEIGHT))
}

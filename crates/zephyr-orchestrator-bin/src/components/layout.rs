use eframe::egui;

use super::tokens::{self, colors, font_size, spacing};

pub(crate) fn section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let max = ui.available_rect_before_wrap();

    let prev_clip = ui.clip_rect();
    ui.set_clip_rect(prev_clip.intersect(egui::Rect::from_x_y_ranges(
        max.left()..=max.right(),
        prev_clip.top()..=prev_clip.bottom(),
    )));

    let mut prepared = egui::Frame::default()
        .fill(colors::SURFACE)
        .corner_radius(0.0)
        .inner_margin(spacing::XL)
        .outer_margin(egui::Margin::symmetric(1, 0))
        .stroke(tokens::border_stroke())
        .begin(ui);

    {
        let ui = &mut prepared.content_ui;
        ui.set_width(ui.available_width());
        section_heading(ui, title);
        ui.add_space(10.0);
        add_contents(ui);
    }

    let resp = prepared.end(ui);

    let border_rect = egui::Rect::from_min_max(
        egui::pos2(max.left() + 1.0, resp.rect.top()),
        egui::pos2(max.right() - 1.0, resp.rect.bottom()),
    );
    ui.painter().rect_stroke(
        border_rect,
        0.0,
        tokens::border_stroke(),
        egui::StrokeKind::Inside,
    );

    ui.set_clip_rect(prev_clip);
    ui.add_space(spacing::MD);
}

pub(crate) fn section_heading(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .strong()
            .size(font_size::HEADING)
            .color(colors::TEXT_HEADING),
    );
}

pub(crate) fn action_panel(
    ui: &mut egui::Ui,
    id: impl Into<egui::Id>,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::TopBottomPanel::bottom(id)
        .frame(
            egui::Frame::default()
                .fill(colors::SURFACE)
                .inner_margin(egui::Margin::symmetric(16, 12))
                .stroke(tokens::border_stroke()),
        )
        .show_inside(ui, |ui| {
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                add_contents,
            );
        });
}

pub(crate) fn centered_row(
    ui: &mut egui::Ui,
    id_salt: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let id = ui.id().with(id_salt);
    let avail = ui.available_width();
    let prev_w: f32 = ui.ctx().data_mut(|d| d.get_temp(id).unwrap_or(avail));
    let offset = ((avail - prev_w) / 2.0).max(0.0);

    ui.horizontal(|ui| {
        ui.add_space(offset);
        let x0 = ui.cursor().left();
        add_contents(ui);
        let w = ui.cursor().left() - x0;
        ui.ctx().data_mut(|d| d.insert_temp(id, w));
    });
}

pub(crate) fn card_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(colors::SURFACE_DARK)
        .corner_radius(0.0)
        .inner_margin(spacing::MD)
        .stroke(tokens::border_stroke())
}

pub(crate) fn overlay_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(colors::VIZ_OVERLAY_BG)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(12, 6))
}

pub(crate) fn title_bar_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(colors::PANEL_BG)
        .inner_margin(egui::Margin::symmetric(
            spacing::LG as i8,
            spacing::MD as i8,
        ))
        .stroke(egui::Stroke::NONE)
}

pub(crate) fn status_bar_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(colors::PANEL_BG)
        .inner_margin(egui::Margin::symmetric(
            spacing::LG as i8,
            spacing::SM as i8,
        ))
        .stroke(egui::Stroke::NONE)
}

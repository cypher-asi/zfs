use eframe::egui;

use super::tokens::{self, colors, font_size, spacing};

/// Card-like container with an uppercased title.
pub(crate) fn section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let max = ui.max_rect();

    let mut prepared = egui::Frame::default()
        .fill(colors::SURFACE)
        .corner_radius(0.0)
        .inner_margin(spacing::XL)
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
        egui::pos2(max.left(), resp.rect.top()),
        egui::pos2(max.right(), resp.rect.bottom()),
    );
    ui.painter()
        .rect_stroke(border_rect, 0.0, tokens::border_stroke(), egui::StrokeKind::Inside);

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

/// Bottom action panel (right-aligned button bar).
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

/// Horizontal row whose content is centered within the available width.
///
/// Uses previous-frame measurement to compute the centering offset, so the
/// first frame may be uncentered (imperceptible in practice).
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

/// Reusable dark card frame (dark fill + border).
pub(crate) fn card_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(colors::SURFACE_DARK)
        .corner_radius(0.0)
        .inner_margin(spacing::MD)
        .stroke(tokens::border_stroke())
}

/// Semi-transparent overlay panel for visualization overlays.
pub(crate) fn overlay_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(colors::VIZ_OVERLAY_BG)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(12, 6))
}

/// Standardized 2-column form grid with `[12.0, 8.0]` spacing.
pub(crate) fn form_grid(
    ui: &mut egui::Ui,
    id: &str,
    add_rows: impl FnOnce(&mut egui::Ui),
) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([spacing::LG, spacing::MD])
        .show(ui, add_rows);
}

/// Centered auth / setup panel layout: icon + title, centered in the panel.
///
/// Call this inside a `CentralPanel` or similar full-screen container.
/// The closure receives the `Ui` positioned below the title.
pub(crate) fn auth_screen_panel(
    ui: &mut egui::Ui,
    icon_texture: &egui::TextureHandle,
    title: &str,
    content_height: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let panel = ui.max_rect();
    let col_w = 380.0_f32.min(panel.width());
    let col = egui::Rect::from_center_size(
        panel.center(),
        egui::vec2(col_w, panel.height()),
    );

    ui.scope_builder(egui::UiBuilder::new().max_rect(col), |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(((panel.height() - content_height) / 2.0).max(20.0));

            ui.add(
                egui::Image::new(icon_texture)
                    .fit_to_exact_size(egui::vec2(56.0, 56.0))
                    .corner_radius(8.0),
            );
            ui.add_space(spacing::XL);

            ui.label(
                egui::RichText::new(title)
                    .strong()
                    .size(font_size::SUBTITLE)
                    .color(colors::TEXT_HEADING),
            );
            ui.add_space(spacing::XL);

            add_contents(ui);
        });
    });
}

/// Standard frame for pre-auth / setup central panels.
pub(crate) fn auth_panel_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(colors::PANEL_BG)
        .inner_margin(spacing::XXXL)
}

/// Standard frame for the title bar panel.
pub(crate) fn title_bar_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(colors::PANEL_BG)
        .inner_margin(egui::Margin::symmetric(
            spacing::LG as i8,
            spacing::MD as i8,
        ))
        .stroke(egui::Stroke::NONE)
}

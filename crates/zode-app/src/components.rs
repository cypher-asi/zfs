use eframe::egui;

// ---------------------------------------------------------------------------
// Theme colors
// ---------------------------------------------------------------------------

pub(crate) mod colors {
    use eframe::egui::Color32;

    pub const SURFACE: Color32 = Color32::from_rgb(1, 1, 1);
    pub const SURFACE_DIM: Color32 = Color32::from_rgb(1, 1, 1);
    pub const BORDER: Color32 = Color32::from_rgb(48, 48, 52);
    pub const ERROR: Color32 = Color32::from_rgb(255, 80, 80);
    pub const WARN: Color32 = Color32::from_rgb(255, 200, 100);
    pub const CONNECTED: Color32 = Color32::from_rgb(46, 230, 176);
    pub const DISCONNECTED: Color32 = Color32::from_rgb(255, 80, 80);
}

// ---------------------------------------------------------------------------
// Section panel — card-like container with an uppercased title
// ---------------------------------------------------------------------------

pub(crate) fn section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::default()
        .fill(colors::SURFACE)
        .corner_radius(0.0)
        .inner_margin(16.0)
        .stroke(egui::Stroke::new(1.0, colors::BORDER))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            section_heading(ui, title);
            ui.add_space(10.0);
            add_contents(ui);
        });
    ui.add_space(8.0);
}

pub(crate) fn section_heading(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .strong()
            .size(10.0)
            .color(egui::Color32::from_rgb(140, 140, 145)),
    );
}

// ---------------------------------------------------------------------------
// Bottom action panel (right-aligned button bar)
// ---------------------------------------------------------------------------

pub(crate) fn action_panel(
    ui: &mut egui::Ui,
    id: impl Into<egui::Id>,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::TopBottomPanel::bottom(id)
        .frame(
            egui::Frame::default()
                .fill(colors::SURFACE_DIM)
                .inner_margin(egui::Margin::symmetric(16, 12))
                .stroke(egui::Stroke::new(1.0, colors::BORDER)),
        )
        .show_inside(ui, |ui| {
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                add_contents,
            );
        });
}

// ---------------------------------------------------------------------------
// Info grid — two-column key–value grid with uniform spacing
// ---------------------------------------------------------------------------

pub(crate) fn info_grid(ui: &mut egui::Ui, id: &str, add_rows: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([12.0, 2.0])
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
        ui.monospace(value);
        copy_button(ui, value);
    });
    ui.end_row();
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

const ICON_SIZE: f32 = 16.0;

pub(crate) const WIDGET_HEIGHT: f32 = 24.0;

pub(crate) fn text_input(buf: &mut String, width: f32) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(buf)
        .desired_width(width)
        .margin(egui::Margin::symmetric(4, 0))
        .vertical_align(egui::Align::Center)
        .min_size(egui::vec2(0.0, WIDGET_HEIGHT))
}

fn styled_button(ui: &mut egui::Ui, label: &str, padding: egui::Vec2, font_size: f32) -> bool {
    let old_padding = ui.spacing().button_padding;
    ui.spacing_mut().button_padding = padding;
    let r = ui.add(
        egui::Button::new(
            egui::RichText::new(label.to_uppercase())
                .color(egui::Color32::WHITE)
                .size(font_size),
        )
        .fill(egui::Color32::BLACK)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 55, 60)))
        .corner_radius(0.0)
        .min_size(egui::vec2(0.0, WIDGET_HEIGHT)),
    );
    ui.spacing_mut().button_padding = old_padding;
    r.clicked()
}

/// Standard button — black bg, white uppercase text, compact padding.
pub(crate) fn std_button(ui: &mut egui::Ui, label: &str) -> bool {
    styled_button(ui, label, egui::vec2(10.0, 4.0), 10.0)
}

/// Primary action button — uses standard style, slightly larger.
pub(crate) fn action_button(ui: &mut egui::Ui, label: &str) -> bool {
    styled_button(ui, label, egui::vec2(12.0, 5.0), 11.0)
}

/// Title-bar icon button (custom-painted for tight layout control).
pub(crate) fn title_bar_icon(ui: &mut egui::Ui, icon: &str, active: bool) -> egui::Response {
    let font_id = egui::FontId::proportional(16.0);
    let galley =
        ui.fonts_mut(|f| f.layout_no_wrap(icon.to_string(), font_id, egui::Color32::PLACEHOLDER));
    let bp = ui.spacing().button_padding;
    let desired = egui::vec2(galley.size().x + bp.x * 2.0, ui.spacing().interact_size.y);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let vis = ui.style().interact_selectable(&resp, active);
    if !active && resp.hovered() {
        ui.painter().rect_filled(rect, vis.corner_radius, vis.bg_fill);
    }
    let text_color = if active {
        egui::Color32::WHITE
    } else {
        vis.text_color()
    };
    let galley = ui.fonts_mut(|f| {
        f.layout_no_wrap(
            icon.to_string(),
            egui::FontId::proportional(16.0),
            text_color,
        )
    });
    let text_pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(text_pos, galley, vis.text_color());
    resp
}

/// Frameless icon button, vertically centered on the current line.
/// Returns the `Response` so callers can check `.clicked()`, add tooltips, etc.
pub(crate) fn icon_button(ui: &mut egui::Ui, icon: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(icon).size(ICON_SIZE)).frame(false))
}

/// Clipboard-icon button that copies `text` and briefly shows a checkmark.
pub(crate) fn copy_button(ui: &mut egui::Ui, text: &str) {
    let id = ui.id().with("copy_feedback").with(text);
    let copied_until: Option<f64> = ui.data(|d| d.get_temp(id));
    let now = ui.input(|i| i.time);
    let showing_check = copied_until.is_some_and(|t| now < t);

    let icon = if showing_check {
        egui_phosphor::regular::CHECK
    } else {
        egui_phosphor::regular::CLIPBOARD
    };

    if icon_button(ui, icon).clicked() {
        ui.ctx().copy_text(text.to_owned());
        ui.data_mut(|d| d.insert_temp(id, now + 1.5));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(1600));
    }
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

/// ALL-CAPS bold field label (e.g. grid keys).
pub(crate) fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).strong());
}

/// Subdued descriptive text shown beneath section headings.
pub(crate) fn hint_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak());
}

/// Placeholder / empty-state text (e.g. "No items.", "resolving...").
pub(crate) fn muted_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak());
}

/// Red error message with spacing below.
pub(crate) fn error_label(ui: &mut egui::Ui, text: &str) {
    ui.colored_label(colors::ERROR, text);
    ui.add_space(4.0);
}

// ---------------------------------------------------------------------------
// Editable list — text input + ADD button + removable monospace items
// ---------------------------------------------------------------------------

fn square_icon_button(ui: &mut egui::Ui, icon: &str) -> bool {
    let size = egui::vec2(WIDGET_HEIGHT, WIDGET_HEIGHT);
    ui.add(
        egui::Button::new(
            egui::RichText::new(icon)
                .size(ICON_SIZE)
                .color(egui::Color32::WHITE),
        )
        .fill(egui::Color32::BLACK)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 55, 60)))
        .corner_radius(0.0)
        .min_size(size),
    )
    .clicked()
}

pub(crate) fn editable_list(
    ui: &mut egui::Ui,
    items: &mut Vec<String>,
    input: &mut String,
    _input_width: f32,
) {
    let btn_w = WIDGET_HEIGHT + ui.spacing().item_spacing.x;

    ui.horizontal(|ui| {
        let w = (ui.available_width() - btn_w).max(80.0);
        let resp = ui.add(text_input(input, w));
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

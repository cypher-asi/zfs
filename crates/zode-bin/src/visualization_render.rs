use eframe::egui;

use crate::components::tokens::colors;
use crate::helpers::shorten_zid;
use crate::visualization::{color_of, radius_of, Camera, GraphNode, NetworkVisualization};

impl NetworkVisualization {
    pub fn render(&mut self, ui: &mut egui::Ui) {
        let peer_count = self
            .nodes
            .iter()
            .filter(|n| !n.is_local && n.connected)
            .count();
        let avail = ui.available_size();
        let (outer_rect, _) = ui.allocate_exact_size(avail, egui::Sense::hover());

        let mut child_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(outer_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        let ui = &mut child_ui;

        let (resp, painter) = ui.allocate_painter(outer_rect.size(), egui::Sense::click_and_drag());
        let rect = resp.rect;
        let center = rect.center();

        self.handle_pan_zoom(&resp, ui);
        self.tick_layout();

        painter.rect_filled(rect, 0.0, colors::PANEL_BG);
        paint_grid(&painter, rect, center, &self.camera);

        let accent = colors::ACCENT;
        let hovered_idx = resp.hover_pos().and_then(|pp| self.hit_test(pp, center));

        self.paint_edges(&painter, center);
        self.paint_nodes(&painter, center, rect, accent, hovered_idx);
        self.paint_hover_tooltip(&painter, center, hovered_idx);
        self.handle_click_selection(&resp, center, ui);
        self.paint_selection_panel(&painter, rect);
        self.paint_overlay_controls(ui, rect, peer_count);

        let energy: f32 = self.nodes.iter().map(|n| n.vel.length_sq()).sum();
        if energy > 0.01 {
            ui.ctx().request_repaint();
        }
    }

    fn handle_pan_zoom(&mut self, resp: &egui::Response, ui: &egui::Ui) {
        if resp.dragged() {
            self.camera.offset += resp.drag_delta() / self.camera.zoom;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let factor = 1.0 + scroll * 0.003;
                self.camera.zoom = (self.camera.zoom * factor).clamp(0.2, 5.0);
            }
        }
    }

    fn paint_edges(&self, painter: &egui::Painter, center: egui::Pos2) {
        let stroke = egui::Stroke::new(1.5, colors::VIZ_EDGE);
        for &[a, b] in &self.edges {
            let p1 = self.world_to_screen(self.nodes[a].pos, center);
            let p2 = self.world_to_screen(self.nodes[b].pos, center);
            painter.line_segment([p1, p2], stroke);
        }
    }

    fn paint_nodes(
        &self,
        painter: &egui::Painter,
        center: egui::Pos2,
        clip: egui::Rect,
        accent: egui::Color32,
        hovered_idx: Option<usize>,
    ) {
        for (i, node) in self.nodes.iter().enumerate() {
            let sp = self.world_to_screen(node.pos, center);
            if !clip.expand(20.0).contains(sp) {
                continue;
            }
            let r = radius_of(node) * self.camera.zoom.sqrt();
            let color = color_of(node, accent);
            let highlighted =
                hovered_idx == Some(i) || self.selected.as_deref() == Some(node.id.as_str());

            if highlighted {
                painter.circle_filled(sp, r + 4.0, color.linear_multiply(0.3));
            }
            if node.is_local {
                painter.circle_filled(sp, r, egui::Color32::BLACK);
                painter.circle_stroke(sp, r, egui::Stroke::new(2.0, egui::Color32::WHITE));
            } else {
                painter.circle_filled(sp, r, color);
            }

            self.paint_node_label(painter, node, sp, r);
        }
    }

    fn paint_node_label(&self, painter: &egui::Painter, node: &GraphNode, sp: egui::Pos2, r: f32) {
        if node.is_local {
            painter.text(
                sp + egui::vec2(0.0, r + 10.0),
                egui::Align2::CENTER_TOP,
                "YOU",
                egui::FontId::proportional(11.0),
                egui::Color32::WHITE,
            );
        } else if self.camera.zoom > 0.6 {
            let short = shorten_zid(&node.id, 6);
            painter.text(
                sp + egui::vec2(0.0, r + 8.0),
                egui::Align2::CENTER_TOP,
                &short,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
        }
    }

    fn paint_hover_tooltip(
        &self,
        painter: &egui::Painter,
        center: egui::Pos2,
        hovered_idx: Option<usize>,
    ) {
        let Some(idx) = hovered_idx else { return };
        let node = &self.nodes[idx];
        let sp = self.world_to_screen(node.pos, center);
        let r = radius_of(node) * self.camera.zoom.sqrt();
        let short = shorten_zid(&node.id, 12);
        let tip = if node.is_local {
            format!("YOU  {short}")
        } else if node.connected {
            format!("{short}  connected")
        } else {
            format!("{short}  discovered")
        };
        painter.text(
            sp + egui::vec2(r + 8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            tip,
            egui::FontId::monospace(10.0),
            colors::VIZ_TOOLTIP,
        );
    }

    fn handle_click_selection(&mut self, resp: &egui::Response, center: egui::Pos2, ui: &egui::Ui) {
        if resp.clicked() {
            if let Some(pp) = resp.interact_pointer_pos() {
                if let Some(idx) = self.hit_test(pp, center) {
                    let id = self.nodes[idx].id.clone();
                    self.selected = if self.selected.as_deref() == Some(&id) {
                        None
                    } else {
                        Some(id)
                    };
                } else {
                    self.selected = None;
                }
            }
        }
        if self.selected.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.selected = None;
        }
    }

    fn paint_selection_panel(&self, painter: &egui::Painter, rect: egui::Rect) {
        let Some(ref sel_id) = self.selected else {
            return;
        };
        let Some(&idx) = self.index.get(sel_id) else {
            return;
        };
        let node = &self.nodes[idx];
        let (status_label, status_color) = if node.is_local {
            ("Local (YOU)", colors::ACCENT)
        } else if node.connected {
            ("Connected", colors::CONNECTED)
        } else {
            ("Discovered", colors::TEXT_MUTED)
        };

        let margin = 12.0;
        let line_h = 14.0;
        let font = egui::FontId::monospace(11.0);
        let ip_label = node.ip_addr.as_deref().unwrap_or("Unknown");
        let loc_label = node.location.as_deref().unwrap_or("Unknown");
        let lines = [
            format!("ID: {}", node.id),
            format!("Status: {status_label}"),
            format!("IP: {ip_label}"),
            format!("Location: {loc_label}"),
        ];

        let max_w = lines
            .iter()
            .map(|l| {
                painter
                    .layout_no_wrap(l.clone(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x
            })
            .fold(0.0f32, f32::max);

        let pad = 10.0;
        let panel_w = max_w + pad * 2.0;
        let panel_h = lines.len() as f32 * line_h + pad * 2.0;
        let panel_rect = egui::Rect::from_min_size(
            egui::pos2(
                rect.right() - panel_w - margin,
                rect.bottom() - panel_h - margin,
            ),
            egui::vec2(panel_w, panel_h),
        );

        painter.rect_filled(panel_rect, 4.0, colors::VIZ_OVERLAY_BG);
        painter.rect_stroke(
            panel_rect,
            4.0,
            egui::Stroke::new(1.0, colors::BORDER_DIM),
            egui::StrokeKind::Outside,
        );

        for (i, line) in lines.iter().enumerate() {
            let color = if i == 1 {
                status_color
            } else {
                colors::VIZ_PANEL_TEXT
            };
            painter.text(
                egui::pos2(
                    panel_rect.left() + pad,
                    panel_rect.top() + pad + i as f32 * line_h,
                ),
                egui::Align2::LEFT_TOP,
                line,
                font.clone(),
                color,
            );
        }
    }

    fn paint_overlay_controls(&mut self, ui: &egui::Ui, rect: egui::Rect, peer_count: usize) {
        let overlay_pos = rect.left_top() + egui::vec2(12.0, 8.0);
        let overlay_w = rect.width() - 24.0;
        egui::Area::new(egui::Id::new("viz_overlay"))
            .fixed_pos(overlay_pos)
            .interactable(true)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.set_width(overlay_w);
                ui.horizontal(|ui| {
                    crate::components::overlay_frame().show(ui, |ui| {
                            crate::components::section_heading(
                                ui,
                                &format!(
                                    "The Grid  \u{2022}  {peer_count} {}",
                                    if peer_count == 1 { "peer" } else { "peers" }
                                ),
                            );
                        });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        crate::components::overlay_frame()
                            .inner_margin(egui::Margin::symmetric(6, 4))
                            .show(ui, |ui| {
                                if crate::components::icon_button(
                                    ui,
                                    egui_phosphor::regular::ARROWS_IN,
                                )
                                .clicked()
                                {
                                    self.camera = Camera::default();
                                }
                            });
                    });
                });
            });
    }
}

// ---------------------------------------------------------------------------
// Grid painting
// ---------------------------------------------------------------------------

struct GridParams {
    spacing: f32,
    dot_radius: f32,
    start_x: f32,
    ys: Vec<f32>,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    height: f32,
}

impl GridParams {
    fn fade(&self, y: f32) -> f32 {
        ((y - self.top) / self.height).clamp(0.0, 1.0).powf(0.6) * 0.85 + 0.15
    }
}

fn grid_params(clip: egui::Rect, center: egui::Pos2, cam: &Camera) -> GridParams {
    const BASE_SPACING: f32 = 50.0;
    let spacing = BASE_SPACING * cam.zoom;
    let dot_radius = (1.5 * cam.zoom).clamp(0.5, 2.0);
    let origin = center + cam.offset * cam.zoom;

    let left = clip.left();
    let right = clip.right();
    let top = clip.top();
    let bottom = clip.bottom();
    let height = (bottom - top).max(1.0);

    let start_x = origin.x + ((left - origin.x) / spacing).floor() * spacing;
    let first_y = origin.y + ((top - origin.y) / spacing).floor() * spacing;

    let mut ys = Vec::new();
    let mut y = first_y;
    while y <= bottom {
        if y >= top {
            ys.push(y);
        }
        y += spacing;
    }

    GridParams {
        spacing,
        dot_radius,
        start_x,
        ys,
        left,
        right,
        top,
        bottom,
        height,
    }
}

fn paint_grid(painter: &egui::Painter, clip: egui::Rect, center: egui::Pos2, cam: &Camera) {
    let g = grid_params(clip, center, cam);
    paint_vertical_lines(painter, &g);
    paint_horizontal_lines(painter, &g);
    paint_grid_dots(painter, &g);
}

fn faded_color(base: egui::Color32, t: f32) -> egui::Color32 {
    let [r, g, b, _] = base.to_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, (t * 255.0) as u8)
}

fn paint_vertical_lines(painter: &egui::Painter, g: &GridParams) {
    let mut x = g.start_x;
    while x <= g.right {
        if x >= g.left {
            let mut prev = g.top;
            for &gy in &g.ys {
                let t = g.fade((prev + gy) * 0.5);
                painter.line_segment(
                    [egui::pos2(x, prev), egui::pos2(x, gy)],
                    egui::Stroke::new(1.0, faded_color(colors::VIZ_GRID_LINE, t)),
                );
                prev = gy;
            }
            if prev < g.bottom {
                let t = g.fade((prev + g.bottom) * 0.5);
                painter.line_segment(
                    [egui::pos2(x, prev), egui::pos2(x, g.bottom)],
                    egui::Stroke::new(1.0, faded_color(colors::VIZ_GRID_LINE, t)),
                );
            }
        }
        x += g.spacing;
    }
}

fn paint_horizontal_lines(painter: &egui::Painter, g: &GridParams) {
    for &gy in &g.ys {
        let t = g.fade(gy);
        painter.line_segment(
            [egui::pos2(g.left, gy), egui::pos2(g.right, gy)],
            egui::Stroke::new(1.0, faded_color(colors::VIZ_GRID_LINE, t)),
        );
    }
}

fn paint_grid_dots(painter: &egui::Painter, g: &GridParams) {
    let mut x = g.start_x;
    while x <= g.right {
        if x >= g.left {
            for &gy in &g.ys {
                let t = g.fade(gy);
                painter.circle_filled(egui::pos2(x, gy), g.dot_radius, faded_color(colors::VIZ_GRID_DOT, t));
            }
        }
        x += g.spacing;
    }
}

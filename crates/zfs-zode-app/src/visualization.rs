use std::collections::{HashMap, HashSet};

use eframe::egui;

const LOCAL_RADIUS: f32 = 12.0;
const PEER_RADIUS: f32 = 8.0;
const DISCOVERED_RADIUS: f32 = 6.0;

const REPULSION_K: f32 = 5000.0;
const SPRING_K: f32 = 0.01;
const CENTER_K: f32 = 0.005;
const DAMPING: f32 = 0.85;
const MIN_DIST: f32 = 20.0;

#[derive(Default)]
pub(crate) struct NetworkVisualization {
    nodes: Vec<GraphNode>,
    edges: Vec<[usize; 2]>,
    index: HashMap<String, usize>,
    camera: Camera,
    selected: Option<String>,
    local_id: Option<String>,
}

struct GraphNode {
    id: String,
    pos: egui::Vec2,
    vel: egui::Vec2,
    is_local: bool,
    connected: bool,
    discovered: bool,
}

struct Camera {
    offset: egui::Vec2,
    zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            offset: egui::Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl NetworkVisualization {
    /// Reconcile the graph against the authoritative ZodeStatus.
    /// Ensures the graph matches `connected_peers` — adds missing nodes,
    /// removes or demotes stale ones.
    pub fn reconcile(&mut self, local_id: &str, peers: &[String]) {
        if self.local_id.as_deref() != Some(local_id) {
            self.nodes.clear();
            self.edges.clear();
            self.index.clear();
            self.selected = None;
            self.local_id = Some(local_id.to_string());
        }

        let idx = self.ensure_node(local_id, true);
        self.nodes[idx].connected = true;

        let peer_set: HashSet<&str> = peers.iter().map(|s| s.as_str()).collect();

        for p in peers {
            let i = self.ensure_node(p, false);
            self.nodes[i].connected = true;
        }

        let local = local_id.to_string();
        let stale: Vec<String> = self
            .index
            .keys()
            .filter(|id| *id != &local)
            .cloned()
            .collect();

        for id in stale {
            if peer_set.contains(id.as_str()) {
                continue;
            }
            if let Some(&i) = self.index.get(&id) {
                if self.nodes[i].connected {
                    self.nodes[i].connected = false;
                    if !self.nodes[i].discovered {
                        self.remove_node(&id);
                    }
                }
            }
        }

        self.rebuild_edges();
    }

    pub fn on_peer_connected(&mut self, id: &str) {
        let i = self.ensure_node(id, false);
        self.nodes[i].connected = true;
        self.rebuild_edges();
    }

    pub fn on_peer_disconnected(&mut self, id: &str) {
        if let Some(&i) = self.index.get(id) {
            self.nodes[i].connected = false;
            if !self.nodes[i].discovered {
                self.remove_node(id);
            }
        }
        if let Some(ref sel) = self.selected {
            if sel == id && !self.index.contains_key(id) {
                self.selected = None;
            }
        }
        self.rebuild_edges();
    }

    pub fn on_peer_discovered(&mut self, id: &str) {
        let i = self.ensure_node(id, false);
        self.nodes[i].discovered = true;
    }

    fn ensure_node(&mut self, id: &str, is_local: bool) -> usize {
        if let Some(&idx) = self.index.get(id) {
            return idx;
        }
        let idx = self.nodes.len();
        let pos = if is_local {
            egui::Vec2::ZERO
        } else {
            let h = djb2(id);
            let angle = (h as f32) * 0.618 * std::f32::consts::TAU;
            let r = 80.0 + (h % 60) as f32;
            egui::vec2(angle.cos() * r, angle.sin() * r)
        };
        self.nodes.push(GraphNode {
            id: id.to_string(),
            pos,
            vel: egui::Vec2::ZERO,
            is_local,
            connected: false,
            discovered: false,
        });
        self.index.insert(id.to_string(), idx);
        idx
    }

    fn remove_node(&mut self, id: &str) {
        let Some(idx) = self.index.remove(id) else {
            return;
        };
        self.nodes.swap_remove(idx);
        if idx < self.nodes.len() {
            let swapped = self.nodes[idx].id.clone();
            self.index.insert(swapped, idx);
        }
    }

    fn rebuild_edges(&mut self) {
        self.edges.clear();
        let Some(ref lid) = self.local_id else { return };
        let Some(&li) = self.index.get(lid) else { return };
        for (i, node) in self.nodes.iter().enumerate() {
            if i != li && node.connected {
                self.edges.push([li, i]);
            }
        }
    }

    fn tick_layout(&mut self) {
        let n = self.nodes.len();
        if n <= 1 {
            return;
        }

        let mut forces = vec![egui::Vec2::ZERO; n];

        for i in 0..n {
            for j in (i + 1)..n {
                let d = self.nodes[i].pos - self.nodes[j].pos;
                let dist = d.length().max(MIN_DIST);
                let f = REPULSION_K / (dist * dist);
                let dir = d / dist;
                forces[i] += dir * f;
                forces[j] -= dir * f;
            }
        }

        for &[a, b] in &self.edges {
            let d = self.nodes[b].pos - self.nodes[a].pos;
            let dist = d.length().max(1.0);
            let f = dist * SPRING_K;
            let dir = d / dist;
            forces[a] += dir * f;
            forces[b] -= dir * f;
        }

        let mut centroid = egui::Vec2::ZERO;
        for node in &self.nodes {
            centroid += node.pos;
        }
        centroid /= n as f32;
        for f in &mut forces {
            *f -= centroid * CENTER_K;
        }

        for (i, node) in self.nodes.iter_mut().enumerate() {
            if node.is_local {
                node.vel = egui::Vec2::ZERO;
                node.pos *= 0.95;
                continue;
            }
            node.vel = (node.vel + forces[i]) * DAMPING;
            node.pos += node.vel;
        }
    }

    fn world_to_screen(&self, world: egui::Vec2, center: egui::Pos2) -> egui::Pos2 {
        center + (world + self.camera.offset) * self.camera.zoom
    }

    fn hit_test(&self, screen_pos: egui::Pos2, center: egui::Pos2) -> Option<usize> {
        for (i, node) in self.nodes.iter().enumerate().rev() {
            let sp = self.world_to_screen(node.pos, center);
            let r = radius_of(node) * self.camera.zoom.sqrt() + 4.0;
            if screen_pos.distance(sp) <= r {
                return Some(i);
            }
        }
        None
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
        let peer_count = self
            .nodes
            .iter()
            .filter(|n| !n.is_local && n.connected)
            .count();
        let accent = egui::Color32::from_rgb(0, 180, 255);

        let avail = ui.available_size();

        let (outer_rect, _) =
            ui.allocate_exact_size(avail, egui::Sense::hover());

        let mut child_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(outer_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        let ui = &mut child_ui;

        {
                let (resp, painter) = ui.allocate_painter(
                    outer_rect.size(),
                    egui::Sense::click_and_drag(),
                );
                let rect = resp.rect;
                let center = rect.center();

                // Pan
                if resp.dragged() {
                    self.camera.offset += resp.drag_delta() / self.camera.zoom;
                }

                // Zoom
                if resp.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        let factor = 1.0 + scroll * 0.003;
                        self.camera.zoom = (self.camera.zoom * factor).clamp(0.2, 5.0);
                    }
                }

                self.tick_layout();

                painter.rect_filled(rect, 0.0, egui::Color32::BLACK);

                // Grid
                paint_grid(&painter, rect, center, &self.camera);

                // Edges
                let edge_stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(60, 60, 70));
                for &[a, b] in &self.edges {
                    let p1 = self.world_to_screen(self.nodes[a].pos, center);
                    let p2 = self.world_to_screen(self.nodes[b].pos, center);
                    painter.line_segment([p1, p2], edge_stroke);
                }

                // Hover detection
                let hovered_idx = resp
                    .hover_pos()
                    .and_then(|pp| self.hit_test(pp, center));

                // Nodes
                for (i, node) in self.nodes.iter().enumerate() {
                    let sp = self.world_to_screen(node.pos, center);
                    if !rect.expand(20.0).contains(sp) {
                        continue;
                    }

                    let r = radius_of(node) * self.camera.zoom.sqrt();
                    let color = color_of(node, accent);
                    let highlighted = hovered_idx == Some(i)
                        || self.selected.as_deref() == Some(node.id.as_str());

                    if highlighted {
                        painter.circle_filled(sp, r + 4.0, color.linear_multiply(0.3));
                    }

                    if node.is_local {
                        painter.circle_filled(sp, r, egui::Color32::BLACK);
                        painter.circle_stroke(sp, r, egui::Stroke::new(2.0, egui::Color32::WHITE));
                    } else {
                        painter.circle_filled(sp, r, color);
                    }

                    if node.is_local {
                        painter.text(
                            sp + egui::vec2(0.0, r + 10.0),
                            egui::Align2::CENTER_TOP,
                            "YOU",
                            egui::FontId::proportional(11.0),
                            egui::Color32::WHITE,
                        );
                    } else if self.camera.zoom > 0.6 {
                        let short = &node.id[..8.min(node.id.len())];
                        painter.text(
                            sp + egui::vec2(0.0, r + 8.0),
                            egui::Align2::CENTER_TOP,
                            short,
                            egui::FontId::proportional(13.0),
                            egui::Color32::WHITE,
                        );
                    }
                }

                // Hover tooltip
                if let Some(idx) = hovered_idx {
                    let node = &self.nodes[idx];
                    let sp = self.world_to_screen(node.pos, center);
                    let r = radius_of(node) * self.camera.zoom.sqrt();
                    let short = &node.id[..16.min(node.id.len())];
                    let tip = if node.is_local {
                        format!("YOU  {short}...")
                    } else if node.connected {
                        format!("{short}...  connected")
                    } else {
                        format!("{short}...  discovered")
                    };
                    painter.text(
                        sp + egui::vec2(r + 8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        tip,
                        egui::FontId::monospace(10.0),
                        egui::Color32::from_rgb(200, 200, 200),
                    );
                }

                // Click to select / deselect
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

                // Escape clears selection
                if self.selected.is_some()
                    && ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    self.selected = None;
                }

                // Selected node info — bottom-right of grid
                if let Some(ref sel_id) = self.selected {
                    if let Some(&idx) = self.index.get(sel_id) {
                        let node = &self.nodes[idx];
                        let accent = egui::Color32::from_rgb(0, 180, 255);
                        let status_text = if node.is_local {
                            ("Local (YOU)", accent)
                        } else if node.connected {
                            ("Connected", egui::Color32::from_rgb(100, 255, 100))
                        } else {
                            ("Discovered", egui::Color32::from_rgb(160, 160, 160))
                        };

                        let margin = 12.0;
                        let line_h = 14.0;
                        let lines = [
                            format!("ID: {}", node.id),
                            format!("Status: {}", status_text.0),
                            "IP: Unknown".to_string(),
                            "Location: Unknown".to_string(),
                        ];

                        let font = egui::FontId::monospace(11.0);
                        let mut max_w: f32 = 0.0;
                        for line in &lines {
                            let galley = painter.layout_no_wrap(
                                line.clone(),
                                font.clone(),
                                egui::Color32::WHITE,
                            );
                            max_w = max_w.max(galley.size().x);
                        }

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

                        painter.rect_filled(
                            panel_rect,
                            4.0,
                            egui::Color32::from_rgba_unmultiplied(10, 10, 12, 210),
                        );
                        painter.rect_stroke(
                            panel_rect,
                            4.0,
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 50, 55)),
                        );

                        for (i, line) in lines.iter().enumerate() {
                            let color = if i == 1 { status_text.1 } else { egui::Color32::from_rgb(190, 190, 190) };
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
                }

                // Overlay controls
                let overlay_pos = rect.left_top() + egui::vec2(12.0, 8.0);
                let overlay_w = rect.width() - 24.0;
                egui::Area::new(egui::Id::new("viz_overlay"))
                    .fixed_pos(overlay_pos)
                    .interactable(true)
                    .order(egui::Order::Foreground)
                    .show(ui.ctx(), |ui| {
                        ui.set_width(overlay_w);
                        ui.horizontal(|ui| {
                            egui::Frame::default()
                                .fill(egui::Color32::from_rgba_unmultiplied(10, 10, 12, 200))
                                .rounding(6.0)
                                .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                                .show(ui, |ui| {
                                    crate::components::section_heading(
                                        ui,
                                        &format!("The Grid  \u{2022}  {peer_count} peers"),
                                    );
                                });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                egui::Frame::default()
                                    .fill(egui::Color32::from_rgba_unmultiplied(10, 10, 12, 200))
                                    .rounding(6.0)
                                    .inner_margin(egui::Margin::symmetric(6.0, 4.0))
                                    .show(ui, |ui| {
                                        if crate::components::icon_button(ui, egui_phosphor::regular::ARROWS_IN).clicked() {
                                            self.camera = Camera::default();
                                        }
                                    });
                            });
                        });
                    });

                let energy: f32 = self.nodes.iter().map(|n| n.vel.length_sq()).sum();
                if energy > 0.01 {
                    ui.ctx().request_repaint();
                }
        }
    }

}

fn radius_of(node: &GraphNode) -> f32 {
    if node.is_local {
        LOCAL_RADIUS
    } else if node.connected {
        PEER_RADIUS
    } else {
        DISCOVERED_RADIUS
    }
}

fn color_of(node: &GraphNode, _accent: egui::Color32) -> egui::Color32 {
    if node.is_local {
        egui::Color32::WHITE
    } else if node.connected {
        egui::Color32::from_rgb(120, 120, 130)
    } else {
        egui::Color32::from_rgba_premultiplied(70, 70, 70, 100)
    }
}

fn paint_grid(painter: &egui::Painter, clip: egui::Rect, center: egui::Pos2, cam: &Camera) {
    const BASE_SPACING: f32 = 50.0;
    const LINE_RGB: [u8; 3] = [48, 48, 54];
    const DOT_RGB: [u8; 3] = [72, 72, 80];
    let spacing = BASE_SPACING * cam.zoom;
    let dot_radius = (1.5 * cam.zoom).clamp(0.5, 2.0);
    let origin = center + cam.offset * cam.zoom;

    let left = clip.left();
    let right = clip.right();
    let top = clip.top();
    let bottom = clip.bottom();
    let height = (bottom - top).max(1.0);

    let fade = |y: f32| -> f32 {
        ((y - top) / height).clamp(0.0, 1.0).powf(0.6) * 0.85 + 0.15
    };

    let start_x = origin.x + ((left - origin.x) / spacing).floor() * spacing;
    let start_y = origin.y + ((top - origin.y) / spacing).floor() * spacing;

    // Collect y positions for segmenting vertical lines
    let mut ys: Vec<f32> = Vec::new();
    let mut y = start_y;
    while y <= bottom {
        if y >= top {
            ys.push(y);
        }
        y += spacing;
    }

    // Vertical lines — drawn per-segment so each segment fades with y
    let mut x = start_x;
    while x <= right {
        if x >= left {
            let mut prev = top;
            for &gy in &ys {
                let mid = (prev + gy) * 0.5;
                let t = fade(mid);
                let c = egui::Color32::from_rgba_unmultiplied(
                    LINE_RGB[0], LINE_RGB[1], LINE_RGB[2],
                    (t * 255.0) as u8,
                );
                painter.line_segment([egui::pos2(x, prev), egui::pos2(x, gy)], egui::Stroke::new(1.0, c));
                prev = gy;
            }
            if prev < bottom {
                let mid = (prev + bottom) * 0.5;
                let t = fade(mid);
                let c = egui::Color32::from_rgba_unmultiplied(
                    LINE_RGB[0], LINE_RGB[1], LINE_RGB[2],
                    (t * 255.0) as u8,
                );
                painter.line_segment([egui::pos2(x, prev), egui::pos2(x, bottom)], egui::Stroke::new(1.0, c));
            }
        }
        x += spacing;
    }

    // Horizontal lines
    for &gy in &ys {
        let t = fade(gy);
        let c = egui::Color32::from_rgba_unmultiplied(
            LINE_RGB[0], LINE_RGB[1], LINE_RGB[2],
            (t * 255.0) as u8,
        );
        painter.line_segment(
            [egui::pos2(left, gy), egui::pos2(right, gy)],
            egui::Stroke::new(1.0, c),
        );
    }

    // Dots at intersections
    let mut x = start_x;
    while x <= right {
        if x >= left {
            for &gy in &ys {
                let t = fade(gy);
                let c = egui::Color32::from_rgba_unmultiplied(
                    DOT_RGB[0], DOT_RGB[1], DOT_RGB[2],
                    (t * 255.0) as u8,
                );
                painter.circle_filled(egui::pos2(x, gy), dot_radius, c);
            }
        }
        x += spacing;
    }
}

fn djb2(s: &str) -> u64 {
    let mut h: u64 = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    h
}

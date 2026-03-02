use std::collections::{HashMap, HashSet};

use eframe::egui;

use crate::components::tokens::colors;
use crate::components::{overlay_frame, section_heading};
use crate::helpers::{node_color, shorten_zid};
use crate::state::AppState;

const LOCAL_RADIUS: f32 = 12.0;
const PEER_RADIUS: f32 = 8.0;
const REPULSION_K: f32 = 5000.0;
const SPRING_K: f32 = 0.01;
const CENTER_K: f32 = 0.005;
const DAMPING: f32 = 0.85;
const MIN_DIST: f32 = 20.0;

#[derive(Default)]
pub(crate) struct TopologyVisualization {
    pub nodes: Vec<TopoNode>,
    pub edges: Vec<[usize; 2]>,
    pub index: HashMap<String, usize>,
    pub camera: Camera,
}

pub(crate) struct TopoNode {
    pub id: String,
    pub pos: egui::Vec2,
    pub vel: egui::Vec2,
    pub node_index: Option<usize>,
    pub connected_peers: HashSet<String>,
}

pub(crate) struct Camera {
    pub offset: egui::Vec2,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            offset: egui::Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl TopologyVisualization {
    /// Reconcile the graph against the current app state.
    pub fn reconcile(&mut self, state: &AppState) {
        let mut all_ids: HashSet<String> = HashSet::new();

        for ns in &state.nodes {
            if ns.zode_id.is_empty() {
                continue;
            }
            all_ids.insert(ns.zode_id.clone());
            let idx = self.ensure_node(&ns.zode_id, Some(ns.node_id));
            self.nodes[idx].connected_peers.clear();

            if let Some(ref status) = ns.status {
                for peer in &status.connected_peers {
                    all_ids.insert(peer.clone());
                    self.nodes[idx].connected_peers.insert(peer.clone());
                    self.ensure_node(peer, None);
                }
            }
        }

        self.nodes.retain(|n| all_ids.contains(&n.id));
        self.index.clear();
        for (i, n) in self.nodes.iter().enumerate() {
            self.index.insert(n.id.clone(), i);
        }

        self.rebuild_edges();
    }

    fn ensure_node(&mut self, id: &str, node_index: Option<usize>) -> usize {
        if let Some(&idx) = self.index.get(id) {
            if node_index.is_some() {
                self.nodes[idx].node_index = node_index;
            }
            return idx;
        }
        let idx = self.nodes.len();
        let h = djb2(id);
        let angle = (h as f32) * 0.618 * std::f32::consts::TAU;
        let r = 80.0 + (h % 60) as f32;
        self.nodes.push(TopoNode {
            id: id.to_string(),
            pos: egui::vec2(angle.cos() * r, angle.sin() * r),
            vel: egui::Vec2::ZERO,
            node_index,
            connected_peers: HashSet::new(),
        });
        self.index.insert(id.to_string(), idx);
        idx
    }

    fn rebuild_edges(&mut self) {
        self.edges.clear();
        let mut seen = HashSet::new();
        for (i, node) in self.nodes.iter().enumerate() {
            for peer_id in &node.connected_peers {
                if let Some(&j) = self.index.get(peer_id) {
                    let key = if i < j { (i, j) } else { (j, i) };
                    if seen.insert(key) {
                        self.edges.push([key.0, key.1]);
                    }
                }
            }
        }
    }

    pub fn tick_layout(&mut self) {
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
        let total = self.nodes.len();
        let managed = self.nodes.iter().filter(|n| n.node_index.is_some()).count();
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

        let hovered_idx = resp.hover_pos().and_then(|pp| self.hit_test(pp, center));

        for &[a, b] in &self.edges {
            let p1 = self.world_to_screen(self.nodes[a].pos, center);
            let p2 = self.world_to_screen(self.nodes[b].pos, center);
            painter.line_segment([p1, p2], egui::Stroke::new(1.5, colors::VIZ_EDGE));
        }

        for (i, node) in self.nodes.iter().enumerate() {
            let sp = self.world_to_screen(node.pos, center);
            if !rect.expand(20.0).contains(sp) {
                continue;
            }
            let r = radius_of(node) * self.camera.zoom.sqrt();
            let color = color_of(node);
            let highlighted = hovered_idx == Some(i);

            if highlighted {
                painter.circle_filled(sp, r + 4.0, color.linear_multiply(0.3));
            }
            painter.circle_filled(sp, r, color);

            if self.camera.zoom > 0.6 {
                let short = shorten_zid(&node.id, 6);
                painter.text(
                    sp + egui::vec2(0.0, r + 8.0),
                    egui::Align2::CENTER_TOP,
                    &short,
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
            }
        }

        if let Some(idx) = hovered_idx {
            let node = &self.nodes[idx];
            let sp = self.world_to_screen(node.pos, center);
            let r = radius_of(node) * self.camera.zoom.sqrt();
            let label = if let Some(ni) = node.node_index {
                format!("Node {ni} — {}", shorten_zid(&node.id, 12))
            } else {
                format!("Peer — {}", shorten_zid(&node.id, 12))
            };
            painter.text(
                sp + egui::vec2(r + 8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::monospace(10.0),
                colors::VIZ_TOOLTIP,
            );
        }

        let overlay_pos = rect.left_top() + egui::vec2(12.0, 8.0);
        let overlay_w = rect.width() - 24.0;
        egui::Area::new(egui::Id::new("topo_overlay"))
            .fixed_pos(overlay_pos)
            .interactable(true)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.set_width(overlay_w);
                ui.horizontal(|ui| {
                    overlay_frame().show(ui, |ui| {
                        section_heading(
                            ui,
                            &format!(
                                "TOPOLOGY  \u{2022}  {managed} nodes  \u{2022}  {total} peers"
                            ),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        overlay_frame()
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
}

fn radius_of(node: &TopoNode) -> f32 {
    if node.node_index.is_some() {
        LOCAL_RADIUS
    } else {
        PEER_RADIUS
    }
}

fn color_of(node: &TopoNode) -> egui::Color32 {
    if let Some(idx) = node.node_index {
        node_color(idx)
    } else {
        colors::VIZ_PEER_NODE
    }
}

fn djb2(s: &str) -> u64 {
    let mut h: u64 = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    h
}

fn faded_color(base: egui::Color32, t: f32) -> egui::Color32 {
    let [r, g, b, _] = base.to_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, (t * 255.0) as u8)
}

fn paint_grid(painter: &egui::Painter, clip: egui::Rect, center: egui::Pos2, cam: &Camera) {
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

    let fade = |y: f32| -> f32 { ((y - top) / height).clamp(0.0, 1.0).powf(0.6) * 0.85 + 0.15 };

    let mut x = start_x;
    while x <= right {
        if x >= left {
            painter.line_segment(
                [egui::pos2(x, top), egui::pos2(x, bottom)],
                egui::Stroke::new(
                    1.0,
                    faded_color(colors::VIZ_GRID_LINE, fade((top + bottom) * 0.5)),
                ),
            );
        }
        x += spacing;
    }

    let mut y = first_y;
    while y <= bottom {
        if y >= top {
            let t = fade(y);
            painter.line_segment(
                [egui::pos2(left, y), egui::pos2(right, y)],
                egui::Stroke::new(1.0, faded_color(colors::VIZ_GRID_LINE, t)),
            );
        }
        y += spacing;
    }

    let mut x = start_x;
    while x <= right {
        if x >= left {
            let mut y = first_y;
            while y <= bottom {
                if y >= top {
                    let t = fade(y);
                    painter.circle_filled(
                        egui::pos2(x, y),
                        dot_radius,
                        faded_color(colors::VIZ_GRID_DOT, t),
                    );
                }
                y += spacing;
            }
        }
        x += spacing;
    }
}

use std::collections::{HashMap, HashSet};

use eframe::egui;

pub(crate) const LOCAL_RADIUS: f32 = 12.0;
pub(crate) const PEER_RADIUS: f32 = 8.0;
pub(crate) const DISCOVERED_RADIUS: f32 = 6.0;

const REPULSION_K: f32 = 5000.0;
const SPRING_K: f32 = 0.01;
const CENTER_K: f32 = 0.005;
const DAMPING: f32 = 0.85;
const MIN_DIST: f32 = 20.0;

#[derive(Default)]
pub(crate) struct NetworkVisualization {
    pub(crate) nodes: Vec<GraphNode>,
    pub(crate) edges: Vec<[usize; 2]>,
    pub(crate) index: HashMap<String, usize>,
    pub(crate) camera: Camera,
    pub(crate) selected: Option<String>,
    pub(crate) local_id: Option<String>,
}

pub(crate) struct GraphNode {
    pub(crate) id: String,
    pub(crate) pos: egui::Vec2,
    pub(crate) vel: egui::Vec2,
    pub(crate) is_local: bool,
    pub(crate) connected: bool,
    pub(crate) discovered: bool,
    pub(crate) ip_addr: Option<String>,
    pub(crate) location: Option<String>,
}

pub(crate) struct Camera {
    pub(crate) offset: egui::Vec2,
    pub(crate) zoom: f32,
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
    pub fn reconcile(
        &mut self,
        local_id: &str,
        peers: &[String],
        peer_ips: &HashMap<String, String>,
    ) {
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
            if let Some(ip) = peer_ips.get(p) {
                self.nodes[i].ip_addr = Some(ip.clone());
            }
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
            ip_addr: None,
            location: None,
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
        let Some(&li) = self.index.get(lid) else {
            return;
        };
        for (i, node) in self.nodes.iter().enumerate() {
            if i != li && node.connected {
                self.edges.push([li, i]);
            }
        }
    }

    pub(crate) fn tick_layout(&mut self) {
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

    pub(crate) fn world_to_screen(&self, world: egui::Vec2, center: egui::Pos2) -> egui::Pos2 {
        center + (world + self.camera.offset) * self.camera.zoom
    }

    pub(crate) fn hit_test(&self, screen_pos: egui::Pos2, center: egui::Pos2) -> Option<usize> {
        for (i, node) in self.nodes.iter().enumerate().rev() {
            let sp = self.world_to_screen(node.pos, center);
            let r = radius_of(node) * self.camera.zoom.sqrt() + 4.0;
            if screen_pos.distance(sp) <= r {
                return Some(i);
            }
        }
        None
    }
}

pub(crate) fn radius_of(node: &GraphNode) -> f32 {
    if node.is_local {
        LOCAL_RADIUS
    } else if node.connected {
        PEER_RADIUS
    } else {
        DISCOVERED_RADIUS
    }
}

pub(crate) fn color_of(node: &GraphNode, _accent: egui::Color32) -> egui::Color32 {
    use crate::components::tokens::colors;
    if node.is_local {
        egui::Color32::WHITE
    } else if node.connected {
        colors::VIZ_PEER_NODE
    } else {
        egui::Color32::from_rgba_premultiplied(70, 70, 70, 100)
    }
}

fn djb2(s: &str) -> u64 {
    let mut h: u64 = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    h
}

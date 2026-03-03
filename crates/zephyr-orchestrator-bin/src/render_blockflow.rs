use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::components::tokens::{colors, font_size};
use crate::components::overlay_frame;
use crate::state::AppState;

const BAR_HEIGHT: f32 = 10.0;
const BAR_MIN_WIDTH: f32 = 80.0;
const BAR_MAX_WIDTH: f32 = 500.0;
const BAR_WIDTH_PER_TX: f32 = 3.0;
const ROW_HEIGHT: f32 = 28.0;
const ROW_TOP_MARGIN: f32 = 36.0;
const BORDER_STROKE: f32 = 1.2;
const BORDER_ALPHA: f32 = 0.50;
const BLOCK_BG_ALPHA: f32 = 0.09;
const GLOW_FILL_ALPHA: f32 = 0.35;
const GLOW_TAIL_LENGTH: f32 = 80.0;
const GLOW_TAIL_MIN_ALPHA: f32 = 0.04;
const GLOW_FADE_IN_SECS: f32 = 0.3;
const FADE_ZONE_FRAC: f32 = 0.25;
const LABEL_WIDTH: f32 = 72.0;
const BLOCK_GAP: f32 = 6.0;
const SCROLL_SPEED: f32 = 700.0;
const MAX_BLOCKS_PER_ZONE: usize = 200;
const BATCH_STAGGER_SECS: f32 = 0.06;
const ZONE_PHASE_MAX_SECS: f32 = 0.25;
const GAP_JITTER_FRAC: f32 = 0.4;
const MICRO_JITTER_SECS: f32 = 0.06;
const ENTRY_LEAD_SECS: f32 = 0.5;
const COLOR_BLEND_MS: f32 = 150.0;

const PROPOSED_THRESHOLD_MS: u128 = 400;
const VOTING_THRESHOLD_MS: u128 = 900;

const PULSE_FILL_SECS: f32 = 0.35;
const PULSE_DRAIN_SECS: f32 = 0.35;
const PULSE_EDGE_FRAC: f32 = 0.10;
const PULSE_BRIGHT_ALPHA: f32 = 0.80;

pub(crate) struct BlockflowVisualization {
    blocks: Vec<FlowBlock>,
    seen: HashSet<(u32, u64)>,
    camera: Camera,
    scroll_pos: f32,
    last_frame: Instant,
    smoothed_speed: f32,
    zone_buf: Vec<u32>,
}

struct FlowBlock {
    zone_id: u32,
    #[allow(dead_code)]
    height: u64,
    tx_count: usize,
    birth_scroll_pos: f32,
    birth_time: Instant,
    #[allow(dead_code)]
    block_hash_hex: String,
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

impl Default for BlockflowVisualization {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            seen: HashSet::new(),
            camera: Camera::default(),
            scroll_pos: 0.0,
            last_frame: Instant::now(),
            smoothed_speed: SCROLL_SPEED,
            zone_buf: Vec::new(),
        }
    }
}

impl BlockflowVisualization {
    fn ingest(&mut self, state: &AppState) -> bool {
        let first_load = self.seen.is_empty() && !state.recent_blocks.is_empty();

        let mut new_blocks = Vec::new();
        for block in state.recent_blocks.iter() {
            let key = (block.zone_id, block.height);
            if self.seen.insert(key) {
                new_blocks.push(block);
            }
        }

        if new_blocks.is_empty() {
            return false;
        }

        let mut zone_head: HashMap<u32, f32> = HashMap::new();
        for b in &self.blocks {
            let entry = zone_head.entry(b.zone_id).or_insert(f32::NEG_INFINITY);
            if b.birth_scroll_pos > *entry {
                *entry = b.birth_scroll_pos;
            }
        }

        if first_load {
            let speed = self.smoothed_speed.max(1.0);
            let mut by_zone: HashMap<u32, Vec<_>> = HashMap::new();
            for b in &new_blocks {
                by_zone.entry(b.zone_id).or_default().push(*b);
            }
            for (&zone_id, zone_blocks) in &mut by_zone {
                zone_blocks.sort_by(|a, b| b.height.cmp(&a.height));
                let phase_offset =
                    pseudo_rand(zone_id as u64, 0) * ZONE_PHASE_MAX_SECS * speed;
                let mut prev_bsp: Option<f32> = None;
                let mut prev_width: f32 = 0.0;
                for block in zone_blocks.iter() {
                    let bar_w = block_width(block.tx_count);
                    let gap_jitter = pseudo_rand(block.zone_id as u64, block.height)
                        * GAP_JITTER_FRAC
                        * BLOCK_GAP;
                    let bsp = match prev_bsp {
                        None => self.scroll_pos - phase_offset,
                        Some(prev) => prev - prev_width - BLOCK_GAP - gap_jitter,
                    };
                    self.blocks.push(FlowBlock {
                        zone_id: block.zone_id,
                        height: block.height,
                        tx_count: block.tx_count,
                        birth_scroll_pos: bsp,
                        birth_time: Instant::now(),
                        block_hash_hex: block.block_hash_hex.clone(),
                    });
                    prev_bsp = Some(bsp);
                    prev_width = bar_w;
                }
            }
        } else {
            let mut zone_batch_idx: HashMap<u32, usize> = HashMap::new();

            for block in new_blocks.iter() {
                let idx = zone_batch_idx.entry(block.zone_id).or_insert(0);
                let within_zone_stagger = *idx as f32 * BATCH_STAGGER_SECS;
                *idx += 1;

                let zone_phase =
                    pseudo_rand(block.zone_id as u64, 0) * ZONE_PHASE_MAX_SECS;
                let jitter =
                    pseudo_rand(block.zone_id as u64, block.height) * MICRO_JITTER_SECS;

                let bar_w = block_width(block.tx_count);
                let desired_bsp = self.scroll_pos
                    + self.smoothed_speed
                        * (within_zone_stagger + zone_phase + jitter);
                let gap_jitter = pseudo_rand(block.zone_id as u64, block.height)
                    * GAP_JITTER_FRAC
                    * BLOCK_GAP;
                let min_bsp = zone_head
                    .get(&block.zone_id)
                    .map(|&prev| prev + bar_w + BLOCK_GAP + gap_jitter)
                    .unwrap_or(desired_bsp);
                let actual_bsp = desired_bsp.max(min_bsp);
                zone_head.insert(block.zone_id, actual_bsp);

                self.blocks.push(FlowBlock {
                    zone_id: block.zone_id,
                    height: block.height,
                    tx_count: block.tx_count,
                    birth_scroll_pos: actual_bsp,
                    birth_time: Instant::now(),
                    block_hash_hex: block.block_hash_hex.clone(),
                });
            }
        }

        self.enforce_limits();
        true
    }

    fn enforce_limits(&mut self) {
        let mut zone_counts = HashMap::<u32, usize>::new();
        for b in &self.blocks {
            *zone_counts.entry(b.zone_id).or_default() += 1;
        }
        let mut to_skip = HashMap::<u32, usize>::new();
        for (&zone, &count) in &zone_counts {
            if count > MAX_BLOCKS_PER_ZONE {
                to_skip.insert(zone, count - MAX_BLOCKS_PER_ZONE);
            }
        }
        if to_skip.is_empty() {
            return;
        }
        let mut skipped = HashMap::<u32, usize>::new();
        self.blocks.retain(|b| {
            if let Some(&skip_count) = to_skip.get(&b.zone_id) {
                let s = skipped.entry(b.zone_id).or_default();
                if *s < skip_count {
                    *s += 1;
                    return false;
                }
            }
            true
        });
    }

    fn cull(&mut self, max_x: f32) {
        let scroll_pos = self.scroll_pos;
        let seen = &mut self.seen;
        self.blocks.retain(|b| {
            let x = scroll_pos - b.birth_scroll_pos;
            let keep = x < max_x + BAR_MAX_WIDTH * 2.0;
            if !keep {
                seen.remove(&(b.zone_id, b.height));
            }
            keep
        });
    }

    pub fn render(&mut self, ui: &mut egui::Ui, state: &AppState) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        self.smoothed_speed = SCROLL_SPEED;
        self.scroll_pos += SCROLL_SPEED * dt;

        let has_new_blocks = self.ingest(state);

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

        self.handle_pan_zoom(&resp, ui);
        self.cull(rect.width() / self.camera.zoom + 200.0);

        painter.rect_filled(rect, 0.0, colors::PANEL_BG);

        self.zone_buf.clear();
        self.zone_buf.extend(self.blocks.iter().map(|b| b.zone_id));
        self.zone_buf.sort_unstable();
        self.zone_buf.dedup();

        let zone_count = self.zone_buf.len();
        let total_blocks = self.blocks.len();
        let total_tps = state.network.actual_tps;

        let entry_lead = self.smoothed_speed * ENTRY_LEAD_SECS;

        for row_idx in 0..self.zone_buf.len() {
            let zone_id = self.zone_buf[row_idx];
            let scaled_bar_h = BAR_HEIGHT * self.camera.zoom;
            let row_y = rect.top()
                + ROW_TOP_MARGIN
                + row_idx as f32 * ROW_HEIGHT * self.camera.zoom
                + self.camera.offset.y * self.camera.zoom;

            if row_y + scaled_bar_h < rect.top() || row_y > rect.bottom() {
                continue;
            }

            let mut zone_indices: Vec<usize> = self
                .blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| b.zone_id == zone_id)
                .map(|(i, _)| i)
                .collect();
            zone_indices.sort_by(|&a, &b| {
                self.blocks[b]
                    .birth_scroll_pos
                    .total_cmp(&self.blocks[a].birth_scroll_pos)
            });

            let mut prev_screen_right: Option<f32> = None;

            for &bi in &zone_indices {
                let block = &self.blocks[bi];
                if block.birth_scroll_pos > self.scroll_pos + entry_lead {
                    continue;
                }
                let x_offset = self.scroll_pos - block.birth_scroll_pos;
                let bar_w = block_width(block.tx_count);
                let scaled_w = bar_w * self.camera.zoom;

                let screen_x = rect.left()
                    + LABEL_WIDTH
                    + (x_offset + self.camera.offset.x) * self.camera.zoom;

                if screen_x > rect.right() {
                    break;
                }
                if screen_x + scaled_w < rect.left() {
                    prev_screen_right = Some(screen_x + scaled_w);
                    continue;
                }

                let scroll_frac =
                    ((x_offset + entry_lead) / entry_lead.max(1.0)).clamp(0.0, 1.0);
                let alpha_mul = 1.0 - (1.0 - scroll_frac).powi(2);

                let bar_rect = egui::Rect::from_min_size(
                    egui::pos2(screen_x, row_y),
                    egui::vec2(scaled_w, scaled_bar_h),
                );

                if let Some(prev_right) = prev_screen_right {
                    let conn_y = row_y + scaled_bar_h * 0.5;
                    let connector_alpha = (25.0 * alpha_mul) as u8;
                    painter.line_segment(
                        [
                            egui::pos2(prev_right, conn_y),
                            egui::pos2(screen_x, conn_y),
                        ],
                        egui::Stroke::new(
                            0.75,
                            with_alpha(colors::NEON_CONNECTOR, connector_alpha),
                        ),
                    );
                }

                let age_ms = block.birth_time.elapsed().as_millis();
                let blended_color = status_color_blended(age_ms);

                let bg_alpha = (BLOCK_BG_ALPHA * 255.0 * alpha_mul) as u8;
                let solid_w = scaled_w * (1.0 - FADE_ZONE_FRAC);
                let fade_w = scaled_w * FADE_ZONE_FRAC;

                let solid_rect = egui::Rect::from_min_size(
                    bar_rect.left_top(),
                    egui::vec2(solid_w, scaled_bar_h),
                );
                painter.rect_filled(
                    solid_rect,
                    0.0,
                    with_alpha(blended_color, bg_alpha),
                );

                let fade_rect = egui::Rect::from_min_size(
                    egui::pos2(bar_rect.left() + solid_w, bar_rect.top()),
                    egui::vec2(fade_w, scaled_bar_h),
                );
                draw_gradient_rect(
                    &painter,
                    fade_rect,
                    with_alpha(blended_color, bg_alpha),
                    with_alpha(blended_color, 0),
                );

                if age_ms >= VOTING_THRESHOLD_MS {
                    let glow_color = colors::BLOCK_CERTIFIED;
                    let certified_age = age_ms.saturating_sub(VOTING_THRESHOLD_MS);
                    let glow_t =
                        (certified_age as f32 / 1000.0 / GLOW_FADE_IN_SECS).min(1.0);

                    let left_alpha =
                        (GLOW_FILL_ALPHA * glow_t * 255.0 * alpha_mul) as u8;
                    let right_alpha =
                        (GLOW_TAIL_MIN_ALPHA * glow_t * 255.0 * alpha_mul) as u8;

                    draw_gradient_rect(
                        &painter,
                        bar_rect,
                        with_alpha(glow_color, left_alpha),
                        with_alpha(glow_color, right_alpha),
                    );

                    if bar_rect.right() < rect.right() {
                        let tail_w = GLOW_TAIL_LENGTH * self.camera.zoom;
                        let tail_rect = egui::Rect::from_min_size(
                            egui::pos2(bar_rect.right(), bar_rect.top()),
                            egui::vec2(tail_w, scaled_bar_h),
                        );
                        draw_gradient_rect(
                            &painter,
                            tail_rect,
                            with_alpha(glow_color, right_alpha),
                            with_alpha(glow_color, 0),
                        );
                    }

                    let certified_secs = certified_age as f32 / 1000.0;
                    let total_pulse = PULSE_FILL_SECS + PULSE_DRAIN_SECS;
                    if certified_secs < total_pulse {
                        let pulse_color = lerp_color(glow_color, egui::Color32::WHITE, 0.5);
                        let edge_w = (scaled_w * PULSE_EDGE_FRAC).max(4.0);
                        let bright =
                            with_alpha(pulse_color, (PULSE_BRIGHT_ALPHA * 255.0 * alpha_mul) as u8);
                        let transparent = with_alpha(pulse_color, 0);

                        if certified_secs < PULSE_FILL_SECS {
                            let fill_t = (certified_secs / PULSE_FILL_SECS).clamp(0.0, 1.0);
                            let lead_x = bar_rect.left() + fill_t * scaled_w;

                            let solid_right = (lead_x - edge_w).max(bar_rect.left());
                            if solid_right > bar_rect.left() {
                                let solid = egui::Rect::from_min_max(
                                    bar_rect.left_top(),
                                    egui::pos2(solid_right, bar_rect.bottom()),
                                );
                                painter.rect_filled(solid, 0.0, bright);
                            }

                            let fringe = egui::Rect::from_min_max(
                                egui::pos2(solid_right, bar_rect.top()),
                                egui::pos2(lead_x, bar_rect.bottom()),
                            );
                            draw_gradient_rect_clipped(
                                &painter, fringe, bar_rect, bright, transparent,
                            );
                        } else {
                            let drain_t = ((certified_secs - PULSE_FILL_SECS)
                                / PULSE_DRAIN_SECS)
                                .clamp(0.0, 1.0);
                            let trail_x = bar_rect.left() + drain_t * scaled_w;

                            let solid_left = (trail_x + edge_w).min(bar_rect.right());
                            if solid_left < bar_rect.right() {
                                let solid = egui::Rect::from_min_max(
                                    egui::pos2(solid_left, bar_rect.top()),
                                    bar_rect.right_bottom(),
                                );
                                painter.rect_filled(solid, 0.0, bright);
                            }

                            let fringe = egui::Rect::from_min_max(
                                egui::pos2(trail_x, bar_rect.top()),
                                egui::pos2(solid_left, bar_rect.bottom()),
                            );
                            draw_gradient_rect_clipped(
                                &painter, fringe, bar_rect, transparent, bright,
                            );
                        }
                    }
                }

                painter.rect_stroke(
                    bar_rect,
                    2.0,
                    egui::Stroke::new(
                        BORDER_STROKE,
                        with_alpha(blended_color, (BORDER_ALPHA * 255.0 * alpha_mul) as u8),
                    ),
                    egui::StrokeKind::Inside,
                );

                prev_screen_right = Some(screen_x + scaled_w);
            }

            let label_bg = egui::Rect::from_min_max(
                egui::pos2(rect.left(), row_y - ROW_HEIGHT * 0.3),
                egui::pos2(rect.left() + LABEL_WIDTH, row_y + scaled_bar_h + ROW_HEIGHT * 0.3),
            );
            painter.rect_filled(label_bg, 0.0, colors::PANEL_BG);
            painter.text(
                egui::pos2(rect.left() + 8.0, row_y + scaled_bar_h * 0.5),
                egui::Align2::LEFT_CENTER,
                format!("ZONE {zone_id}  \u{25B8}"),
                egui::FontId::proportional(11.0 * self.camera.zoom.sqrt()),
                egui::Color32::WHITE,
            );
        }

        let overlay_pos = rect.left_top() + egui::vec2(12.0, 8.0);
        let overlay_w = rect.width() - 24.0;
        egui::Area::new(egui::Id::new("blockflow_overlay"))
            .fixed_pos(overlay_pos)
            .interactable(true)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.set_width(overlay_w);
                ui.horizontal(|ui| {
                    overlay_frame().show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "BLOCKFLOW  \u{2022}  {:>2} ZONES  \u{2022}  {:>5} BLOCKS  \u{2022}  {:>6.1} TPS",
                                zone_count, total_blocks, total_tps,
                            ))
                            .strong()
                            .size(font_size::HEADING)
                            .color(colors::TEXT_HEADING)
                            .family(egui::FontFamily::Monospace),
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

        let is_animating = self.smoothed_speed > 0.5 || has_new_blocks;
        if is_animating {
            ui.ctx()
                .request_repaint_after(Duration::from_millis(32));
        }
    }

    fn handle_pan_zoom(&mut self, resp: &egui::Response, _ui: &egui::Ui) {
        if resp.dragged() {
            self.camera.offset += resp.drag_delta() / self.camera.zoom;
        }
    }
}

fn status_color_blended(age_ms: u128) -> egui::Color32 {
    let age = age_ms as f32;
    let proposed = PROPOSED_THRESHOLD_MS as f32;
    let voting = VOTING_THRESHOLD_MS as f32;

    if age < proposed {
        colors::BLOCK_PROPOSED
    } else if age < proposed + COLOR_BLEND_MS {
        let t = (age - proposed) / COLOR_BLEND_MS;
        lerp_color(colors::BLOCK_PROPOSED, colors::BLOCK_VOTING, t)
    } else if age < voting {
        colors::BLOCK_VOTING
    } else if age < voting + COLOR_BLEND_MS {
        let t = (age - voting) / COLOR_BLEND_MS;
        lerp_color(colors::BLOCK_VOTING, colors::BLOCK_CERTIFIED, t)
    } else {
        colors::BLOCK_CERTIFIED
    }
}

fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    egui::Color32::from_rgba_unmultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
        (a.a() as f32 + (b.a() as f32 - a.a() as f32) * t) as u8,
    )
}

fn pseudo_rand(a: u64, b: u64) -> f32 {
    let mut h: u64 = 0xcbf29ce484222325;
    h ^= a;
    h = h.wrapping_mul(0x100000001b3);
    h ^= b;
    h = h.wrapping_mul(0x100000001b3);
    (h % 10000) as f32 / 10000.0
}

fn block_width(tx_count: usize) -> f32 {
    (BAR_MIN_WIDTH + tx_count as f32 * BAR_WIDTH_PER_TX).min(BAR_MAX_WIDTH)
}

fn with_alpha(c: egui::Color32, a: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

fn draw_gradient_rect(
    painter: &egui::Painter,
    rect: egui::Rect,
    color_left: egui::Color32,
    color_right: egui::Color32,
) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), color_left);
    mesh.colored_vertex(rect.right_top(), color_right);
    mesh.colored_vertex(rect.right_bottom(), color_right);
    mesh.colored_vertex(rect.left_bottom(), color_left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Like `draw_gradient_rect` but clips the drawn region to `clip`.
/// Interpolates colors so the visible portion matches what the full
/// gradient would look like at those positions.
fn draw_gradient_rect_clipped(
    painter: &egui::Painter,
    rect: egui::Rect,
    clip: egui::Rect,
    color_left: egui::Color32,
    color_right: egui::Color32,
) {
    let clamped = egui::Rect::from_min_max(
        egui::pos2(rect.left().max(clip.left()), rect.top().max(clip.top())),
        egui::pos2(rect.right().min(clip.right()), rect.bottom().min(clip.bottom())),
    );
    if clamped.width() <= 0.0 || clamped.height() <= 0.0 {
        return;
    }
    let full_w = rect.width().max(1e-6);
    let t_left = ((clamped.left() - rect.left()) / full_w).clamp(0.0, 1.0);
    let t_right = ((clamped.right() - rect.left()) / full_w).clamp(0.0, 1.0);
    let cl = lerp_color(color_left, color_right, t_left);
    let cr = lerp_color(color_left, color_right, t_right);
    draw_gradient_rect(painter, clamped, cl, cr);
}


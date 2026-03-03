use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::components::tokens::colors;
use crate::components::{overlay_frame, section_heading};
use crate::state::{AppState, BlockStatus};

const BAR_HEIGHT: f32 = 10.0;
const BAR_MIN_WIDTH: f32 = 80.0;
const BAR_MAX_WIDTH: f32 = 500.0;
const BAR_WIDTH_PER_TX: f32 = 3.0;
const ROW_HEIGHT: f32 = 28.0;
const ROW_TOP_MARGIN: f32 = 36.0;
const BORDER_STROKE: f32 = 1.2;
const SUBTLE_BG_ALPHA: f32 = 0.06;
const GLOW_FILL_ALPHA: f32 = 0.35;
const GLOW_TAIL_LENGTH: f32 = 80.0;
const GLOW_TAIL_MIN_ALPHA: f32 = 0.04;
const GLOW_FADE_IN_SECS: f32 = 0.3;
const FADE_ZONE_FRAC: f32 = 0.25;
const LABEL_WIDTH: f32 = 72.0;
const BLOCK_GAP: f32 = 6.0;
const BASE_SCROLL_SPEED: f32 = 40.0;
const TPS_SPEED_FACTOR: f32 = 12.0;
const MAX_SCROLL_SPEED: f32 = 800.0;
const MAX_BLOCKS_PER_ZONE: usize = 200;
const ENTRANCE_DURATION_SECS: f32 = 0.1;

const PROPOSED_THRESHOLD_MS: u128 = 300;
const VOTING_THRESHOLD_MS: u128 = 600;

pub(crate) struct BlockflowVisualization {
    blocks: Vec<FlowBlock>,
    seen: HashSet<(u32, u64)>,
    camera: Camera,
}

struct FlowBlock {
    zone_id: u32,
    #[allow(dead_code)]
    height: u64,
    tx_count: usize,
    birth: Instant,
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
        }
    }
}

impl BlockflowVisualization {
    pub fn ingest(&mut self, state: &AppState, speed: f32) {
        let now = Instant::now();
        let first_load = self.seen.is_empty() && !state.recent_blocks.is_empty();

        let mut new_blocks = Vec::new();
        for block in state.recent_blocks.iter() {
            let key = (block.zone_id, block.height);
            if self.seen.insert(key) {
                new_blocks.push(block);
            }
        }

        if new_blocks.is_empty() {
            return;
        }

        if first_load {
            let mut by_zone: HashMap<u32, Vec<_>> = HashMap::new();
            for b in &new_blocks {
                by_zone.entry(b.zone_id).or_default().push(*b);
            }
            for (_, zone_blocks) in &mut by_zone {
                zone_blocks.sort_by(|a, b| b.height.cmp(&a.height));
                for (i, block) in zone_blocks.iter().enumerate() {
                    let stagger = Duration::from_secs_f32(
                        i as f32 * (BAR_MIN_WIDTH + BLOCK_GAP) / speed.max(1.0),
                    );
                    self.blocks.push(FlowBlock {
                        zone_id: block.zone_id,
                        height: block.height,
                        tx_count: block.tx_count,
                        birth: now.checked_sub(stagger).unwrap_or(now),
                        block_hash_hex: block.block_hash_hex.clone(),
                    });
                }
            }
        } else {
            for block in &new_blocks {
                self.blocks.push(FlowBlock {
                    zone_id: block.zone_id,
                    height: block.height,
                    tx_count: block.tx_count,
                    birth: now,
                    block_hash_hex: block.block_hash_hex.clone(),
                });
            }
        }

        self.enforce_limits();
    }

    fn enforce_limits(&mut self) {
        let mut keep_counts = HashMap::<u32, usize>::new();
        self.blocks.retain(|b| {
            let count = keep_counts.entry(b.zone_id).or_default();
            if *count < MAX_BLOCKS_PER_ZONE {
                *count += 1;
                true
            } else {
                false
            }
        });
    }

    fn cull(&mut self, max_x: f32, speed: f32) {
        let now = Instant::now();
        self.blocks.retain(|b| {
            let age = now.duration_since(b.birth).as_secs_f32();
            let x = age * speed;
            x < max_x + BAR_MAX_WIDTH * 2.0
        });
    }

    pub fn render(&mut self, ui: &mut egui::Ui, state: &AppState) {
        let speed = scroll_speed(state.network.actual_tps);
        self.ingest(state, speed);

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
        self.cull(rect.width() / self.camera.zoom + 200.0, speed);

        painter.rect_filled(rect, 0.0, colors::PANEL_BG);

        let now = Instant::now();

        let mut zones: Vec<u32> = self.blocks.iter().map(|b| b.zone_id).collect();
        zones.sort_unstable();
        zones.dedup();

        let zone_count = zones.len();
        let total_blocks = self.blocks.len();
        let total_tps = state.network.actual_tps;

        for (row_idx, &zone_id) in zones.iter().enumerate() {
            let scaled_bar_h = BAR_HEIGHT * self.camera.zoom;
            let row_y = rect.top()
                + ROW_TOP_MARGIN
                + row_idx as f32 * ROW_HEIGHT * self.camera.zoom
                + self.camera.offset.y * self.camera.zoom;

            if row_y + scaled_bar_h < rect.top() || row_y > rect.bottom() {
                continue;
            }

            let label_x = rect.left() + 8.0;
            painter.text(
                egui::pos2(label_x, row_y + scaled_bar_h * 0.5),
                egui::Align2::LEFT_CENTER,
                format!("ZONE {zone_id}  \u{25B8}"),
                egui::FontId::proportional(11.0 * self.camera.zoom.sqrt()),
                egui::Color32::WHITE,
            );

            let mut zone_blocks: Vec<&FlowBlock> = self
                .blocks
                .iter()
                .filter(|b| b.zone_id == zone_id)
                .collect();
            zone_blocks.sort_by(|a, b| b.birth.cmp(&a.birth));

            let mut prev_screen_right: Option<f32> = None;
            let mut min_next_x: f32 = 0.0;

            for block in &zone_blocks {
                let age = now.duration_since(block.birth).as_secs_f32();
                let natural_x = age * speed;
                let x_offset = natural_x.max(min_next_x);
                let bar_w = (BAR_MIN_WIDTH + block.tx_count as f32 * BAR_WIDTH_PER_TX)
                    .min(BAR_MAX_WIDTH);
                let scaled_w = bar_w * self.camera.zoom;
                min_next_x = x_offset + bar_w + BLOCK_GAP;

                let screen_x = rect.left()
                    + LABEL_WIDTH
                    + (x_offset + self.camera.offset.x) * self.camera.zoom;

                if screen_x + scaled_w < rect.left() || screen_x > rect.right() {
                    prev_screen_right = Some(screen_x + scaled_w);
                    continue;
                }

                let entrance_t = (age / ENTRANCE_DURATION_SECS).min(1.0);
                let alpha_mul = entrance_t;

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

                let status = block_status(block, now);
                let status_color = status_to_color(status);
                let age_ms = now.duration_since(block.birth).as_millis();

                let bg_alpha = (SUBTLE_BG_ALPHA * 255.0 * alpha_mul) as u8;
                let solid_w = scaled_w * (1.0 - FADE_ZONE_FRAC);
                let fade_w = scaled_w * FADE_ZONE_FRAC;

                let solid_rect = egui::Rect::from_min_size(
                    bar_rect.left_top(),
                    egui::vec2(solid_w, scaled_bar_h),
                );
                painter.rect_filled(
                    solid_rect,
                    0.0,
                    with_alpha(status_color, bg_alpha),
                );

                let fade_rect = egui::Rect::from_min_size(
                    egui::pos2(bar_rect.left() + solid_w, bar_rect.top()),
                    egui::vec2(fade_w, scaled_bar_h),
                );
                draw_gradient_rect(
                    &painter,
                    fade_rect,
                    with_alpha(status_color, bg_alpha),
                    with_alpha(status_color, 0),
                );

                if status == BlockStatus::Certified {
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
                        with_alpha(status_color, left_alpha),
                        with_alpha(status_color, right_alpha),
                    );

                    let tail_w = GLOW_TAIL_LENGTH * self.camera.zoom;
                    let tail_rect = egui::Rect::from_min_size(
                        egui::pos2(bar_rect.right(), bar_rect.top()),
                        egui::vec2(tail_w, scaled_bar_h),
                    );
                    draw_gradient_rect(
                        &painter,
                        tail_rect,
                        with_alpha(status_color, right_alpha),
                        with_alpha(status_color, 0),
                    );
                }

                painter.rect_stroke(
                    bar_rect,
                    2.0,
                    egui::Stroke::new(
                        BORDER_STROKE,
                        with_alpha(status_color, (255.0 * alpha_mul) as u8),
                    ),
                    egui::StrokeKind::Inside,
                );

                prev_screen_right = Some(screen_x + scaled_w);
            }
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
                        section_heading(
                            ui,
                            &format!(
                                "BLOCKFLOW  \u{2022}  {zone_count} zones  \u{2022}  {total_blocks} blocks  \u{2022}  {total_tps:.1} tps"
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

        ui.ctx().request_repaint();
    }

    fn handle_pan_zoom(&mut self, resp: &egui::Response, ui: &egui::Ui) {
        if resp.dragged() {
            self.camera.offset += resp.drag_delta() / self.camera.zoom;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let factor = 1.0 + scroll * 0.003;
                self.camera.zoom = (self.camera.zoom * factor).clamp(0.3, 4.0);
            }
        }
    }
}

fn scroll_speed(tps: f64) -> f32 {
    (BASE_SCROLL_SPEED + (tps as f32).sqrt() * TPS_SPEED_FACTOR).min(MAX_SCROLL_SPEED)
}

fn block_status(block: &FlowBlock, now: Instant) -> BlockStatus {
    let age_ms = now.duration_since(block.birth).as_millis();
    if age_ms < PROPOSED_THRESHOLD_MS {
        BlockStatus::Proposed
    } else if age_ms < VOTING_THRESHOLD_MS {
        BlockStatus::Voting
    } else {
        BlockStatus::Certified
    }
}

fn status_to_color(status: BlockStatus) -> egui::Color32 {
    match status {
        BlockStatus::Proposed => colors::NEON_CYAN,
        BlockStatus::Voting => colors::NEON_AMBER,
        BlockStatus::Certified => colors::NEON_GREEN,
    }
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


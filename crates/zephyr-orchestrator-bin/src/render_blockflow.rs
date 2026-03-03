use std::time::Instant;

use eframe::egui;

use crate::components::tokens::{colors, font_size};
use crate::components::{overlay_frame, section_heading};
use crate::state::{AppState, BlockStatus};

const BLOCK_SIZE: f32 = 56.0;
const TX_SQUARE: f32 = 5.0;
const TX_GAP: f32 = 2.0;
const TX_PADDING: f32 = 4.0;
const ROW_HEIGHT: f32 = 80.0;
const ROW_TOP_MARGIN: f32 = 50.0;
const LABEL_WIDTH: f32 = 72.0;
const SCROLL_SPEED: f32 = 60.0;
const MAX_BLOCKS_PER_ZONE: usize = 200;
const ENTRANCE_DURATION_SECS: f32 = 0.1;

const PROPOSED_THRESHOLD_MS: u128 = 300;
const VOTING_THRESHOLD_MS: u128 = 600;

pub(crate) struct BlockflowVisualization {
    blocks: Vec<FlowBlock>,
    last_block_count: usize,
    camera: Camera,
}

struct FlowBlock {
    zone_id: u32,
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
            last_block_count: 0,
            camera: Camera::default(),
        }
    }
}

impl BlockflowVisualization {
    pub fn ingest(&mut self, state: &AppState) {
        let total = state.recent_blocks.len();
        if total <= self.last_block_count {
            self.last_block_count = total;
            return;
        }

        let new_count = total - self.last_block_count;
        let now = Instant::now();
        for block in state.recent_blocks.iter().skip(self.last_block_count).take(new_count) {
            self.blocks.push(FlowBlock {
                zone_id: block.zone_id,
                height: block.height,
                tx_count: block.tx_count,
                birth: now,
                block_hash_hex: block.block_hash_hex.clone(),
            });
        }
        self.last_block_count = total;

        self.enforce_limits();
    }

    fn enforce_limits(&mut self) {
        let mut zone_counts = std::collections::HashMap::<u32, usize>::new();
        for b in self.blocks.iter().rev() {
            *zone_counts.entry(b.zone_id).or_default() += 1;
        }

        let mut keep_counts = std::collections::HashMap::<u32, usize>::new();
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

    fn cull(&mut self, max_x: f32) {
        let now = Instant::now();
        self.blocks.retain(|b| {
            let age = now.duration_since(b.birth).as_secs_f32();
            let x = age * SCROLL_SPEED;
            x < max_x + BLOCK_SIZE * 2.0
        });
    }

    pub fn render(&mut self, ui: &mut egui::Ui, state: &AppState) {
        self.ingest(state);

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

        let now = Instant::now();

        let mut zones: Vec<u32> = self.blocks.iter().map(|b| b.zone_id).collect();
        zones.sort_unstable();
        zones.dedup();

        let zone_count = zones.len();
        let total_blocks = self.blocks.len();
        let total_tps = state.network.actual_tps;

        for (row_idx, &zone_id) in zones.iter().enumerate() {
            let row_y = rect.top()
                + ROW_TOP_MARGIN
                + row_idx as f32 * ROW_HEIGHT * self.camera.zoom
                + self.camera.offset.y * self.camera.zoom;

            if row_y + BLOCK_SIZE * self.camera.zoom < rect.top()
                || row_y > rect.bottom()
            {
                continue;
            }

            let label_x = rect.left() + 8.0;
            painter.text(
                egui::pos2(label_x, row_y + BLOCK_SIZE * self.camera.zoom * 0.5),
                egui::Align2::LEFT_CENTER,
                format!("ZONE {zone_id}  \u{25B8}"),
                egui::FontId::proportional(11.0 * self.camera.zoom.sqrt()),
                egui::Color32::WHITE,
            );

            let zone_blocks: Vec<&FlowBlock> = self
                .blocks
                .iter()
                .filter(|b| b.zone_id == zone_id)
                .collect();

            let mut prev_screen_right: Option<f32> = None;

            for block in &zone_blocks {
                let age = now.duration_since(block.birth).as_secs_f32();
                let x_offset = age * SCROLL_SPEED;
                let screen_x = rect.left()
                    + LABEL_WIDTH
                    + (x_offset + self.camera.offset.x) * self.camera.zoom;
                let screen_y = row_y;
                let scaled_size = BLOCK_SIZE * self.camera.zoom;

                if screen_x + scaled_size < rect.left() || screen_x > rect.right() {
                    prev_screen_right = Some(screen_x + scaled_size);
                    continue;
                }

                let entrance_t = (age / ENTRANCE_DURATION_SECS).min(1.0);
                let scale = 0.5 + 0.5 * entrance_t;
                let alpha_mul = entrance_t;
                let draw_size = scaled_size * scale;
                let center_offset = (scaled_size - draw_size) * 0.5;

                let block_rect = egui::Rect::from_min_size(
                    egui::pos2(screen_x + center_offset, screen_y + center_offset),
                    egui::vec2(draw_size, draw_size),
                );

                if let Some(prev_right) = prev_screen_right {
                    let conn_y = screen_y + scaled_size * 0.5;
                    painter.line_segment(
                        [
                            egui::pos2(prev_right, conn_y),
                            egui::pos2(screen_x + center_offset, conn_y),
                        ],
                        egui::Stroke::new(1.5, colors::NEON_CONNECTOR),
                    );
                }

                let status = block_status(block, now);
                let status_color = status_to_color(status);
                let fill_color = with_alpha(status_color, (0.15 * 255.0 * alpha_mul) as u8);
                let border_color = with_alpha(status_color, (255.0 * alpha_mul) as u8);

                painter.rect_filled(block_rect, 3.0, fill_color);
                painter.rect_stroke(
                    block_rect,
                    3.0,
                    egui::Stroke::new(1.5, border_color),
                    egui::StrokeKind::Outside,
                );

                if draw_size > 20.0 {
                    draw_transactions(&painter, block_rect, block.tx_count, alpha_mul);
                }

                if draw_size > 30.0 {
                    let label_pos = egui::pos2(
                        block_rect.right() - 3.0,
                        block_rect.bottom() - 3.0,
                    );
                    painter.text(
                        label_pos,
                        egui::Align2::RIGHT_BOTTOM,
                        format!("#{}", block.height),
                        egui::FontId::proportional(
                            (font_size::TINY * self.camera.zoom.sqrt()).max(7.0),
                        ),
                        with_alpha(egui::Color32::WHITE, (180.0 * alpha_mul) as u8),
                    );
                }

                prev_screen_right = Some(screen_x + center_offset + draw_size);
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

fn draw_transactions(painter: &egui::Painter, block_rect: egui::Rect, tx_count: usize, alpha: f32) {
    if tx_count == 0 {
        return;
    }

    let inner = block_rect.shrink(TX_PADDING);
    let cols = ((inner.width() + TX_GAP) / (TX_SQUARE + TX_GAP)).floor() as usize;
    let rows = ((inner.height() - 12.0 + TX_GAP) / (TX_SQUARE + TX_GAP)).floor() as usize;
    if cols == 0 || rows == 0 {
        return;
    }

    let max_visible = cols * rows;
    let draw_count = tx_count.min(max_visible);
    let tx_color = with_alpha(egui::Color32::WHITE, (178.0 * alpha) as u8);

    for i in 0..draw_count {
        let col = i % cols;
        let row = i / cols;
        if row >= rows {
            break;
        }
        let x = inner.left() + col as f32 * (TX_SQUARE + TX_GAP);
        let y = inner.top() + row as f32 * (TX_SQUARE + TX_GAP);
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(TX_SQUARE, TX_SQUARE)),
            1.0,
            tx_color,
        );
    }
}

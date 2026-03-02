use std::collections::{HashMap, HashSet};

use eframe::egui;
use grid_core::ProgramId;

use crate::app::ZodeApp;
use crate::components::tokens::{self, colors, font_size, spacing};
use crate::components::{loading_state, muted_label, section};
use crate::state::{DetailSelection, StateSnapshot};

const CARD_WIDTH: f32 = 260.0;
const CARD_HEIGHT: f32 = 100.0;

pub(crate) fn render_programs(app: &mut ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let Some(ref status) = state.status else {
        loading_state(ui);
        return;
    };

    let Some(ref zode) = app.zode else {
        loading_state(ui);
        return;
    };

    let known_programs: HashMap<ProgramId, &str> = zode::default_program_ids()
        .into_iter()
        .map(|(name, pid)| (pid, name))
        .collect();

    let subscribed: HashSet<ProgramId> = status
        .topics
        .iter()
        .filter_map(|t| t.strip_prefix("prog/"))
        .filter_map(|hex| ProgramId::from_hex(hex).ok())
        .collect();

    let registry = zode.service_registry();
    let services = registry.try_lock().ok().map(|r| r.list_services());

    let mut service_program_ids: HashSet<ProgramId> = HashSet::new();

    if let Some(ref services) = services {
        for svc in services {
            let mut entries: Vec<ProgramEntry> = Vec::new();

            for desc in &svc.descriptor.owned_programs {
                if let Ok(pid) = desc.program_id() {
                    entries.push(ProgramEntry {
                        name: desc.name.clone(),
                        version: Some(desc.version.clone()),
                        program_id: pid,
                        relation: ProgramRelation::Owned,
                        subscribed: subscribed.contains(&pid),
                    });
                    service_program_ids.insert(pid);
                }
            }

            for &pid in &svc.descriptor.required_programs {
                let name = known_programs
                    .get(&pid)
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| short_hex(&pid));
                entries.push(ProgramEntry {
                    name,
                    version: None,
                    program_id: pid,
                    relation: ProgramRelation::Required,
                    subscribed: subscribed.contains(&pid),
                });
                service_program_ids.insert(pid);
            }

            if !entries.is_empty() {
                let title = format!("{} v{}", svc.descriptor.name, svc.descriptor.version);
                render_service_programs(
                    ui,
                    &title,
                    svc.running,
                    &entries,
                    &mut app.detail_selection,
                );
            }
        }
    }

    let standalone: Vec<ProgramEntry> = subscribed
        .iter()
        .filter(|pid| !service_program_ids.contains(pid))
        .map(|&pid| {
            let name = known_programs
                .get(&pid)
                .map(|n| n.to_string())
                .unwrap_or_else(|| short_hex(&pid));
            ProgramEntry {
                name,
                version: None,
                program_id: pid,
                relation: ProgramRelation::Default,
                subscribed: true,
            }
        })
        .collect();

    if !standalone.is_empty() {
        render_service_programs(
            ui,
            "Default Programs",
            true,
            &standalone,
            &mut app.detail_selection,
        );
    }

    if subscribed.is_empty() && services.as_ref().is_none_or(|s| s.is_empty()) {
        section(ui, "Programs", |ui| {
            muted_label(ui, "No programs subscribed.");
            ui.add_space(spacing::SM);
            muted_label(ui, "Programs will appear here when enabled in Settings.");
        });
    }
}

fn short_hex(pid: &ProgramId) -> String {
    let hex = pid.to_hex();
    format!("{}…", &hex[..12.min(hex.len())])
}

#[derive(Clone, Copy, PartialEq)]
enum ProgramRelation {
    Owned,
    Required,
    Default,
}

struct ProgramEntry {
    name: String,
    version: Option<String>,
    program_id: ProgramId,
    relation: ProgramRelation,
    subscribed: bool,
}

fn render_service_programs(
    ui: &mut egui::Ui,
    title: &str,
    service_running: bool,
    entries: &[ProgramEntry],
    detail_selection: &mut Option<DetailSelection>,
) {
    section(ui, title, |ui| {
        if !service_running {
            muted_label(ui, "Service is stopped.");
            ui.add_space(spacing::SM);
        }

        let avail_w = ui.available_width();
        let cols = ((avail_w + spacing::MD) / (CARD_WIDTH + spacing::MD))
            .floor()
            .max(1.0) as usize;

        for row in entries.chunks(cols) {
            ui.horizontal(|ui| {
                for entry in row {
                    program_card(ui, entry, detail_selection);
                    ui.add_space(spacing::MD);
                }
            });
            ui.add_space(spacing::MD);
        }
    });
}

fn program_card(
    ui: &mut egui::Ui,
    entry: &ProgramEntry,
    detail_selection: &mut Option<DetailSelection>,
) {
    let border_color = colors::BORDER;

    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(CARD_WIDTH, CARD_HEIGHT), egui::Sense::click());

    let painter = ui.painter_at(rect);

    painter.rect(
        rect,
        0.0,
        colors::SURFACE_DARK,
        egui::Stroke::new(tokens::STROKE_DEFAULT, border_color),
        egui::StrokeKind::Inside,
    );

    if resp.hovered() {
        painter.rect(
            rect,
            0.0,
            egui::Color32::from_white_alpha(4),
            egui::Stroke::NONE,
            egui::StrokeKind::Inside,
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let pad = spacing::LG;
    let inner = rect.shrink(pad);

    painter.text(
        inner.left_top(),
        egui::Align2::LEFT_TOP,
        &entry.name,
        egui::FontId::proportional(font_size::SUBTITLE),
        colors::TEXT_HEADING,
    );

    if let Some(ref ver) = entry.version {
        painter.text(
            egui::pos2(inner.left(), inner.top() + 20.0),
            egui::Align2::LEFT_TOP,
            format!("v{ver}"),
            egui::FontId::proportional(font_size::SMALL),
            colors::TEXT_MUTED,
        );
    }

    let relation_label = match entry.relation {
        ProgramRelation::Owned => "OWNED",
        ProgramRelation::Required => "REQUIRED",
        ProgramRelation::Default => "DEFAULT",
    };
    let relation_color = match entry.relation {
        ProgramRelation::Owned => colors::ACCENT,
        ProgramRelation::Required => colors::TEXT_SECONDARY,
        ProgramRelation::Default => colors::TEXT_MUTED,
    };
    painter.text(
        egui::pos2(inner.right(), inner.top()),
        egui::Align2::RIGHT_TOP,
        relation_label,
        egui::FontId::proportional(font_size::SMALL),
        relation_color,
    );

    let hex = entry.program_id.to_hex();
    let short_id = &hex[..8.min(hex.len())];
    painter.text(
        egui::pos2(inner.left(), inner.bottom() - 14.0),
        egui::Align2::LEFT_TOP,
        format!("{short_id}…"),
        egui::FontId::proportional(font_size::SMALL),
        colors::TEXT_SECONDARY,
    );

    let status_text = if entry.subscribed {
        "SUBSCRIBED"
    } else {
        "INACTIVE"
    };
    let status_color = if entry.subscribed {
        colors::CONNECTED
    } else {
        colors::TEXT_MUTED
    };
    painter.text(
        egui::pos2(inner.right(), inner.bottom() - 14.0),
        egui::Align2::RIGHT_TOP,
        status_text,
        egui::FontId::proportional(font_size::SMALL),
        status_color,
    );

    if resp.clicked() {
        *detail_selection = Some(DetailSelection::Program(entry.program_id));
    }
}

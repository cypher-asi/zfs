use std::collections::HashMap;

use eframe::egui;
use grid_core::{FieldSchema, ProgramId, ProofSystem};
use grid_programs_interlink::InterlinkDescriptor;
use grid_programs_zid::ZidDescriptor;
use grid_service::ServiceId;

use crate::app::ZodeApp;
use crate::components::section_heading;
use crate::components::tokens::{self, colors, font_size, spacing};
use crate::state::DetailSelection;

fn detail_close_button(ui: &mut egui::Ui) -> bool {
    let btn = ui.add(
        egui::Button::new(
            egui::RichText::new(egui_phosphor::regular::X)
                .size(12.0)
                .color(colors::TEXT_SECONDARY),
        )
        .frame(false),
    );
    if btn.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    btn.clicked()
}

fn detail_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(font_size::SMALL)
            .color(egui::Color32::WHITE),
    );
}

fn detail_field(ui: &mut egui::Ui, key: &str, value: &str) {
    detail_label(ui, key);
    ui.add(egui::Label::new(value).wrap());
    ui.add_space(spacing::MD);
}

fn detail_field_copyable(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        detail_label(ui, key);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::components::copy_button(ui, value);
        });
    });
    ui.add(egui::Label::new(egui::RichText::new(value).monospace()).wrap());
    ui.add_space(spacing::MD);
}

fn detail_field_colored(ui: &mut egui::Ui, key: &str, value: &str, color: egui::Color32) {
    detail_label(ui, key);
    ui.label(egui::RichText::new(value).monospace().color(color));
    ui.add_space(spacing::MD);
}

struct ProgramMeta {
    version: u32,
    field_schema: FieldSchema,
    proof_required: bool,
    proof_system: Option<ProofSystem>,
}

fn build_program_meta() -> HashMap<ProgramId, ProgramMeta> {
    let mut map = HashMap::new();

    let mut insert = |version: u32,
                      schema: FieldSchema,
                      proof_required: bool,
                      proof_system: Option<ProofSystem>,
                      pid: Result<ProgramId, grid_core::GridError>| {
        if let Ok(pid) = pid {
            map.insert(
                pid,
                ProgramMeta {
                    version,
                    field_schema: schema,
                    proof_required,
                    proof_system,
                },
            );
        }
    };

    let zid_schema = ZidDescriptor::field_schema();
    let il_schema = InterlinkDescriptor::field_schema();

    let z1 = ZidDescriptor::v1();
    insert(
        1,
        zid_schema.clone(),
        z1.proof_required,
        z1.proof_system,
        z1.program_id(),
    );

    let z2 = ZidDescriptor::v2();
    insert(
        2,
        zid_schema,
        z2.proof_required,
        z2.proof_system,
        z2.program_id(),
    );

    let i1 = InterlinkDescriptor::v1();
    insert(
        1,
        il_schema.clone(),
        i1.proof_required,
        i1.proof_system,
        i1.program_id(),
    );

    let i2 = InterlinkDescriptor::v2();
    insert(
        2,
        il_schema,
        i2.proof_required,
        i2.proof_system,
        i2.program_id(),
    );

    map
}

pub(crate) fn render_detail(app: &mut ZodeApp, ui: &mut egui::Ui) {
    let selection = match app.detail_selection {
        Some(ref sel) => sel.clone(),
        None => return,
    };

    let mut close = false;
    let max = ui.max_rect();

    let prev_clip = ui.clip_rect();
    ui.set_clip_rect(prev_clip.intersect(egui::Rect::from_x_y_ranges(
        max.left()..=max.right(),
        prev_clip.top()..=prev_clip.bottom(),
    )));

    let mut prepared = egui::Frame::default()
        .fill(colors::SURFACE)
        .corner_radius(0.0)
        .inner_margin(spacing::XL)
        .outer_margin(egui::Margin::symmetric(0, 0))
        .begin(ui);

    {
        let ui = &mut prepared.content_ui;
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = detail_close_button(ui);
            });
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_width(ui.available_width());
            match selection {
                DetailSelection::Service(service_id) => {
                    render_service_detail(app, ui, &service_id);
                }
                DetailSelection::Program(program_id) => {
                    render_program_detail(app, ui, &program_id);
                }
            }
        });
    }

    let resp = prepared.end(ui);

    let border_rect = egui::Rect::from_min_max(
        egui::pos2(max.left(), resp.rect.top()),
        egui::pos2(max.right(), resp.rect.bottom()),
    );
    ui.painter().rect_stroke(
        border_rect,
        0.0,
        tokens::border_stroke(),
        egui::StrokeKind::Inside,
    );

    ui.set_clip_rect(prev_clip);

    if close {
        app.detail_closing = true;
    }
}

fn render_service_detail(app: &ZodeApp, ui: &mut egui::Ui, service_id: &ServiceId) {
    let Some(ref zode) = app.zode else {
        ui.label("Zode not running.");
        return;
    };

    let registry = zode.service_registry();
    let Ok(registry) = registry.try_read() else {
        ui.label("Loading…");
        return;
    };

    let services = registry.list_services();
    drop(registry);

    let Some(svc) = services.iter().find(|s| s.id == *service_id) else {
        ui.label("Service not found.");
        return;
    };

    let known_programs: HashMap<ProgramId, &str> = zode::default_program_ids()
        .into_iter()
        .map(|(name, pid)| (pid, name))
        .collect();

    let id_hex = svc.id.to_hex();

    section_heading(
        ui,
        &format!("{} v{}", svc.descriptor.name, svc.descriptor.version),
    );

    if !svc.descriptor.summary.is_empty() {
        ui.add_space(spacing::SM);
        ui.label(
            egui::RichText::new(&svc.descriptor.summary)
                .size(font_size::BODY)
                .color(colors::TEXT_SECONDARY),
        );
    }

    ui.add_space(10.0);

    let (status_text, status_color) = if svc.running {
        ("RUNNING", colors::CONNECTED)
    } else {
        ("INACTIVE", colors::ERROR)
    };

    detail_field_copyable(ui, "Service ID", &id_hex);
    detail_field_colored(ui, "Status", status_text, status_color);

    if let Ok(topic) = svc.descriptor.topic() {
        detail_field_copyable(ui, "Topic", &topic);
    }

    detail_field_copyable(ui, "Endpoint", &format!("/services/{}/", &id_hex));

    ui.add_space(spacing::LG);

    if !svc.descriptor.required_programs.is_empty() {
        ui.label(
            egui::RichText::new("REQUIRED PROGRAMS")
                .strong()
                .size(font_size::BODY)
                .color(egui::Color32::WHITE),
        );
        ui.add_space(spacing::SM);
        for pid in &svc.descriptor.required_programs {
            let hex = pid.to_hex();
            let name = known_programs
                .get(pid)
                .map(|n| format!("{n} ({}…)", &hex[..8.min(hex.len())]))
                .unwrap_or_else(|| format!("{}…", &hex[..12.min(hex.len())]));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("•").color(colors::TEXT_MUTED));
                ui.add(
                    egui::Label::new(egui::RichText::new(&name).monospace())
                        .truncate()
                        .wrap_mode(egui::TextWrapMode::Truncate),
                );
            });
        }
        ui.add_space(spacing::MD);
    }

    if !svc.descriptor.owned_programs.is_empty() {
        ui.label(
            egui::RichText::new("OWNED PROGRAMS")
                .strong()
                .size(font_size::BODY)
                .color(egui::Color32::WHITE),
        );
        ui.add_space(spacing::SM);
        for desc in &svc.descriptor.owned_programs {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("•").color(colors::TEXT_MUTED));
                ui.label(format!("{} v{}", desc.name, desc.version));
            });
        }
        ui.add_space(spacing::MD);
    }

    if !svc.routes.is_empty() {
        ui.label(
            egui::RichText::new("ROUTES")
                .strong()
                .size(font_size::BODY)
                .color(egui::Color32::WHITE),
        );
        ui.add_space(spacing::SM);

        for route in &svc.routes {
            ui.horizontal(|ui| {
                ui.monospace(route.method);
                ui.add(
                    egui::Label::new(egui::RichText::new(route.path).monospace())
                        .truncate()
                        .wrap_mode(egui::TextWrapMode::Truncate),
                );
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(route.description)
                        .small()
                        .color(colors::TEXT_MUTED),
                )
                .truncate()
                .wrap_mode(egui::TextWrapMode::Truncate),
            );
            ui.add_space(spacing::SM);
        }
    }
}

fn render_program_detail(app: &ZodeApp, ui: &mut egui::Ui, program_id: &ProgramId) {
    let Some(ref zode) = app.zode else {
        ui.label("Zode not running.");
        return;
    };

    let meta_map = build_program_meta();
    let known_programs: HashMap<ProgramId, &str> = zode::default_program_ids()
        .into_iter()
        .map(|(name, pid)| (pid, name))
        .collect();

    let id_hex = program_id.to_hex();
    let name = known_programs
        .get(program_id)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}…", &id_hex[..12.min(id_hex.len())]));

    let status = zode.status();
    let subscribed = status.topics.iter().any(|t| t == &format!("prog/{id_hex}"));

    let registry = zode.service_registry();
    let relation = if let Ok(reg) = registry.try_read() {
        let services = reg.list_services();
        drop(reg);
        determine_relation(program_id, &services)
    } else {
        "UNKNOWN"
    };

    let meta = meta_map.get(program_id);
    let version_str = meta.map(|m| format!("v{}", m.version)).unwrap_or_default();

    let heading = if version_str.is_empty() {
        name.clone()
    } else {
        format!("{name} {version_str}")
    };

    section_heading(ui, &heading);
    ui.add_space(10.0);

    let (status_text, status_color) = if subscribed {
        ("SUBSCRIBED", colors::CONNECTED)
    } else {
        ("INACTIVE", colors::ERROR)
    };

    detail_field_copyable(ui, "Program ID", &id_hex);
    detail_field_colored(ui, "Status", status_text, status_color);
    detail_field(ui, "Relation", relation);
    detail_field_copyable(ui, "Topic", &format!("prog/{id_hex}"));

    ui.add_space(spacing::LG);

    if let Some(meta) = meta {
        let proof_label = match meta.proof_system {
            Some(ProofSystem::Groth16) => "Groth16",
            Some(ProofSystem::None) | None => "None",
        };

        detail_field(
            ui,
            "Proof Required",
            if meta.proof_required { "Yes" } else { "No" },
        );
        detail_field(ui, "Proof System", proof_label);

        let hash = meta.field_schema.schema_hash();
        detail_field_copyable(ui, "Schema Hash", &hex::encode(hash));

        ui.add_space(spacing::LG);

        ui.label(
            egui::RichText::new("FIELD SCHEMA")
                .strong()
                .size(font_size::BODY)
                .color(egui::Color32::WHITE),
        );
        ui.add_space(spacing::SM);

        for field in &meta.field_schema.fields {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(&field.key).monospace())
                        .truncate()
                        .wrap_mode(egui::TextWrapMode::Truncate),
                );
            });
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(cbor_type_label(field.value_type))
                        .small()
                        .color(colors::TEXT_MUTED),
                );
                if field.optional {
                    ui.label(
                        egui::RichText::new("optional")
                            .small()
                            .color(colors::TEXT_MUTED),
                    );
                }
            });
            ui.add_space(spacing::XS);
        }
    }
}

fn determine_relation(pid: &ProgramId, services: &[grid_service::ServiceInfo]) -> &'static str {
    for svc in services {
        for desc in &svc.descriptor.owned_programs {
            if desc.program_id().ok().as_ref() == Some(pid) {
                return "OWNED";
            }
        }
        if svc.descriptor.required_programs.contains(pid) {
            return "REQUIRED";
        }
    }
    "DEFAULT"
}

fn cbor_type_label(ct: grid_core::CborType) -> &'static str {
    match ct {
        grid_core::CborType::UnsignedInt => "UnsignedInt",
        grid_core::CborType::NegativeInt => "NegativeInt",
        grid_core::CborType::ByteString => "ByteString",
        grid_core::CborType::TextString => "TextString",
        grid_core::CborType::Array => "Array",
        grid_core::CborType::Map => "Map",
        grid_core::CborType::Bool => "Bool",
        grid_core::CborType::Null => "Null",
    }
}

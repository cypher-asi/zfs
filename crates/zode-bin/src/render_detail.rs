use std::collections::HashMap;

use eframe::egui;
use grid_core::{FieldSchema, ProgramId, ProofSystem};
use grid_programs_interlink::InterlinkDescriptor;
use grid_programs_zid::ZidDescriptor;
use grid_service::ServiceId;

use crate::app::ZodeApp;
use crate::components::tokens::{colors, font_size, spacing};
use crate::components::{info_grid, kv_row, kv_row_copyable, section_heading};
use crate::state::DetailSelection;

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

fn render_service_detail(app: &ZodeApp, ui: &mut egui::Ui, service_id: &ServiceId) {
    let Some(ref zode) = app.zode else {
        ui.label("Zode not running.");
        return;
    };

    let registry = zode.service_registry();
    let Ok(registry) = registry.try_lock() else {
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
    ui.add_space(spacing::MD);

    let (status_text, status_color) = if svc.running {
        ("RUNNING", colors::CONNECTED)
    } else {
        ("INACTIVE", colors::ERROR)
    };

    info_grid(ui, "svc_detail", |ui| {
        kv_row_copyable(ui, "Service ID", &id_hex);
        kv_row(ui, "Status", status_text);
        ui.painter().text(
            ui.cursor().left_top(),
            egui::Align2::LEFT_TOP,
            "",
            egui::FontId::default(),
            egui::Color32::TRANSPARENT,
        );

        if let Ok(topic) = svc.descriptor.topic() {
            kv_row(ui, "Topic", &topic);
        }

        kv_row(ui, "Endpoint", &format!("/services/{}/", &id_hex));
    });

    ui.add_space(spacing::LG);

    if !svc.descriptor.required_programs.is_empty() {
        ui.label(
            egui::RichText::new("Required Programs")
                .strong()
                .size(font_size::BODY)
                .color(colors::TEXT_HEADING),
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
                ui.monospace(&name);
            });
        }
        ui.add_space(spacing::MD);
    }

    if !svc.descriptor.owned_programs.is_empty() {
        ui.label(
            egui::RichText::new("Owned Programs")
                .strong()
                .size(font_size::BODY)
                .color(colors::TEXT_HEADING),
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
            egui::RichText::new("Routes")
                .strong()
                .size(font_size::BODY)
                .color(colors::TEXT_HEADING),
        );
        ui.add_space(spacing::SM);

        egui::Grid::new("svc_routes_grid")
            .num_columns(3)
            .spacing([spacing::LG, spacing::XS])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("METHOD")
                        .small()
                        .color(colors::TEXT_MUTED),
                );
                ui.label(
                    egui::RichText::new("PATH")
                        .small()
                        .color(colors::TEXT_MUTED),
                );
                ui.label(
                    egui::RichText::new("DESCRIPTION")
                        .small()
                        .color(colors::TEXT_MUTED),
                );
                ui.end_row();

                for route in &svc.routes {
                    ui.monospace(route.method);
                    ui.monospace(route.path);
                    ui.label(route.description);
                    ui.end_row();
                }
            });
    }

    // Status indicator dot next to heading
    let heading_rect = ui.min_rect();
    ui.painter().circle_filled(
        egui::pos2(heading_rect.left() - 12.0, heading_rect.top() + 10.0),
        4.0,
        status_color,
    );
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
    let relation = if let Ok(reg) = registry.try_lock() {
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
    ui.add_space(spacing::MD);

    let (status_text, status_color) = if subscribed {
        ("SUBSCRIBED", colors::CONNECTED)
    } else {
        ("INACTIVE", colors::TEXT_MUTED)
    };

    info_grid(ui, "prog_detail", |ui| {
        kv_row_copyable(ui, "Program ID", &id_hex);
        kv_row(ui, "Status", status_text);
        kv_row(ui, "Relation", relation);
        kv_row(ui, "GossipSub Topic", &format!("prog/{id_hex}"));
    });

    ui.add_space(spacing::LG);

    if let Some(meta) = meta {
        let proof_label = match meta.proof_system {
            Some(ProofSystem::Groth16) => "Groth16",
            Some(ProofSystem::None) | None => "None",
        };

        info_grid(ui, "prog_proof", |ui| {
            kv_row(
                ui,
                "Proof Required",
                if meta.proof_required { "Yes" } else { "No" },
            );
            kv_row(ui, "Proof System", proof_label);

            let hash = meta.field_schema.schema_hash();
            kv_row_copyable(ui, "Schema Hash", &hex::encode(hash));
        });

        ui.add_space(spacing::LG);

        ui.label(
            egui::RichText::new("Field Schema")
                .strong()
                .size(font_size::BODY)
                .color(colors::TEXT_HEADING),
        );
        ui.add_space(spacing::SM);

        egui::Grid::new("prog_schema_grid")
            .num_columns(3)
            .spacing([spacing::LG, spacing::XS])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("FIELD")
                        .small()
                        .color(colors::TEXT_MUTED),
                );
                ui.label(
                    egui::RichText::new("CBOR TYPE")
                        .small()
                        .color(colors::TEXT_MUTED),
                );
                ui.label(
                    egui::RichText::new("OPTIONAL")
                        .small()
                        .color(colors::TEXT_MUTED),
                );
                ui.end_row();

                for field in &meta.field_schema.fields {
                    ui.monospace(&field.key);
                    ui.label(cbor_type_label(field.value_type));
                    ui.label(if field.optional { "Yes" } else { "No" });
                    ui.end_row();
                }
            });
    }

    // Status indicator dot
    let heading_rect = ui.min_rect();
    ui.painter().circle_filled(
        egui::pos2(heading_rect.left() - 12.0, heading_rect.top() + 10.0),
        4.0,
        status_color,
    );
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

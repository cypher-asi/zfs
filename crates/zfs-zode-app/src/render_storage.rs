use std::collections::HashMap;

use eframe::egui;
use zfs_core::ShapeProof;
use zfs_storage::SectorStore;

use crate::app::ZodeApp;
use crate::components::{
    colors, copy_button, field_label, hint_label, info_grid, kv_row, muted_label, section,
};
use crate::helpers::format_bytes;
use crate::state::StateSnapshot;

pub(crate) fn render_storage(app: &ZodeApp, ui: &mut egui::Ui, state: &StateSnapshot) {
    let Some(ref status) = state.status else {
        ui.vertical_centered(|ui| {
            let avail = ui.available_height();
            ui.add_space((avail / 2.0 - 20.0).max(0.0));
            ui.spinner();
            ui.label("Loading...");
        });
        return;
    };

    let Some(ref zode) = app.zode else {
        return;
    };

    let known_programs: HashMap<zfs_core::ProgramId, &str> = zfs_programs::default_program_ids()
        .into_iter()
        .map(|(name, pid)| (pid, name))
        .collect();

    render_stats_section(ui, zode);
    ui.add_space(8.0);
    render_programs_section(ui, zode, status, &known_programs);
}

fn render_stats_section(ui: &mut egui::Ui, zode: &zfs_zode::Zode) {
    section(ui, "Sector Storage", |ui| {
        match zode.storage().sector_stats() {
            Ok(stats) => {
                info_grid(ui, "sector_stats_grid", |ui| {
                    kv_row(ui, "Sectors", &format!("{}", stats.sector_count));
                    kv_row(ui, "Entries", &format!("{}", stats.entry_count));
                    kv_row(ui, "Size", &format_bytes(stats.sector_size_bytes));
                    kv_row(ui, "Protocol", "/zfs/sector/2.0.0");
                });
            }
            Err(e) => {
                ui.colored_label(colors::ERROR, format!("Sector stats error: {e}"));
            }
        }
    });
}

fn render_programs_section(
    ui: &mut egui::Ui,
    zode: &zfs_zode::Zode,
    status: &zfs_zode::ZodeStatus,
    known_programs: &HashMap<zfs_core::ProgramId, &str>,
) {
    section(ui, "Program Data", |ui| {
        ui.set_min_height(ui.available_height());

        if status.topics.is_empty() {
            muted_label(ui, "No subscribed programs.");
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("sector_storage_scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for topic in &status.topics {
                    render_program_entry(ui, zode, topic, known_programs);
                }
            });
    });
}

fn render_program_entry(
    ui: &mut egui::Ui,
    zode: &zfs_zode::Zode,
    topic: &str,
    known_programs: &HashMap<zfs_core::ProgramId, &str>,
) {
    let Some(hex) = topic.strip_prefix("prog/") else {
        return;
    };
    let Ok(pid) = zfs_core::ProgramId::from_hex(hex) else {
        return;
    };
    let label = match known_programs.get(&pid) {
        Some(name) => format!("Program: {} [{}]", &hex[..16.min(hex.len())], name),
        None => format!("Program: {}", &hex[..16.min(hex.len())]),
    };
    ui.collapsing(label, |ui| {
        let sectors = zode.storage().list_sectors(&pid).unwrap_or_default();
        if sectors.is_empty() {
            muted_label(ui, "No sectors stored.");
        } else {
            for sid in &sectors {
                render_sector_entry(ui, zode, &pid, sid);
            }
        }
    });
}

fn render_sector_entry(
    ui: &mut egui::Ui,
    zode: &zfs_zode::Zode,
    program_id: &zfs_core::ProgramId,
    sector_id: &zfs_core::SectorId,
) {
    let sid_hex = sector_id.to_hex();
    let header_id = format!("sector_{sid_hex}");
    let entry_count = zode
        .storage()
        .log_length(program_id, sector_id)
        .unwrap_or(0);
    let label = format!("  {sid_hex} ({entry_count} entries)");

    egui::CollapsingHeader::new(egui::RichText::new(label).monospace())
        .id_salt(&header_id)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                field_label(ui, "Sector ID");
                ui.monospace(&sid_hex);
                copy_button(ui, &sid_hex);
            });

            let entries = zode
                .storage()
                .read_log(program_id, sector_id, 0, 64)
                .unwrap_or_default();
            if entries.is_empty() {
                muted_label(ui, "Empty sector log.");
            } else {
                for (i, data) in entries.iter().enumerate() {
                    render_log_entry(ui, i, data, &sid_hex, zode, program_id, sector_id);
                }
            }
        });
}

fn render_log_entry(
    ui: &mut egui::Ui,
    index: usize,
    data: &[u8],
    sid_hex: &str,
    zode: &zfs_zode::Zode,
    program_id: &zfs_core::ProgramId,
    sector_id: &zfs_core::SectorId,
) {
    let short = &sid_hex[..8.min(sid_hex.len())];
    let entry_id = format!("entry_{short}_{index}");
    let label = format!("  [{index}] {} bytes", data.len());

    egui::CollapsingHeader::new(egui::RichText::new(label).monospace())
        .id_salt(&entry_id)
        .show(ui, |ui| {
            render_entry_content(ui, data, &format!("{short}_{index}"));
            ui.add_space(4.0);
            render_proof_section(ui, zode, program_id, sector_id, index, short);
        });
}

fn render_proof_section(
    ui: &mut egui::Ui,
    zode: &zfs_zode::Zode,
    program_id: &zfs_core::ProgramId,
    sector_id: &zfs_core::SectorId,
    index: usize,
    short_id: &str,
) {
    let proof_data = zode
        .storage()
        .get_proof(program_id, sector_id, index as u64)
        .ok()
        .flatten();

    match proof_data {
        Some(bytes) => match ciborium::from_reader::<ShapeProof, _>(&bytes[..]) {
            Ok(proof) => {
                let ct_hex = hex::encode(&proof.ciphertext_hash);
                let schema_hex = hex::encode(&proof.schema_hash);
                let system_label = match proof.proof_system {
                    zfs_core::ProofSystem::Groth16 => "Groth16",
                    zfs_core::ProofSystem::None => "None",
                };

                egui::CollapsingHeader::new(
                    egui::RichText::new("Shape Proof")
                        .color(egui::Color32::from_rgb(80, 200, 120)),
                )
                .id_salt(format!("proof_{short_id}_{index}"))
                .show(ui, |ui| {
                    info_grid(ui, &format!("proof_grid_{short_id}_{index}"), |ui| {
                        kv_row(ui, "Proof System", system_label);
                        kv_row(ui, "Size Bucket", &format!("{}", proof.size_bucket));
                        kv_row(
                            ui,
                            "CT Hash",
                            &ct_hex[..16.min(ct_hex.len())],
                        );
                        kv_row(
                            ui,
                            "Schema Hash",
                            &schema_hex[..16.min(schema_hex.len())],
                        );
                        kv_row(
                            ui,
                            "Proof Bytes",
                            &format!("{} bytes", proof.proof_bytes.len()),
                        );
                    });
                });
            }
            Err(_) => {
                muted_label(ui, "Proof data corrupted.");
            }
        },
        None => {
            muted_label(ui, "No proof attached.");
        }
    }
}

fn render_entry_content(ui: &mut egui::Ui, data: &[u8], label: &str) {
    ui.horizontal(|ui| {
        field_label(ui, "Size");
        ui.label(format!(
            "{} ({})",
            format_bytes(data.len() as u64),
            data.len()
        ));
    });

    render_text_preview(ui, data, label);
    ui.add_space(4.0);
    render_hex_preview(ui, data, label);
}

fn render_text_preview(ui: &mut egui::Ui, data: &[u8], short_sid: &str) {
    if let Ok(text) = std::str::from_utf8(data) {
        hint_label(ui, "Content appears to be valid UTF-8.");
        ui.add_space(4.0);
        let preview = if text.len() > 2048 {
            format!("{}...", &text[..2048])
        } else {
            text.to_string()
        };
        egui::CollapsingHeader::new("Text preview")
            .id_salt(format!("txt_{short_sid}"))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.monospace(&preview);
                    });
            });
    } else {
        hint_label(ui, "Content is binary / encrypted ciphertext.");
    }
}

const HEX_PREVIEW_BYTES: usize = 256;

fn render_hex_preview(ui: &mut egui::Ui, data: &[u8], short_sid: &str) {
    egui::CollapsingHeader::new("Hex dump")
        .id_salt(format!("hex_{short_sid}"))
        .show(ui, |ui| {
            let slice = &data[..data.len().min(HEX_PREVIEW_BYTES)];
            let hex_lines = format_hex_dump(slice);
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    ui.monospace(&hex_lines);
                });
            if data.len() > HEX_PREVIEW_BYTES {
                muted_label(
                    ui,
                    &format!(
                        "... showing first {} of {} bytes",
                        HEX_PREVIEW_BYTES,
                        data.len()
                    ),
                );
            }
        });
}

fn format_hex_dump(data: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        let offset = i * 16;
        out.push_str(&format!("{offset:08x}  "));

        for (j, byte) in chunk.iter().enumerate() {
            out.push_str(&format!("{byte:02x} "));
            if j == 7 {
                out.push(' ');
            }
        }
        let padding = 16 - chunk.len();
        for j in 0..padding {
            out.push_str("   ");
            if chunk.len() + j == 7 {
                out.push(' ');
            }
        }

        out.push_str(" |");
        for &b in chunk {
            let c = if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            };
            out.push(c);
        }
        out.push_str("|\n");
    }
    out
}

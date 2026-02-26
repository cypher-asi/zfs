use std::sync::Arc;

use eframe::egui;
use zero_neural::{
    derive_machine_keypair_from_shares, ed25519_to_did_key, generate_identity, verify_shares,
    MachineKeyCapabilities, ShamirShare,
};

use crate::app::ZodeApp;
use crate::components::{
    action_button, copy_button, editable_list, error_label, field_label, hint_label, info_grid,
    kv_row, kv_row_copyable, section, section_heading, std_button,
};
use crate::state::DerivedMachineKey;

pub(crate) fn render_identity(app: &mut ZodeApp, ui: &mut egui::Ui) {
    let has_identity = !app.identity_state.shares.is_empty();

    if app.identity_state.recovery_mode {
        render_recovery(app, ui);
    } else if has_identity {
        render_identity_info(app, ui);
        ui.add_space(4.0);
        render_machine_keys(app, ui);
    } else {
        render_no_identity(app, ui);
    }

    if let Some(ref err) = app.identity_state.error.clone() {
        ui.add_space(4.0);
        error_label(ui, err);
    }
}

fn render_no_identity(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "NEURAL KEY", |ui| {
        hint_label(
            ui,
            "No identity loaded. Generate a new Neural Key or recover from existing shares.",
        );
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if action_button(ui, "Generate Neural Key") {
                generate_new_identity(app);
            }
            ui.add_space(8.0);
            if action_button(ui, "Recover from Shares") {
                app.identity_state.recovery_mode = true;
                app.identity_state.error = None;
            }
        });
    });
}

fn generate_new_identity(app: &mut ZodeApp) {
    let mut rng = rand::thread_rng();
    let mut identity_id = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rng, &mut identity_id);

    // ML-DSA-65 key generation needs significant stack space; run on a
    // dedicated thread to avoid overflowing the main (egui) thread stack.
    let result = std::thread::Builder::new()
        .name("neural-keygen".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut rng = rand::thread_rng();
            generate_identity(3, 5, &identity_id, &mut rng)
        })
        .expect("failed to spawn keygen thread")
        .join()
        .expect("keygen thread panicked");

    match result {
        Ok(bundle) => {
            app.identity_state.shares = bundle.shares;
            app.identity_state.threshold = bundle.threshold;
            app.identity_state.identity_id = identity_id;
            app.identity_state.verifying_key = Some(bundle.verifying_key);
            app.identity_state.did = Some(bundle.did);
            app.identity_state.show_shares = true;
            app.identity_state.error = None;
        }
        Err(e) => {
            app.identity_state.error = Some(format!("Generation failed: {e}"));
        }
    }
}

fn render_identity_info(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "IDENTITY", |ui| {
        info_grid(ui, "identity_info_grid", |ui| {
            if let Some(ref did) = app.identity_state.did {
                field_label(ui, "DID");
                ui.horizontal(|ui| {
                    ui.monospace(truncate_did(did));
                    copy_button(ui, did);
                });
                ui.end_row();
            }

            kv_row(
                ui,
                "Threshold",
                &format!(
                    "{} of {}",
                    app.identity_state.threshold,
                    app.identity_state.shares.len()
                ),
            );
        });

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if std_button(
                ui,
                if app.identity_state.show_shares {
                    "Hide Shares"
                } else {
                    "Show Shares"
                },
            ) {
                app.identity_state.show_shares = !app.identity_state.show_shares;
            }
            ui.add_space(8.0);
            if std_button(ui, "Clear Identity") {
                app.identity_state = Default::default();
            }
        });

        if app.identity_state.show_shares {
            ui.add_space(8.0);
            ui.colored_label(
                crate::components::colors::WARN,
                "Store these shares in separate secure locations.",
            );
            ui.add_space(4.0);
            for share in &app.identity_state.shares {
                let hex = share.to_hex();
                ui.horizontal(|ui| {
                    ui.monospace(
                        egui::RichText::new(format!(
                            "Share {}: {}...",
                            share.index(),
                            &hex[..hex.len().min(24)]
                        ))
                        .weak(),
                    );
                    copy_button(ui, &hex);
                });
            }
        }
    });
}

fn render_recovery(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "RECOVER IDENTITY", |ui| {
        hint_label(
            ui,
            "Enter your Shamir shares (hex-encoded) to recover your identity.",
        );
        ui.add_space(8.0);

        editable_list(
            ui,
            &mut app.identity_state.recovery_inputs,
            &mut app.identity_state.recovery_input,
            0.0,
        );

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if action_button(ui, "Recover") {
                attempt_recovery(app);
            }
            ui.add_space(8.0);
            if std_button(ui, "Cancel") {
                app.identity_state.recovery_mode = false;
                app.identity_state.recovery_inputs.clear();
                app.identity_state.recovery_input.clear();
                app.identity_state.error = None;
            }
        });
    });
}

fn attempt_recovery(app: &mut ZodeApp) {
    let parsed: Result<Vec<ShamirShare>, _> = app
        .identity_state
        .recovery_inputs
        .iter()
        .map(|h| ShamirShare::from_hex(h.trim()))
        .collect();

    let shares = match parsed {
        Ok(s) => s,
        Err(e) => {
            app.identity_state.error = Some(format!("Invalid share: {e}"));
            return;
        }
    };

    if shares.is_empty() {
        app.identity_state.error = Some("Enter at least one share.".into());
        return;
    }

    let mut identity_id = app.identity_state.identity_id;
    if identity_id == [0u8; 16] {
        let mut rng = rand::thread_rng();
        rand::RngCore::fill_bytes(&mut rng, &mut identity_id);
    }

    let result = std::thread::Builder::new()
        .name("neural-recover".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || verify_shares(&shares, &identity_id))
        .expect("failed to spawn recovery thread")
        .join()
        .expect("recovery thread panicked");

    match result {
        Ok(info) => {
            let shares: Vec<ShamirShare> = app
                .identity_state
                .recovery_inputs
                .iter()
                .filter_map(|h| ShamirShare::from_hex(h.trim()).ok())
                .collect();
            app.identity_state.shares = shares;
            app.identity_state.identity_id = identity_id;
            app.identity_state.verifying_key = Some(info.verifying_key);
            app.identity_state.did = Some(info.did);
            app.identity_state.recovery_mode = false;
            app.identity_state.recovery_inputs.clear();
            app.identity_state.recovery_input.clear();
            app.identity_state.error = None;
        }
        Err(e) => {
            app.identity_state.error = Some(format!("Recovery failed: {e}"));
        }
    }
}

fn render_machine_keys(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "MACHINE KEYS", |ui| {
        section_heading(ui, "DERIVE NEW KEY");
        ui.add_space(6.0);

        egui::Grid::new("machine_key_form")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                field_label(ui, "Machine ID (hex)");
                ui.add(
                    egui::TextEdit::singleline(&mut app.identity_state.new_machine_id_hex)
                        .desired_width(200.0)
                        .hint_text("32 hex chars = 16 bytes"),
                );
                ui.end_row();

                field_label(ui, "Epoch");
                ui.add(egui::DragValue::new(&mut app.identity_state.new_epoch).range(0..=u64::MAX));
                ui.end_row();
            });

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.checkbox(&mut app.identity_state.cap_sign, "Sign");
            ui.checkbox(&mut app.identity_state.cap_encrypt, "Encrypt");
            ui.checkbox(&mut app.identity_state.cap_store, "Store");
            ui.checkbox(&mut app.identity_state.cap_fetch, "Fetch");
        });

        ui.add_space(8.0);

        if action_button(ui, "Derive Key") {
            derive_machine_key(app);
        }

        if !app.identity_state.machine_keys.is_empty() {
            ui.add_space(12.0);
            section_heading(ui, "DERIVED KEYS");
            ui.add_space(6.0);

            for mk in &app.identity_state.machine_keys {
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(20, 20, 22))
                    .rounding(0.0)
                    .inner_margin(8.0)
                    .stroke(egui::Stroke::new(1.0, crate::components::colors::BORDER))
                    .show(ui, |ui| {
                        info_grid(ui, &format!("mk_{}", mk.did), |ui| {
                            kv_row_copyable(ui, "DID", &mk.did);
                            kv_row(ui, "Epoch", &mk.epoch.to_string());
                            kv_row(ui, "Caps", &format!("{:?}", mk.capabilities));
                            kv_row(ui, "Machine ID", &hex::encode(mk.machine_id));
                        });
                    });
                ui.add_space(4.0);
            }
        }
    });
}

fn derive_machine_key(app: &mut ZodeApp) {
    let hex_str = app.identity_state.new_machine_id_hex.trim();
    let machine_id_bytes = match hex::decode(hex_str) {
        Ok(b) if b.len() == 16 => {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(&b);
            arr
        }
        Ok(b) => {
            app.identity_state.error =
                Some(format!("Machine ID must be 16 bytes, got {}", b.len()));
            return;
        }
        Err(e) => {
            app.identity_state.error = Some(format!("Invalid hex: {e}"));
            return;
        }
    };

    let mut caps = MachineKeyCapabilities::empty();
    if app.identity_state.cap_sign {
        caps |= MachineKeyCapabilities::SIGN;
    }
    if app.identity_state.cap_encrypt {
        caps |= MachineKeyCapabilities::ENCRYPT;
    }
    if app.identity_state.cap_store {
        caps |= MachineKeyCapabilities::STORE;
    }
    if app.identity_state.cap_fetch {
        caps |= MachineKeyCapabilities::FETCH;
    }

    let shares = app.identity_state.shares.clone();
    let identity_id = app.identity_state.identity_id;
    let epoch = app.identity_state.new_epoch;

    let result = std::thread::Builder::new()
        .name("neural-derive".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            derive_machine_keypair_from_shares(
                &shares,
                &identity_id,
                &machine_id_bytes,
                epoch,
                caps,
            )
        })
        .expect("failed to spawn derivation thread")
        .join()
        .expect("derivation thread panicked");

    match result {
        Ok(kp) => {
            let pk = kp.public_key();
            let did = ed25519_to_did_key(&pk.ed25519_bytes());
            let keypair = Arc::new(kp);
            app.identity_state.machine_keys.push(DerivedMachineKey {
                machine_id: machine_id_bytes,
                epoch: app.identity_state.new_epoch,
                capabilities: caps,
                did,
                public_key: pk,
                keypair,
            });
            app.identity_state.error = None;
        }
        Err(e) => {
            app.identity_state.error = Some(format!("Derivation failed: {e}"));
        }
    }
}

fn truncate_did(did: &str) -> String {
    if did.len() > 32 {
        format!("{}...{}", &did[..16], &did[did.len() - 8..])
    } else {
        did.to_string()
    }
}

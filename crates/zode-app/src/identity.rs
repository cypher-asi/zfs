use std::sync::Arc;

use eframe::egui;
use zid::{
    derive_machine_keypair_from_shares, ed25519_to_did_key, generate_identity, verify_shares,
    IdentityId, MachineId, MachineKeyCapabilities, ShamirShare,
};

use crate::app::ZodeApp;
use crate::components::{
    action_button, copy_button, editable_list, error_label, field_label, hint_label, info_grid,
    kv_row, kv_row_copyable, section, std_button,
};
use crate::profile;
use crate::state::DerivedMachineKey;
use crate::vault::VaultPlaintext;

pub(crate) fn render_identity(app: &mut ZodeApp, ui: &mut egui::Ui) {
    let has_identity = !app.identity_state.shares.is_empty();

    if app.identity_state.recovery_mode {
        render_recovery(app, ui);
    } else if has_identity {
        render_identity_info(app, ui);
        ui.add_space(4.0);
        render_machine_keys(app, ui);
        ui.add_space(4.0);
        if app.identity_state.pending_save {
            render_save_profile(app, ui);
        } else if app.active_profile_id.is_some() {
            render_profile_panel(app, ui);
        }
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

pub(crate) fn generate_new_identity(app: &mut ZodeApp) {
    let mut rng = rand::thread_rng();
    let mut identity_id = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rng, &mut identity_id);

    let result = std::thread::Builder::new()
        .name("neural-keygen".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut rng = rand::thread_rng();
            generate_identity(3, 5, IdentityId::new(identity_id), &mut rng)
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
            auto_derive_machine_key(app);
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

pub(crate) fn attempt_recovery(app: &mut ZodeApp) {
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
        .spawn(move || verify_shares(&shares, IdentityId::new(identity_id)))
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
            auto_derive_machine_key(app);
        }
        Err(e) => {
            app.identity_state.error = Some(format!("Recovery failed: {e}"));
        }
    }
}

fn render_machine_keys(app: &mut ZodeApp, ui: &mut egui::Ui) {
    section(ui, "MACHINE KEYS", |ui| {
        if app.identity_state.machine_keys.is_empty() {
            hint_label(ui, "No machine keys yet.");
            ui.add_space(8.0);
            if action_button(ui, "Derive Machine Key") {
                auto_derive_machine_key(app);
            }
        } else {
            for mk in &app.identity_state.machine_keys {
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(20, 20, 22))
                    .corner_radius(0.0)
                    .inner_margin(8.0)
                    .stroke(egui::Stroke::new(1.0, crate::components::colors::BORDER))
                    .show(ui, |ui| {
                        info_grid(ui, &format!("mk_{}", mk.did), |ui| {
                            kv_row_copyable(ui, "DID", &mk.did);
                            kv_row(ui, "Epoch", &mk.epoch.to_string());
                            kv_row(ui, "Caps", &format!("{:?}", mk.capabilities));
                        });
                    });
                ui.add_space(4.0);
            }

            if std_button(ui, "Derive Additional Key") {
                auto_derive_machine_key(app);
            }
        }
    });
}

fn auto_derive_machine_key(app: &mut ZodeApp) {
    let mut rng = rand::thread_rng();
    let mut machine_id_bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rng, &mut machine_id_bytes);

    let caps = MachineKeyCapabilities::SIGN | MachineKeyCapabilities::ENCRYPT;
    let epoch = (app.identity_state.machine_keys.len() as u64) + 1;

    let shares = app.identity_state.shares.clone();
    let identity_id = app.identity_state.identity_id;

    let result = std::thread::Builder::new()
        .name("neural-derive".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            derive_machine_keypair_from_shares(
                &shares,
                IdentityId::new(identity_id),
                MachineId::new(machine_id_bytes),
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
                epoch,
                capabilities: caps,
                did,
                public_key: pk,
                keypair,
            });
            app.identity_state.error = None;
            app.identity_state.pending_save = true;
        }
        Err(e) => {
            app.identity_state.error = Some(format!("Derivation failed: {e}"));
        }
    }
}

fn render_save_profile(app: &mut ZodeApp, ui: &mut egui::Ui) {
    let has_session_password = app.session_password.is_some();
    let mut do_save = false;

    section(ui, "SAVE PROFILE", |ui| {
        if has_session_password {
            hint_label(ui, "Vault will be updated using your session password.");
            ui.add_space(8.0);
            if action_button(ui, "Update Vault") {
                do_save = true;
            }
        } else {
            hint_label(
                ui,
                "Save your identity and machine keys to an encrypted vault on disk.",
            );
            ui.add_space(8.0);

            egui::Grid::new("save_profile_form")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    field_label(ui, "Profile Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.identity_state.save_profile_name)
                            .desired_width(200.0),
                    );
                    ui.end_row();

                    field_label(ui, "Password");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.identity_state.save_password)
                            .password(true)
                            .desired_width(200.0)
                            .hint_text("Vault encryption password"),
                    );
                    ui.end_row();
                });

            ui.add_space(8.0);
            if action_button(ui, "Save Profile") {
                do_save = true;
            }
        }

        if let Some(ref status) = app.identity_state.save_status {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(status).weak().italics());
        }
    });

    if do_save {
        save_profile_to_disk(app);
    }
}

fn save_profile_to_disk(app: &mut ZodeApp) {
    let mk = match app.identity_state.machine_keys.last() {
        Some(mk) => mk,
        None => {
            app.identity_state.save_status = Some("No machine key to save".into());
            return;
        }
    };

    let libp2p_bytes = app
        .zode
        .as_ref()
        .map(|z| z.keypair_protobuf().to_vec())
        .unwrap_or_default();

    let plaintext = VaultPlaintext {
        shares: app
            .identity_state
            .shares
            .iter()
            .map(|s| s.to_hex())
            .collect(),
        identity_id: app.identity_state.identity_id,
        machine_id: mk.machine_id,
        epoch: mk.epoch,
        capabilities: mk.capabilities.bits(),
        libp2p_keypair: libp2p_bytes,
    };

    let base = profile::base_dir();

    if let Some(ref profile_id) = app.active_profile_id.clone() {
        let password = app.session_password.clone().unwrap_or_default();
        match profile::update_vault(&base, profile_id, &plaintext, &password) {
            Ok(()) => {
                app.identity_state.pending_save = false;
                app.identity_state.save_status = Some("Vault updated.".into());
            }
            Err(e) => {
                app.identity_state.save_status = Some(format!("Save failed: {e}"));
            }
        }
    } else {
        let password = app.identity_state.save_password.clone();
        if password.is_empty() {
            app.identity_state.save_status = Some("Password is required.".into());
            return;
        }
        let name = app.identity_state.save_profile_name.clone();
        if name.is_empty() {
            app.identity_state.save_status = Some("Profile name is required.".into());
            return;
        }

        let peer_id = app
            .zode
            .as_ref()
            .map(|z| z.status().zode_id)
            .unwrap_or_default();
        let did = app.identity_state.did.clone().unwrap_or_default();

        match profile::create_profile(
            &base,
            profile::CreateProfileParams {
                name,
                peer_id,
                did,
                plaintext,
                password: password.clone(),
            },
        ) {
            Ok(meta) => {
                app.active_profile_id = Some(meta.id.clone());
                app.session_password = Some(password);
                app.identity_state.save_password.clear();
                app.identity_state.pending_save = false;
                app.identity_state.save_status = Some("Profile saved.".into());
                app.profiles.push(meta);
            }
            Err(e) => {
                app.identity_state.save_status = Some(format!("Save failed: {e}"));
            }
        }
    }
}

fn render_profile_panel(app: &mut ZodeApp, ui: &mut egui::Ui) {
    let profile_id = match app.active_profile_id.as_ref() {
        Some(id) => id.clone(),
        None => return,
    };
    let meta = app.profiles.iter().find(|p| p.id == profile_id).cloned();
    let mut do_delete = false;

    section(ui, "PROFILE", |ui| {
        if let Some(ref meta) = meta {
            info_grid(ui, "profile_panel_grid", |ui| {
                kv_row(ui, "Name", &meta.name);
                if !meta.did.is_empty() {
                    field_label(ui, "DID");
                    ui.horizontal(|ui| {
                        ui.monospace(truncate_did(&meta.did));
                        copy_button(ui, &meta.did);
                    });
                    ui.end_row();
                }
                if !meta.peer_id.is_empty() {
                    field_label(ui, "Peer");
                    ui.horizontal(|ui| {
                        ui.monospace(truncate_did(&meta.peer_id));
                        copy_button(ui, &meta.peer_id);
                    });
                    ui.end_row();
                }
            });
        }

        if let Some(ref status) = app.identity_state.save_status {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(status).weak().italics());
        }

        ui.add_space(12.0);

        if app.confirm_delete_profile.as_deref() == Some(&*profile_id) {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Delete this profile?")
                        .size(11.0)
                        .color(crate::components::colors::ERROR),
                );
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Yes, delete")
                                .size(11.0)
                                .color(crate::components::colors::ERROR),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    do_delete = true;
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Cancel")
                                .size(11.0)
                                .color(egui::Color32::from_rgb(160, 160, 165)),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    app.confirm_delete_profile = None;
                }
            });
        } else if ui
            .add(
                egui::Button::new(
                    egui::RichText::new("Delete profile")
                        .size(11.0)
                        .color(egui::Color32::from_rgb(100, 100, 108)),
                )
                .frame(false),
            )
            .clicked()
        {
            app.confirm_delete_profile = Some(profile_id.clone());
        }
    });

    if do_delete {
        app.do_delete_profile(&profile_id);
    }
}

fn truncate_did(did: &str) -> String {
    if did.len() > 32 {
        format!("{}...{}", &did[..16], &did[did.len() - 8..])
    } else {
        did.to_string()
    }
}

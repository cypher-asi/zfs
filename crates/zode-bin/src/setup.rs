use eframe::egui;

use crate::app::ZodeApp;
use crate::components::tokens::spacing;
use crate::components::{
    action_button, auth_panel_frame, auth_screen_panel, centered_row, editable_list, error_label,
    field_label, form_grid, ghost_button, hint_label, link_button, status_label,
    text_input_password, warn_label,
};
use crate::helpers::shorten_id;
use crate::identity;
use crate::profile;
use crate::state::AppPhase;
use crate::vault::VaultPlaintext;

impl ZodeApp {
    pub(crate) fn render_setup_screen(&mut self, ctx: &egui::Context) {
        match self.identity_state.setup_step {
            0 => self.render_setup_step_generate(ctx),
            1 => self.render_setup_step_profile(ctx),
            _ => {}
        }
    }

    fn render_setup_step_generate(&mut self, ctx: &egui::Context) {
        let tex = self.icon_texture(ctx);

        egui::CentralPanel::default()
            .frame(auth_panel_frame())
            .show(ctx, |ui| {
                auth_screen_panel(ui, &tex, "SETUP YOUR ZODE", 220.0, |ui| {
                    if self.identity_state.recovery_mode {
                        self.render_setup_recovery(ui);
                    } else {
                        hint_label(
                            ui,
                            "Generate a new Neural Key or recover from existing shards.",
                        );
                        ui.add_space(spacing::XL);

                        centered_row(ui, "setup_btns", |ui| {
                            if action_button(ui, "Generate Neural Key") {
                                identity::generate_new_identity(self);
                                if self.identity_state.error.is_none() {
                                    self.identity_state.setup_step = 1;
                                }
                            }
                            ui.add_space(spacing::MD);
                            if action_button(ui, "Recover from Shards") {
                                self.identity_state.recovery_mode = true;
                                self.identity_state.error = None;
                            }
                        });
                    }

                    if let Some(ref err) = self.identity_state.error.clone() {
                        ui.add_space(spacing::MD);
                        error_label(ui, err);
                    }
                });
            });
    }

    fn render_setup_recovery(&mut self, ui: &mut egui::Ui) {
        hint_label(
            ui,
            "Enter your Shamir shares (hex-encoded) to recover your identity.",
        );
        ui.add_space(spacing::MD);

        editable_list(
            ui,
            &mut self.identity_state.recovery_inputs,
            &mut self.identity_state.recovery_input,
            0.0,
        );

        ui.add_space(spacing::MD);

        centered_row(ui, "recovery_btns", |ui| {
            if action_button(ui, "Recover") {
                identity::attempt_recovery(self);
                if self.identity_state.error.is_none() {
                    self.identity_state.setup_step = 1;
                }
            }
            ui.add_space(spacing::MD);
            if action_button(ui, "Cancel") {
                self.identity_state.recovery_mode = false;
                self.identity_state.recovery_inputs.clear();
                self.identity_state.recovery_input.clear();
                self.identity_state.error = None;
            }
        });
    }

    fn render_setup_step_profile(&mut self, ctx: &egui::Context) {
        let tex = self.icon_texture(ctx);

        let mut do_create = false;

        egui::CentralPanel::default()
            .frame(auth_panel_frame())
            .show(ctx, |ui| {
                let panel = ui.max_rect();

                let back_rect = egui::Rect::from_min_size(panel.min, egui::vec2(80.0, 28.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(back_rect), |ui| {
                    if ghost_button(ui, egui_phosphor::regular::ARROW_LEFT, "Back") {
                        self.identity_state.setup_step = 0;
                        self.identity_state.error = None;
                        self.identity_state.show_shares = false;
                    }
                });

                let warn_h = 20.0;
                let warn_rect = egui::Rect::from_min_max(
                    egui::pos2(panel.min.x, panel.max.y - warn_h),
                    panel.max,
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(warn_rect), |ui| {
                    ui.vertical_centered(|ui| {
                        warn_label(ui, "Back up your Neural Key shards before continuing.");
                    });
                });

                auth_screen_panel(ui, &tex, "SAVE YOUR PROFILE", 520.0, |ui| {
                    if let Some(ref did) = self.identity_state.did.clone() {
                        centered_row(ui, "did_row", |ui| {
                            field_label(ui, "DID");
                            ui.monospace(shorten_id(did, 16, 8));
                        });
                        ui.add_space(spacing::SM);
                    }

                    ui.add_space(spacing::SM);
                    for share in &self.identity_state.shares {
                        let hex = share.to_hex();
                        let visible_len = hex.len().min(24);
                        let truncated = format!("{}...", &hex[..visible_len]);
                        let masked = "*".repeat(visible_len) + "...";
                        let display = format!(
                            "Shard {}: {}",
                            share.index(),
                            if self.identity_state.show_shares {
                                &truncated
                            } else {
                                &masked
                            },
                        );
                        centered_row(ui, &format!("share_{}", share.index()), |ui| {
                            ui.monospace(egui::RichText::new(display).weak());
                            crate::components::copy_button(ui, &hex);
                        });
                    }
                    ui.add_space(spacing::SM);
                    if link_button(
                        ui,
                        if self.identity_state.show_shares {
                            "Hide Shards"
                        } else {
                            "Show Shards"
                        },
                    ) {
                        self.identity_state.show_shares = !self.identity_state.show_shares;
                    }

                    ui.add_space(spacing::XL);

                    form_grid(ui, "setup_profile_form", |ui| {
                        field_label(ui, "Profile Name");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.identity_state.save_profile_name)
                                .desired_width(200.0),
                        );
                        ui.end_row();

                        field_label(ui, "Password");
                        ui.add(
                            text_input_password(&mut self.identity_state.save_password, 200.0)
                                .hint_text("Vault encryption password"),
                        );
                        ui.end_row();

                        field_label(ui, "Confirm Password");
                        let resp = ui.add(
                            text_input_password(
                                &mut self.identity_state.setup_password_confirm,
                                200.0,
                            )
                            .hint_text("Confirm password"),
                        );
                        ui.end_row();

                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            do_create = true;
                        }
                    });

                    ui.add_space(spacing::XL);
                    if action_button(ui, "Create Profile") {
                        do_create = true;
                    }

                    if let Some(ref err) = self.identity_state.error.clone() {
                        ui.add_space(spacing::MD);
                        error_label(ui, err);
                    }
                    if let Some(ref status) = self.identity_state.save_status.clone() {
                        ui.add_space(spacing::SM);
                        status_label(ui, status);
                    }
                });
            });

        if do_create {
            self.do_setup_create_profile();
        }
    }

    fn do_setup_create_profile(&mut self) {
        let password = self.identity_state.save_password.clone();
        if password.is_empty() {
            self.identity_state.error = Some("Password is required.".into());
            return;
        }
        if password != self.identity_state.setup_password_confirm {
            self.identity_state.error = Some("Passwords do not match.".into());
            return;
        }
        let name = self.identity_state.save_profile_name.clone();
        if name.is_empty() {
            self.identity_state.error = Some("Profile name is required.".into());
            return;
        }

        let mk = match self.identity_state.machine_keys.last() {
            Some(mk) => mk,
            None => {
                self.identity_state.error = Some("No machine key derived.".into());
                return;
            }
        };

        let plaintext = VaultPlaintext {
            shares: self
                .identity_state
                .shares
                .iter()
                .map(|s| s.to_hex())
                .collect(),
            identity_id: self.identity_state.identity_id,
            machine_id: mk.machine_id,
            epoch: mk.epoch,
            capabilities: mk.capabilities.bits(),
            libp2p_keypair: Vec::new(),
        };

        let did = self.identity_state.did.clone().unwrap_or_default();
        let base = profile::base_dir();

        match profile::create_profile(
            &base,
            profile::CreateProfileParams {
                name,
                peer_id: String::new(),
                did,
                plaintext,
                password: password.clone(),
            },
        ) {
            Ok(meta) => {
                let profile_id = meta.id.clone();
                self.active_profile_id = Some(profile_id.clone());
                self.session_password = Some(password);
                self.profiles.push(meta);

                if let Ok(data_dir) = profile::data_dir_for_profile(&base, &profile_id) {
                    self.settings.data_dir = data_dir.to_string_lossy().to_string();
                }

                self.identity_state.save_password.clear();
                self.identity_state.setup_password_confirm.clear();
                self.identity_state.pending_save = false;
                self.identity_state.error = None;
                self.identity_state.save_status = None;

                self.boot_zode_with_keypair(None);
                self.phase = AppPhase::Revealing;
                self.reveal_start = None;
            }
            Err(e) => {
                self.identity_state.error = Some(format!("Profile creation failed: {e}"));
            }
        }
    }
}

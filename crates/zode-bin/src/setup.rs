use eframe::egui;

use crate::app::ZodeApp;
use crate::components::{action_button, centered_row, editable_list, error_label, field_label, hint_label};
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
        let frame = egui::Frame::default()
            .fill(egui::Color32::BLACK)
            .inner_margin(32.0);

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let panel = ui.max_rect();
            let col_w = 380.0_f32.min(panel.width());
            let col = egui::Rect::from_center_size(
                panel.center(),
                egui::vec2(col_w, panel.height()),
            );

            ui.scope_builder(egui::UiBuilder::new().max_rect(col), |ui| {
                ui.vertical_centered(|ui| {
                    let content_height = 220.0;
                    ui.add_space(((panel.height() - content_height) / 2.0).max(20.0));

                    ui.add(
                        egui::Image::new(&tex)
                            .fit_to_exact_size(egui::vec2(56.0, 56.0))
                            .corner_radius(8.0),
                    );
                    ui.add_space(16.0);

                    ui.label(
                        egui::RichText::new("SETUP YOUR ZODE")
                            .strong()
                            .size(12.0)
                            .color(egui::Color32::from_rgb(140, 140, 145)),
                    );
                    ui.add_space(8.0);

                    if self.identity_state.recovery_mode {
                        self.render_setup_recovery(ui);
                    } else {
                        hint_label(ui, "Generate a new Neural Key or recover from existing shards.");
                        ui.add_space(16.0);

                        centered_row(ui, "setup_btns", |ui| {
                            if action_button(ui, "Generate Neural Key") {
                                identity::generate_new_identity(self);
                                if self.identity_state.error.is_none() {
                                    self.identity_state.setup_step = 1;
                                }
                            }
                            ui.add_space(8.0);
                            if action_button(ui, "Recover from Shards") {
                                self.identity_state.recovery_mode = true;
                                self.identity_state.error = None;
                            }
                        });
                    }

                    if let Some(ref err) = self.identity_state.error.clone() {
                        ui.add_space(8.0);
                        error_label(ui, err);
                    }
                });
            });
        });
    }

    fn render_setup_recovery(&mut self, ui: &mut egui::Ui) {
        hint_label(
            ui,
            "Enter your Shamir shares (hex-encoded) to recover your identity.",
        );
        ui.add_space(8.0);

        editable_list(
            ui,
            &mut self.identity_state.recovery_inputs,
            &mut self.identity_state.recovery_input,
            0.0,
        );

        ui.add_space(8.0);

        centered_row(ui, "recovery_btns", |ui| {
            if action_button(ui, "Recover") {
                identity::attempt_recovery(self);
                if self.identity_state.error.is_none() {
                    self.identity_state.setup_step = 1;
                }
            }
            ui.add_space(8.0);
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
        let frame = egui::Frame::default()
            .fill(egui::Color32::BLACK)
            .inner_margin(32.0);

        let mut do_create = false;

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let panel = ui.max_rect();
            let col_w = 380.0_f32.min(panel.width());
            let col = egui::Rect::from_center_size(
                panel.center(),
                egui::vec2(col_w, panel.height()),
            );

            ui.scope_builder(egui::UiBuilder::new().max_rect(col), |ui| {
                ui.vertical_centered(|ui| {
                    let content_h = if self.identity_state.show_shares {
                        520.0
                    } else {
                        380.0
                    };
                    ui.add_space(((panel.height() - content_h) / 2.0).max(20.0));

                    ui.add(
                        egui::Image::new(&tex)
                            .fit_to_exact_size(egui::vec2(56.0, 56.0))
                            .corner_radius(8.0),
                    );
                    ui.add_space(16.0);

                    ui.label(
                        egui::RichText::new("CREATE PROFILE")
                            .strong()
                            .size(12.0)
                            .color(egui::Color32::from_rgb(140, 140, 145)),
                    );
                    ui.add_space(8.0);

                    if let Some(ref did) = self.identity_state.did.clone() {
                        centered_row(ui, "did_row", |ui| {
                            field_label(ui, "DID");
                            ui.monospace(truncate_did(did));
                        });
                        ui.add_space(4.0);
                    }

                    ui.colored_label(
                        crate::components::colors::WARN,
                        "Back up your Neural Key shares before continuing.",
                    );

                    if self.identity_state.show_shares {
                        ui.add_space(4.0);
                        for share in &self.identity_state.shares {
                            let hex = share.to_hex();
                            centered_row(
                                ui,
                                &format!("share_{}", share.index()),
                                |ui| {
                                    ui.monospace(
                                        egui::RichText::new(format!(
                                            "Share {}: {}...",
                                            share.index(),
                                            &hex[..hex.len().min(24)]
                                        ))
                                        .weak(),
                                    );
                                    crate::components::copy_button(ui, &hex);
                                },
                            );
                        }
                    }
                    ui.add_space(4.0);
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(if self.identity_state.show_shares {
                                    "Hide Shares"
                                } else {
                                    "Show Shares"
                                })
                                .size(11.0)
                                .color(egui::Color32::from_rgb(100, 100, 108)),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.identity_state.show_shares = !self.identity_state.show_shares;
                    }

                    ui.add_space(16.0);

                    egui::Grid::new("setup_profile_form")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            field_label(ui, "Profile Name");
                            ui.add(
                                egui::TextEdit::singleline(
                                    &mut self.identity_state.save_profile_name,
                                )
                                .desired_width(200.0),
                            );
                            ui.end_row();

                            field_label(ui, "Password");
                            ui.add(
                                egui::TextEdit::singleline(
                                    &mut self.identity_state.save_password,
                                )
                                .password(true)
                                .desired_width(200.0)
                                .hint_text("Vault encryption password"),
                            );
                            ui.end_row();

                            field_label(ui, "Confirm Password");
                            let resp = ui.add(
                                egui::TextEdit::singleline(
                                    &mut self.identity_state.setup_password_confirm,
                                )
                                .password(true)
                                .desired_width(200.0)
                                .hint_text("Confirm password"),
                            );
                            ui.end_row();

                            if resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                do_create = true;
                            }
                        });

                    ui.add_space(16.0);
                    if action_button(ui, "Create Profile") {
                        do_create = true;
                    }

                    if let Some(ref err) = self.identity_state.error.clone() {
                        ui.add_space(8.0);
                        error_label(ui, err);
                    }
                    if let Some(ref status) = self.identity_state.save_status.clone() {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(status).weak().italics());
                    }
                });
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

                let data_dir = profile::data_dir_for_profile(&base, &profile_id);
                self.settings.data_dir = data_dir.to_string_lossy().to_string();

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

fn truncate_did(did: &str) -> String {
    if did.len() > 32 {
        format!("{}...{}", &did[..16], &did[did.len() - 8..])
    } else {
        did.to_string()
    }
}

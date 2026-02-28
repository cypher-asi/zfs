use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use grid_core::{GossipSectorAppend, ProgramId, SectorId, ShapeProof};
use grid_crypto::SectorKey;
use grid_programs_interlink::interlink::{
    ChannelId, InterlinkDescriptor, ZMessage, TEST_CHANNEL_ID,
};
use grid_proof_groth16::Groth16ShapeProver;
use grid_storage::SectorStore;
use hkdf::Hkdf;
use sha2::Sha256;
use tracing::info;
use zid::MachineKeyCapabilities;

use crate::app::ZodeApp;
use crate::components::{
    error_label, field_label, info_grid, kv_row, section, std_button, text_input,
};
use crate::helpers::format_timestamp_ms;
use crate::state::{DisplayMessage, InterlinkState, InterlinkUpdate, SignatureStatus};

fn derive_test_sector_key() -> SectorKey {
    let hk = Hkdf::<Sha256>::new(None, b"interlink-main-channel-v1");
    let mut key_bytes = [0u8; 32];
    hk.expand(b"grid:test-channel-key:v1", &mut key_bytes)
        .expect("32-byte expand cannot fail");
    SectorKey::from_bytes(key_bytes)
}

// ---------------------------------------------------------------------------
// ZodeApp interlink lifecycle
// ---------------------------------------------------------------------------

impl ZodeApp {
    pub(crate) fn init_interlink(&mut self) {
        let sector_key = derive_test_sector_key();

        let real_key = self
            .identity_state
            .machine_keys
            .iter()
            .find(|mk| mk.capabilities.contains(MachineKeyCapabilities::SIGN));

        let (signing_keypair, machine_did) = match real_key {
            Some(mk) => (Arc::clone(&mk.keypair), mk.did.clone()),
            None => {
                self.interlink_state = Some(InterlinkState::error_only(
                    "No machine key with SIGN capability. Derive one on the Identity tab first.",
                ));
                return;
            }
        };

        let data_dir = self.settings.data_dir.clone();
        let channel_id = ChannelId::from_str_id(TEST_CHANNEL_ID);
        let program_id = InterlinkDescriptor::v2()
            .program_id()
            .expect("Interlink descriptor is valid");
        let sector_id = channel_id.sector_id();

        let prover = load_or_generate_prover(&data_dir);

        let (update_tx, update_rx) = tokio::sync::mpsc::channel::<InterlinkUpdate>(4);
        let (refresh_tx, refresh_rx) = tokio::sync::mpsc::channel::<()>(4);

        if let Some(ref zode) = self.zode {
            Self::spawn_interlink_updater(
                &self.rt,
                zode,
                &sector_key,
                program_id,
                sector_id.clone(),
                update_tx,
                refresh_rx,
            );
        }

        self.interlink_state = Some(InterlinkState {
            messages: Vec::new(),
            compose: String::new(),
            sector_key: Some(sector_key),
            machine_did,
            signing_keypair: Some(signing_keypair),
            channel_id: Some(channel_id),
            program_id: Some(program_id),
            sector_id: Some(sector_id),
            prover: Some(prover),
            error: None,
            initialized: true,
            scroll_to_bottom: true,
            focus_compose: true,
            update_rx: Some(update_rx),
            refresh_tx: Some(refresh_tx),
        });
    }

    fn spawn_interlink_updater(
        rt: &tokio::runtime::Runtime,
        zode: &Arc<zode::Zode>,
        sector_key: &SectorKey,
        program_id: ProgramId,
        sector_id: SectorId,
        update_tx: tokio::sync::mpsc::Sender<InterlinkUpdate>,
        mut refresh_rx: tokio::sync::mpsc::Receiver<()>,
    ) {
        let bg_storage = Arc::clone(zode.storage());
        let bg_key = sector_key.clone();
        rt.spawn(async move {
            let mut known_len: u64 = 0;
            loop {
                known_len = poll_new_entries(
                    &bg_storage,
                    &bg_key,
                    &program_id,
                    &sector_id,
                    known_len,
                    &update_tx,
                )
                .await;
                tokio::select! {
                    _ = refresh_rx.recv() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                }
            }
        });
    }

    pub(crate) fn send_message(&mut self) {
        let Some(ref zode) = self.zode else {
            if let Some(ref mut il) = self.interlink_state {
                il.error = Some("ZODE is not running".into());
            }
            return;
        };
        let storage = Arc::clone(zode.storage());
        let il = self.interlink_state.as_mut().unwrap();
        let text = il.compose.trim().to_string();
        if text.is_empty() {
            return;
        }

        let (Some(ref sector_key), Some(ref program_id), Some(ref sector_id), Some(ref prover)) =
            (&il.sector_key, &il.program_id, &il.sector_id, &il.prover)
        else {
            il.error = Some("Interlink not fully initialized".into());
            return;
        };

        il.compose.clear();

        let msg = match build_interlink_message(il, text) {
            Ok(m) => m,
            Err(e) => {
                il.error = Some(e);
                return;
            }
        };

        match encrypt_message(&msg, sector_key, program_id, sector_id, prover) {
            Ok((ciphertext, proof)) => {
                do_append(il, &storage, zode, ciphertext, Some(proof));
            }
            Err(e) => {
                il.error = Some(e);
            }
        }
    }
}

fn do_append(
    il: &mut InterlinkState,
    storage: &Arc<grid_storage::RocksStorage>,
    zode: &Arc<zode::Zode>,
    ciphertext: Vec<u8>,
    shape_proof: Option<ShapeProof>,
) {
    let (Some(ref program_id), Some(ref sector_id)) = (&il.program_id, &il.sector_id) else {
        il.error = Some("Interlink not fully initialized".into());
        return;
    };
    match storage.append(program_id, sector_id, &ciphertext) {
        Ok(index) => {
            if let Some(ref proof) = shape_proof {
                if let Ok(proof_bytes) = encode_proof_cbor(proof) {
                    let _ = storage.store_proof(program_id, sector_id, index, &proof_bytes);
                }
            }
            il.error = None;
            broadcast_gossip(
                zode,
                *program_id,
                sector_id.clone(),
                index,
                ciphertext,
                shape_proof,
            );
            if let Some(ref tx) = il.refresh_tx {
                let _ = tx.try_send(());
            }
        }
        Err(e) => {
            il.error = Some(format!("Sector append failed: {e}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Message construction and encryption
// ---------------------------------------------------------------------------

fn build_interlink_message(il: &InterlinkState, text: String) -> Result<ZMessage, String> {
    let channel_id = il
        .channel_id
        .as_ref()
        .ok_or_else(|| "No channel ID".to_string())?;
    let signing_keypair = il
        .signing_keypair
        .as_ref()
        .ok_or_else(|| "No signing keypair".to_string())?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    ZMessage::new_signed(
        il.machine_did.clone(),
        channel_id.clone(),
        text,
        now_ms,
        |signable| signing_keypair.sign(signable).to_bytes(),
    )
    .map_err(|e| format!("Sign failed: {e}"))
}

fn encrypt_message(
    msg: &ZMessage,
    key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
    prover: &Groth16ShapeProver,
) -> Result<(Vec<u8>, ShapeProof), String> {
    let plaintext = msg
        .encode_canonical()
        .map_err(|e| format!("Encode failed: {e}"))?;
    let schema = InterlinkDescriptor::field_schema();
    grid_sdk::sector_encrypt_and_prove(&plaintext, key, program_id, sector_id, prover, &schema)
        .map_err(|e| format!("Encrypt+prove failed: {e}"))
}

// ---------------------------------------------------------------------------
// Background polling
// ---------------------------------------------------------------------------

async fn poll_new_entries(
    storage: &Arc<grid_storage::RocksStorage>,
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
    known_len: u64,
    update_tx: &tokio::sync::mpsc::Sender<InterlinkUpdate>,
) -> u64 {
    let current_len = storage
        .log_length(program_id, sector_id)
        .unwrap_or(known_len);
    if current_len <= known_len {
        return known_len;
    }
    let entries = storage
        .read_log(program_id, sector_id, known_len, 64)
        .unwrap_or_default();
    if entries.is_empty() {
        return known_len;
    }
    let new_len = known_len + entries.len() as u64;
    let upd = decrypt_entries(entries, sector_key, program_id, sector_id);
    if update_tx.send(upd).await.is_err() {
        return new_len;
    }
    new_len
}

fn decrypt_entries(
    entries: Vec<Vec<u8>>,
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> InterlinkUpdate {
    let mut display: Vec<DisplayMessage> = Vec::new();
    let mut first_error: Option<String> = None;
    for ct in &entries {
        match decrypt_one(ct, sector_key, program_id, sector_id) {
            Ok(msg) => display.push(msg),
            Err(e) if first_error.is_none() => first_error = Some(e),
            Err(_) => {}
        }
    }
    InterlinkUpdate {
        new_messages: display,
        error: first_error,
    }
}

fn decrypt_one(
    ciphertext: &[u8],
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> Result<DisplayMessage, String> {
    let plaintext =
        grid_sdk::sector_decrypt_poseidon(ciphertext, sector_key, program_id, sector_id)
            .map_err(|e| format!("Decrypt: {e}"))?;
    let msg = ZMessage::decode_canonical(&plaintext).map_err(|e| format!("Decode: {e}"))?;
    let sig_status = match msg.verify_signature(|signable, sig_bytes| {
        zid::verify_did_ed25519(&msg.sender_did, signable, sig_bytes).is_ok()
    }) {
        Ok(true) => SignatureStatus::Verified,
        Ok(false) => SignatureStatus::None,
        Err(_) => SignatureStatus::Failed,
    };
    Ok(DisplayMessage {
        sender: msg.sender_did,
        content: msg.content,
        timestamp_ms: msg.timestamp_ms,
        signature_status: sig_status,
    })
}

// ---------------------------------------------------------------------------
// Gossip broadcast
// ---------------------------------------------------------------------------

fn broadcast_gossip(
    zode: &Arc<zode::Zode>,
    program_id: ProgramId,
    sector_id: SectorId,
    index: u64,
    ciphertext: Vec<u8>,
    shape_proof: Option<ShapeProof>,
) {
    let gossip = GossipSectorAppend {
        program_id,
        sector_id,
        index,
        payload: ciphertext,
        shape_proof,
    };
    let topic = grid_core::program_topic(&gossip.program_id);
    if let Ok(data) = grid_core::encode_canonical(&gossip) {
        zode.publish(topic, data);
    }
}

// ---------------------------------------------------------------------------
// Proof key loading / generation
// ---------------------------------------------------------------------------

/// Ensure the Groth16 proving and verifying key files exist on disk.
fn ensure_proof_keys(data_dir: &str) {
    let key_dir = PathBuf::from(data_dir).join("proof_keys");
    grid_proof_groth16::ensure_keys(&key_dir);
}

fn load_or_generate_prover(data_dir: &str) -> Box<Groth16ShapeProver> {
    ensure_proof_keys(data_dir);
    let key_dir = PathBuf::from(data_dir).join("proof_keys");
    let prover = Groth16ShapeProver::load(&key_dir).expect("failed to load proving keys");
    info!("loaded Groth16 proving keys");
    Box::new(prover)
}

fn encode_proof_cbor(proof: &ShapeProof) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    ciborium::into_writer(proof, &mut buf).map_err(|e| format!("CBOR encode proof: {e}"))?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// UI rendering
// ---------------------------------------------------------------------------

pub(crate) fn render_interlink(app: &mut ZodeApp, ui: &mut egui::Ui) {
    if app.interlink_state.is_none() || !app.interlink_state.as_ref().unwrap().initialized {
        app.init_interlink();
    }
    render_interlink_header(app, ui);
    drain_interlink_updates(app);
    render_interlink_messages(app, ui);
    render_interlink_compose(app, ui);
}

fn render_interlink_header(app: &ZodeApp, ui: &mut egui::Ui) {
    let il = app.interlink_state.as_ref().unwrap();

    section(ui, "INTERLINK", |ui| {
        if let (Some(ref sector_key), Some(ref channel_id)) = (&il.sector_key, &il.channel_id) {
            let key_preview: String = sector_key.as_bytes()[..8]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let ch_display = String::from_utf8_lossy(channel_id.as_bytes()).to_string();

            info_grid(ui, "interlink_info_grid", |ui| {
                kv_row(ui, "Channel", &ch_display);

                field_label(ui, "Sector Key");
                ui.label(
                    egui::RichText::new(format!("{key_preview}..."))
                        .monospace()
                        .weak(),
                );
                ui.end_row();

                kv_row(ui, "Messages", &format!("{}", il.messages.len()));
                kv_row(ui, "Protocol", "/grid/sector/2.0.0");
            });
        }
    });

    if let Some(ref err) = il.error {
        error_label(ui, err);
    }
}

fn drain_interlink_updates(app: &mut ZodeApp) {
    let il = app.interlink_state.as_mut().unwrap();
    if let Some(ref mut rx) = il.update_rx {
        while let Ok(upd) = rx.try_recv() {
            if upd.error.is_some() {
                il.error = upd.error;
            }
            if !upd.new_messages.is_empty() {
                il.messages.extend(upd.new_messages);
                il.scroll_to_bottom = true;
            }
        }
    }
}

fn short_sender(did: &str) -> String {
    const COMMON_PREFIX: &str = "Zx12D3KooW";
    if did.starts_with(COMMON_PREFIX) {
        let unique = &did[COMMON_PREFIX.len()..];
        let n = 6.min(unique.len());
        format!("Zx..{}", &unique[unique.len() - n..])
    } else if did.len() > 10 {
        format!("{}..{}", &did[..4], &did[did.len() - 6..])
    } else {
        did.to_string()
    }
}

fn render_interlink_messages(app: &mut ZodeApp, ui: &mut egui::Ui) {
    let il = app.interlink_state.as_mut().unwrap();
    let should_scroll = il.scroll_to_bottom;
    il.scroll_to_bottom = false;

    let available = ui.available_height() - 40.0;
    egui::ScrollArea::vertical()
        .max_height(available.max(100.0))
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            let il = app.interlink_state.as_ref().unwrap();
            if il.messages.is_empty() {
                ui.label(
                    egui::RichText::new("No messages yet. Type something below!")
                        .weak()
                        .italics(),
                );
            } else {
                for msg in &il.messages {
                    render_single_message(ui, msg);
                }
            }
            if should_scroll {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
        });
    ui.separator();
}

fn render_single_message(ui: &mut egui::Ui, msg: &DisplayMessage) {
    let time = format_timestamp_ms(msg.timestamp_ms);
    let name = short_sender(&msg.sender);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("[{time}]")).monospace().weak());
        ui.label(egui::RichText::new(format!("{name}:")).monospace().strong());
        ui.label(&msg.content);

        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| match msg.signature_status {
                SignatureStatus::Verified => {
                    let size = 14.0;
                    let (resp, painter) =
                        ui.allocate_painter(egui::Vec2::splat(size), egui::Sense::hover());
                    let c = resp.rect.center();
                    painter.add(egui::Shape::line(
                        vec![
                            c + egui::vec2(-3.5, 0.5),
                            c + egui::vec2(-1.0, 3.0),
                            c + egui::vec2(4.5, -3.5),
                        ],
                        egui::Stroke::new(2.0, crate::components::colors::CONNECTED),
                    ));
                }
                SignatureStatus::Failed => {
                    let size = 14.0;
                    let (resp, painter) =
                        ui.allocate_painter(egui::Vec2::splat(size), egui::Sense::hover());
                    let c = resp.rect.center();
                    let stroke = egui::Stroke::new(2.0, crate::components::colors::ERROR);
                    painter.line_segment(
                        [c + egui::vec2(-3.0, -3.0), c + egui::vec2(3.0, 3.0)],
                        stroke,
                    );
                    painter.line_segment(
                        [c + egui::vec2(3.0, -3.0), c + egui::vec2(-3.0, 3.0)],
                        stroke,
                    );
                }
                SignatureStatus::None => {}
            },
        );
    });
}

fn render_interlink_compose(app: &mut ZodeApp, ui: &mut egui::Ui) {
    let mut do_send = false;
    ui.horizontal(|ui| {
        let il = app.interlink_state.as_mut().unwrap();
        let should_focus = il.focus_compose;
        il.focus_compose = false;
        let resp = ui.add(
            text_input(&mut il.compose, ui.available_width() - 70.0).hint_text("Type a message..."),
        );
        if should_focus {
            resp.request_focus();
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            do_send = true;
            resp.request_focus();
        }
        if std_button(ui, "Send") {
            do_send = true;
        }
    });
    if do_send {
        app.send_message();
    }
}

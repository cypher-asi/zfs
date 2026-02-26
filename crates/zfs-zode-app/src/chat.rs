use std::sync::Arc;

use eframe::egui;
use hkdf::Hkdf;
use sha2::Sha256;
use zero_neural::ed25519_to_did_key;
use zero_neural::testkit::derive_machine_keypair_from_seed;
use zero_neural::MachineKeyCapabilities;
use zfs_core::{GossipSectorAppend, ProgramId, SectorId};
use zfs_crypto::{decrypt_sector, encrypt_sector, pad_to_bucket, unpad_from_bucket, SectorKey};
use zfs_programs::zchat::{ChannelId, ZChatDescriptor, ZChatMessage, TEST_CHANNEL_ID};
use zfs_storage::SectorStore;

use crate::app::ZodeApp;
use crate::components::{
    error_label, field_label, info_grid, kv_row, section, std_button, text_input,
};
use crate::helpers::format_timestamp_ms;
use crate::state::{ChatState, ChatUpdate, DisplayMessage};

fn derive_test_sector_key() -> SectorKey {
    let hk = Hkdf::<Sha256>::new(None, b"interlink-main-channel-v1");
    let mut key_bytes = [0u8; 32];
    hk.expand(b"zfs:test-channel-key:v1", &mut key_bytes)
        .expect("32-byte expand cannot fail");
    SectorKey::from_bytes(key_bytes)
}

fn derive_test_machine_did(zode_id: &str) -> String {
    use sha2::Digest;
    let hash: [u8; 32] =
        sha2::Sha256::digest(format!("interlink-main-machine:{zode_id}").as_bytes()).into();
    let identity_id = [0x01; 16];
    let machine_id = [0x02; 16];
    let caps = MachineKeyCapabilities::SIGN | MachineKeyCapabilities::ENCRYPT;
    let kp = derive_machine_keypair_from_seed(hash, &identity_id, &machine_id, 0, caps)
        .expect("deterministic derivation cannot fail");
    ed25519_to_did_key(&kp.public_key().ed25519_bytes())
}

fn build_aad(program_id: &ProgramId, sector_id: &SectorId) -> Vec<u8> {
    let mut aad = Vec::with_capacity(32 + sector_id.as_bytes().len());
    aad.extend_from_slice(program_id.as_bytes());
    aad.extend_from_slice(sector_id.as_bytes());
    aad
}

// ---------------------------------------------------------------------------
// ZodeApp chat lifecycle
// ---------------------------------------------------------------------------

impl ZodeApp {
    pub(crate) fn init_chat(&mut self) {
        let sector_key = derive_test_sector_key();
        let zode_id = self
            .zode
            .as_ref()
            .map(|z| z.status().zode_id)
            .unwrap_or_default();
        let machine_did = std::thread::spawn(move || derive_test_machine_did(&zode_id))
            .join()
            .expect("key derivation thread panicked");
        let channel_id = ChannelId::from_str_id(TEST_CHANNEL_ID);
        let program_id = ZChatDescriptor::v1()
            .program_id()
            .expect("Interlink descriptor is valid");
        let sector_id = channel_id.sector_id();

        let (update_tx, update_rx) = tokio::sync::mpsc::channel::<ChatUpdate>(4);
        let (refresh_tx, refresh_rx) = tokio::sync::mpsc::channel::<()>(4);

        if let Some(ref zode) = self.zode {
            Self::spawn_chat_updater(
                &self.rt,
                zode,
                &sector_key,
                program_id,
                sector_id.clone(),
                update_tx,
                refresh_rx,
            );
        }

        self.chat_state = Some(ChatState {
            messages: Vec::new(),
            compose: String::new(),
            sector_key,
            machine_did,
            channel_id,
            program_id,
            sector_id,
            error: None,
            initialized: true,
            scroll_to_bottom: true,
            update_rx,
            refresh_tx,
        });
    }

    fn spawn_chat_updater(
        rt: &tokio::runtime::Runtime,
        zode: &Arc<zfs_zode::Zode>,
        sector_key: &SectorKey,
        program_id: ProgramId,
        sector_id: SectorId,
        update_tx: tokio::sync::mpsc::Sender<ChatUpdate>,
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
            if let Some(ref mut chat) = self.chat_state {
                chat.error = Some("Zode is not running".into());
            }
            return;
        };
        let storage = Arc::clone(zode.storage());
        let chat = self.chat_state.as_mut().unwrap();
        let text = chat.compose.trim().to_string();
        if text.is_empty() {
            return;
        }
        chat.compose.clear();

        let msg = build_chat_message(chat, text);
        let aad = build_aad(&chat.program_id, &chat.sector_id);

        match encrypt_message(&msg, &chat.sector_key, &aad) {
            Ok(ciphertext) => {
                do_append(chat, &storage, zode, ciphertext);
            }
            Err(e) => {
                chat.error = Some(e);
            }
        }
    }
}

fn do_append(
    chat: &mut ChatState,
    storage: &Arc<zfs_storage::RocksStorage>,
    zode: &Arc<zfs_zode::Zode>,
    ciphertext: Vec<u8>,
) {
    match storage.append(&chat.program_id, &chat.sector_id, &ciphertext) {
        Ok(index) => {
            chat.error = None;
            broadcast_gossip(
                zode,
                chat.program_id,
                chat.sector_id.clone(),
                index,
                ciphertext,
            );
            let _ = chat.refresh_tx.try_send(());
        }
        Err(e) => {
            chat.error = Some(format!("Sector append failed: {e}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Message construction and encryption
// ---------------------------------------------------------------------------

fn build_chat_message(chat: &ChatState, text: String) -> ZChatMessage {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    ZChatMessage {
        sender_did: chat.machine_did.clone(),
        channel_id: chat.channel_id.clone(),
        content: text,
        timestamp_ms: now_ms,
        signature: Vec::new(),
    }
}

fn encrypt_message(msg: &ZChatMessage, key: &SectorKey, aad: &[u8]) -> Result<Vec<u8>, String> {
    let plaintext = msg
        .encode_canonical()
        .map_err(|e| format!("Encode failed: {e}"))?;
    let padded = pad_to_bucket(&plaintext);
    encrypt_sector(&padded, key, aad).map_err(|e| format!("Encrypt failed: {e}"))
}

// ---------------------------------------------------------------------------
// Background polling
// ---------------------------------------------------------------------------

async fn poll_new_entries(
    storage: &Arc<zfs_storage::RocksStorage>,
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
    known_len: u64,
    update_tx: &tokio::sync::mpsc::Sender<ChatUpdate>,
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
) -> ChatUpdate {
    let aad = build_aad(program_id, sector_id);
    let mut display: Vec<DisplayMessage> = Vec::new();
    let mut first_error: Option<String> = None;
    for ct in &entries {
        match decrypt_one(ct, sector_key, &aad) {
            Ok(msg) => display.push(msg),
            Err(e) if first_error.is_none() => first_error = Some(e),
            Err(_) => {}
        }
    }
    ChatUpdate {
        new_messages: display,
        error: first_error,
    }
}

fn decrypt_one(
    ciphertext: &[u8],
    sector_key: &SectorKey,
    aad: &[u8],
) -> Result<DisplayMessage, String> {
    let padded =
        decrypt_sector(ciphertext, sector_key, aad).map_err(|e| format!("Decrypt: {e}"))?;
    let plaintext = unpad_from_bucket(&padded).map_err(|e| format!("Unpad: {e}"))?;
    let msg = ZChatMessage::decode_canonical(&plaintext).map_err(|e| format!("Decode: {e}"))?;
    let sig_status = if msg.signature.is_empty() {
        crate::state::SignatureStatus::None
    } else {
        crate::state::SignatureStatus::Unknown
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
    zode: &Arc<zfs_zode::Zode>,
    program_id: ProgramId,
    sector_id: SectorId,
    index: u64,
    ciphertext: Vec<u8>,
) {
    let gossip = GossipSectorAppend {
        program_id,
        sector_id,
        index,
        payload: ciphertext,
        shape_proof: None,
    };
    let topic = zfs_programs::program_topic(&gossip.program_id);
    if let Ok(data) = zfs_core::encode_canonical(&gossip) {
        zode.publish(topic, data);
    }
}

// ---------------------------------------------------------------------------
// UI rendering
// ---------------------------------------------------------------------------

pub(crate) fn render_chat(app: &mut ZodeApp, ui: &mut egui::Ui) {
    if app.chat_state.is_none() || !app.chat_state.as_ref().unwrap().initialized {
        app.init_chat();
    }
    render_chat_header(app, ui);
    drain_chat_updates(app);
    render_chat_messages(app, ui);
    render_chat_compose(app, ui);
}

fn render_chat_header(app: &ZodeApp, ui: &mut egui::Ui) {
    let chat = app.chat_state.as_ref().unwrap();
    let key_preview: String = chat.sector_key.as_bytes()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let ch_display = String::from_utf8_lossy(chat.channel_id.as_bytes()).to_string();

    section(ui, "INTERLINK", |ui| {
        info_grid(ui, "chat_info_grid", |ui| {
            kv_row(ui, "Channel", &ch_display);

            field_label(ui, "Sector Key");
            ui.label(
                egui::RichText::new(format!("{key_preview}..."))
                    .monospace()
                    .weak(),
            );
            ui.end_row();

            kv_row(ui, "Messages", &format!("{}", chat.messages.len()));
            kv_row(ui, "Protocol", "/zfs/sector/2.0.0");
        });
    });

    if let Some(ref err) = chat.error {
        error_label(ui, err);
    }
}

fn drain_chat_updates(app: &mut ZodeApp) {
    let chat = app.chat_state.as_mut().unwrap();
    while let Ok(upd) = chat.update_rx.try_recv() {
        if upd.error.is_some() {
            chat.error = upd.error;
        }
        if !upd.new_messages.is_empty() {
            chat.messages.extend(upd.new_messages);
            chat.scroll_to_bottom = true;
        }
    }
}

fn short_sender(did: &str) -> String {
    if did.len() > 6 {
        format!("...{}", &did[did.len() - 6..])
    } else {
        did.to_string()
    }
}

fn render_chat_messages(app: &mut ZodeApp, ui: &mut egui::Ui) {
    let chat = app.chat_state.as_mut().unwrap();
    let should_scroll = chat.scroll_to_bottom;
    chat.scroll_to_bottom = false;

    let available = ui.available_height() - 40.0;
    egui::ScrollArea::vertical()
        .max_height(available.max(100.0))
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            let chat = app.chat_state.as_ref().unwrap();
            if chat.messages.is_empty() {
                ui.label(
                    egui::RichText::new("No messages yet. Type something below!")
                        .weak()
                        .italics(),
                );
            } else {
                for msg in &chat.messages {
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
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(format!("[{time}]")).monospace().weak());
        ui.label(egui::RichText::new(format!("{name}:")).monospace().strong());
        ui.label(&msg.content);
    });
}

fn render_chat_compose(app: &mut ZodeApp, ui: &mut egui::Ui) {
    let mut do_send = false;
    ui.horizontal(|ui| {
        let chat = app.chat_state.as_mut().unwrap();
        let resp = ui.add(
            text_input(&mut chat.compose, ui.available_width() - 70.0)
                .hint_text("Type a message..."),
        );
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

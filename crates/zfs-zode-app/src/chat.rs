use std::sync::Arc;

use eframe::egui;
use hkdf::Hkdf;
use sha2::Sha256;
use zero_neural::ed25519_to_did_key;
use zero_neural::testkit::derive_machine_keypair_from_seed;
use zero_neural::MachineKeyCapabilities;
use zfs_core::{GossipSector, ProgramId, SectorId};
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
        let sector_id = channel_id.sector_id();
        let program_id = ZChatDescriptor::v1()
            .program_id()
            .expect("Interlink descriptor is valid");

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
            let mut known_len = 0usize;
            loop {
                let cur_len = bg_storage
                    .get(&program_id, &sector_id)
                    .ok()
                    .flatten()
                    .map(|v| v.len())
                    .unwrap_or(0);
                if cur_len != known_len {
                    known_len = cur_len;
                    let upd =
                        build_chat_update(&bg_storage, &bg_key, &program_id, &sector_id);
                    if update_tx.send(upd).await.is_err() {
                        return;
                    }
                }
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

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let aad = build_aad(&chat.program_id, &chat.sector_id);

        let new_msg = ZChatMessage {
            sender_did: chat.machine_did.clone(),
            channel_id: chat.channel_id.clone(),
            content: text.clone(),
            timestamp_ms: now_ms,
        };

        let mut messages = load_messages(&storage, &chat.sector_key, &chat.program_id, &chat.sector_id, &aad);
        messages.push(new_msg);

        match encode_message_list(&messages) {
            Ok(plaintext) => {
                let padded = pad_to_bucket(&plaintext);
                match encrypt_sector(&padded, &chat.sector_key, &aad) {
                    Ok(ciphertext) => {
                        if let Err(e) = storage.put(
                            &chat.program_id,
                            &chat.sector_id,
                            &ciphertext,
                            true,
                            None,
                        ) {
                            chat.error = Some(format!("Sector write failed: {e}"));
                            return;
                        }
                        chat.messages.push(DisplayMessage {
                            sender: chat.machine_did.clone(),
                            content: text,
                            timestamp_ms: now_ms,
                        });
                        chat.error = None;
                        chat.scroll_to_bottom = true;
                        broadcast_gossip(
                            zode,
                            chat.program_id,
                            chat.sector_id.clone(),
                            ciphertext,
                        );
                        let _ = chat.refresh_tx.try_send(());
                    }
                    Err(e) => {
                        chat.error = Some(format!("Encrypt failed: {e}"));
                    }
                }
            }
            Err(e) => {
                chat.error = Some(format!("Encode failed: {e}"));
            }
        }
    }
}

fn encode_message_list(messages: &[ZChatMessage]) -> Result<Vec<u8>, String> {
    zfs_core::encode_canonical(&messages.to_vec()).map_err(|e| format!("{e}"))
}

fn decode_message_list(bytes: &[u8]) -> Result<Vec<ZChatMessage>, String> {
    zfs_core::decode_canonical(bytes).map_err(|e| format!("{e}"))
}

fn load_messages(
    storage: &Arc<zfs_storage::RocksStorage>,
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
    aad: &[u8],
) -> Vec<ZChatMessage> {
    match SectorStore::get(storage.as_ref(), program_id, sector_id) {
        Ok(Some(ciphertext)) => match decrypt_sector(&ciphertext, sector_key, aad) {
            Ok(padded) => unpad_from_bucket(&padded)
                .ok()
                .and_then(|pt| decode_message_list(&pt).ok())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn broadcast_gossip(
    zode: &Arc<zfs_zode::Zode>,
    program_id: ProgramId,
    sector_id: SectorId,
    ciphertext: Vec<u8>,
) {
    let gossip = GossipSector {
        program_id,
        sector_id,
        payload: ciphertext,
        overwrite: true,
    };
    let topic = zfs_programs::program_topic(&gossip.program_id);
    if let Ok(data) = zfs_core::encode_canonical(&gossip) {
        zode.publish(topic, data);
    }
}

fn build_chat_update(
    storage: &Arc<zfs_storage::RocksStorage>,
    sector_key: &SectorKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> ChatUpdate {
    let aad = build_aad(program_id, sector_id);

    let ciphertext = match SectorStore::get(storage.as_ref(), program_id, sector_id) {
        Ok(Some(ct)) => ct,
        Ok(None) => return ChatUpdate::empty(),
        Err(e) => return ChatUpdate::error(format!("Sector read failed: {e}")),
    };

    match decrypt_sector(&ciphertext, sector_key, &aad) {
        Ok(padded) => match unpad_from_bucket(&padded).and_then(|pt| {
            decode_message_list(&pt).map_err(|e| zfs_crypto::CryptoError::PaddingError(e))
        }) {
            Ok(msgs) => {
                let mut display: Vec<DisplayMessage> = msgs
                    .into_iter()
                    .map(|m| DisplayMessage {
                        sender: m.sender_did,
                        content: m.content,
                        timestamp_ms: m.timestamp_ms,
                    })
                    .collect();
                display.sort_by_key(|m| m.timestamp_ms);
                ChatUpdate {
                    messages: display,
                    error: None,
                }
            }
            Err(e) => ChatUpdate::error(format!("Decode failed: {e}")),
        },
        Err(e) => ChatUpdate::error(format!("Decrypt failed: {e}")),
    }
}


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
            kv_row(ui, "Protocol", "/zfs/sector/1.0.0");
        });
    });

    if let Some(ref err) = chat.error {
        error_label(ui, err);
    }
}

fn drain_chat_updates(app: &mut ZodeApp) {
    let chat = app.chat_state.as_mut().unwrap();
    while let Ok(upd) = chat.update_rx.try_recv() {
        chat.error = upd.error;
        chat.messages = upd.messages;
        chat.scroll_to_bottom = true;
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
                    let time = format_timestamp_ms(msg.timestamp_ms);
                    let name = short_sender(&msg.sender);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(format!("[{time}]")).monospace().weak());
                        ui.label(egui::RichText::new(format!("{name}:")).monospace().strong());
                        ui.label(&msg.content);
                    });
                }
            }
            if should_scroll {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
        });
    ui.separator();
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

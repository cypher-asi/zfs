use std::sync::Arc;

use eframe::egui;
use hkdf::Hkdf;
use sha2::Sha256;
use zfs_core::{Cid, GossipBlock, Head, ProgramId, SectorId};
use zfs_crypto::{decrypt_sector, encrypt_sector, SectorKey};
use zfs_programs::zchat::{ChannelId, ZChatDescriptor, ZChatMessage, TEST_CHANNEL_ID};
use zfs_storage::{BlockStore, HeadStore, ProgramIndex};
use zero_neural::{
    derive_machine_keypair, ed25519_to_did_key, MachineKeyCapabilities, NeuralKey,
};

use crate::app::ZodeApp;
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
    let nk = NeuralKey::from_bytes(hash);
    let identity_id = [0x01; 16];
    let machine_id = [0x02; 16];
    let caps = MachineKeyCapabilities::SIGN | MachineKeyCapabilities::ENCRYPT;
    let kp = derive_machine_keypair(&nk, &identity_id, &machine_id, 0, caps)
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
            last_head_cid: None,
            version: 0,
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
            let mut known = 0usize;
            loop {
                let cur = bg_storage
                    .list_cids(&program_id)
                    .map(|v| v.len())
                    .unwrap_or(known);
                if cur != known {
                    known = cur;
                    let upd = build_chat_update(&bg_storage, &bg_key, &program_id, &sector_id);
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

        match encode_and_encrypt(chat, &text, now_ms) {
            Ok((cid, ciphertext, head)) => {
                if let Err(e) = persist_message(&storage, &cid, &ciphertext, &head, &chat.program_id) {
                    chat.error = Some(e);
                    return;
                }
                chat.last_head_cid = Some(cid);
                chat.messages.push(DisplayMessage {
                    sender: chat.machine_did.clone(),
                    content: text,
                    timestamp_ms: now_ms,
                });
                chat.error = None;
                chat.scroll_to_bottom = true;
                broadcast_gossip(zode, chat.program_id, cid, ciphertext, head);
                let _ = chat.refresh_tx.try_send(());
            }
            Err(e) => {
                chat.error = Some(e);
            }
        }
    }
}

fn encode_and_encrypt(
    chat: &mut ChatState,
    text: &str,
    now_ms: u64,
) -> Result<(Cid, Vec<u8>, Head), String> {
    let msg = ZChatMessage {
        sender_did: chat.machine_did.clone(),
        channel_id: chat.channel_id.clone(),
        content: text.to_string(),
        timestamp_ms: now_ms,
    };
    let cbor = msg.encode_canonical().map_err(|e| format!("Encode failed: {e}"))?;
    let aad = build_aad(&chat.program_id, &chat.sector_id);
    let ciphertext =
        encrypt_sector(&cbor, &chat.sector_key, &aad).map_err(|e| format!("Encrypt failed: {e}"))?;
    let cid = Cid::from_ciphertext(&ciphertext);
    chat.version += 1;
    let head = Head {
        sector_id: chat.sector_id.clone(),
        cid,
        version: chat.version,
        program_id: chat.program_id,
        prev_head_cid: chat.last_head_cid,
        timestamp_ms: now_ms,
        signature: None,
    };
    Ok((cid, ciphertext, head))
}

fn persist_message(
    storage: &Arc<zfs_storage::RocksStorage>,
    cid: &Cid,
    ciphertext: &[u8],
    head: &Head,
    program_id: &ProgramId,
) -> Result<(), String> {
    storage
        .put(cid, ciphertext)
        .map_err(|e| format!("Block write failed: {e}"))?;
    storage
        .put_head(&head.sector_id, head)
        .map_err(|e| format!("Head write failed: {e}"))?;
    storage
        .add_cid(program_id, cid)
        .map_err(|e| format!("Index write failed: {e}"))?;
    Ok(())
}

fn broadcast_gossip(
    zode: &Arc<zfs_zode::Zode>,
    program_id: ProgramId,
    cid: Cid,
    ciphertext: Vec<u8>,
    head: Head,
) {
    let gossip = GossipBlock {
        program_id,
        cid,
        ciphertext,
        head: Some(head),
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

    let (last_head_cid, version) = match storage.get_head(sector_id) {
        Ok(Some(h)) => (Some(h.cid), h.version),
        Ok(None) => return empty_chat_update(None),
        Err(e) => return error_chat_update(format!("Failed to read head: {e}")),
    };

    let cids = match storage.list_cids(program_id) {
        Ok(c) => c,
        Err(e) => return error_chat_update(format!("Failed to list CIDs: {e}")),
    };

    let (mut msgs, error) = decrypt_messages(storage, &cids, sector_key, &aad);
    msgs.sort_by_key(|m| m.timestamp_ms);
    ChatUpdate {
        messages: msgs,
        last_head_cid,
        version,
        error,
    }
}

fn decrypt_messages(
    storage: &Arc<zfs_storage::RocksStorage>,
    cids: &[Cid],
    sector_key: &SectorKey,
    aad: &[u8],
) -> (Vec<DisplayMessage>, Option<String>) {
    let mut msgs = Vec::new();
    for cid in cids {
        match storage.get(cid) {
            Ok(Some(ciphertext)) => match decrypt_sector(&ciphertext, sector_key, aad) {
                Ok(plaintext) => match ZChatMessage::decode_canonical(&plaintext) {
                    Ok(msg) => msgs.push(DisplayMessage {
                        sender: msg.sender_did,
                        content: msg.content,
                        timestamp_ms: msg.timestamp_ms,
                    }),
                    Err(e) => msgs.push(DisplayMessage {
                        sender: "system".into(),
                        content: format!("[decode error: {e}]"),
                        timestamp_ms: 0,
                    }),
                },
                Err(e) => msgs.push(DisplayMessage {
                    sender: "system".into(),
                    content: format!("[decrypt error: {e}]"),
                    timestamp_ms: 0,
                }),
            },
            Ok(None) => msgs.push(DisplayMessage {
                sender: "system".into(),
                content: format!("[block not found: {}]", cid.to_hex()),
                timestamp_ms: 0,
            }),
            Err(e) => return (msgs, Some(format!("Storage read error: {e}"))),
        }
    }
    (msgs, None)
}

fn empty_chat_update(last_head_cid: Option<Cid>) -> ChatUpdate {
    ChatUpdate {
        messages: Vec::new(),
        last_head_cid,
        version: 0,
        error: None,
    }
}

fn error_chat_update(error: String) -> ChatUpdate {
    ChatUpdate {
        messages: Vec::new(),
        last_head_cid: None,
        version: 0,
        error: Some(error),
    }
}

pub(crate) fn render_chat(app: &mut ZodeApp, ui: &mut egui::Ui) {
    if app.zode.is_none() {
        ui.spinner();
        ui.label("Zode is stopped. Go to Settings to start.");
        return;
    }

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

    crate::components::section(ui, "INTERLINK", |ui| {
        crate::components::info_grid(ui, "chat_info_grid", |ui| {
            crate::components::kv_row(ui, "Channel", &ch_display);

            crate::components::field_label(ui, "Sector Key");
            ui.label(
                egui::RichText::new(format!("{key_preview}..."))
                    .monospace()
                    .weak(),
            );
            ui.end_row();

            crate::components::kv_row(ui, "Messages", &format!("{}", chat.messages.len()));
        });
    });

    if let Some(ref err) = chat.error {
        crate::components::error_label(ui, err);
    }
}

fn drain_chat_updates(app: &mut ZodeApp) {
    let chat = app.chat_state.as_mut().unwrap();
    while let Ok(upd) = chat.update_rx.try_recv() {
        chat.last_head_cid = upd.last_head_cid;
        chat.version = upd.version;
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
                        ui.label(
                            egui::RichText::new(format!("{name}:"))
                                .monospace()
                                .strong(),
                        );
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
            crate::components::text_input(&mut chat.compose, ui.available_width() - 70.0)
                .hint_text("Type a message..."),
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            do_send = true;
            resp.request_focus();
        }
        if crate::components::std_button(ui, "Send") {
            do_send = true;
        }
    });
    if do_send {
        app.send_message();
    }
}

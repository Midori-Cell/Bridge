//! The pages of the main area: a conversation (phone or other PC), the welcome page and Settings.

use std::sync::atomic::Ordering;

use bridge_core::{Device, Direction, Peer};
use egui::{vec2, CornerRadius, Rect, RichText, Ui};

use crate::app::{App, Conv, Data, Sheet};
use crate::chat::{self, Chat};
use crate::hub::SendKey;
use crate::common::{self, Compose, HistoryAction, MsgAction, TransferAction};
use crate::theme::{self, palette};
use crate::widgets;

/// A phone counts as online if it talked to the PC this recently (the phone asks for news every 10 s at most).
const ONLINE_WITHIN: f64 = 20.0;

impl App {
    // ------------------------------------------------------------ welcome

    pub(crate) fn welcome_page(&mut self, ui: &mut Ui) {
        widgets::title(ui, "Welcome to Bridge");
        widgets::footnote(ui, "Send notes and files between this PC and your phone, or another PC on the same network. Nothing leaves your network.");
        ui.add_space(14.0);
        widgets::grouped(ui, |g| {
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Pair a phone");
                    widgets::trailing(ui, |ui| {
                        if widgets::primary(ui, "Pair a phone").clicked() {
                            self.start_phone_pairing();
                        }
                    });
                });
                widgets::footnote(ui, "Scan a QR code with the Bridge app on the phone.");
            });
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Pair a PC");
                    widgets::trailing(ui, |ui| {
                        if widgets::primary(ui, "Pair a PC").clicked() {
                            self.sheet = Some(Sheet::PairPc);
                        }
                    });
                });
                widgets::footnote(ui, "Another PC on this network that runs Bridge.");
            });
        });
    }

    // ------------------------------------------------------------ conversation

    /// The whole conversation page scrolls: device card, chat, transfers, then the files.
    /// The chat takes the height of the window minus a strip at the bottom, so the transfers (which
    /// may need an answer) and the top of the file list stay visible without scrolling.
    pub(crate) fn conversation_page(&mut self, ui: &mut Ui, c: &Conv, d: &Data) {
        const PEEK: f32 = 118.0;
        const AUTO_MAX: f32 = 520.0;
        const MAX_CHAT: f32 = 1600.0;
        let viewport = ui.available_height();
        egui::ScrollArea::vertical().id_salt("conversation").auto_shrink([false, false]).show(ui, |ui| {
            let top = ui.cursor().min.y;
            if c.phone {
                widgets::section_first(ui, "PAIRED PHONES");
                widgets::grouped(ui, |g| {
                    for ph in &d.phones {
                        g.row(|ui| self.phone_row(ui, ph, d.now));
                    }
                });
            } else if let Some(pc) = d.peers.iter().find(|p| p.id == c.id) {
                widgets::section_first(ui, "PAIRED PC");
                widgets::grouped(ui, |g| g.row(|ui| self.pc_row(ui, pc)));
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("CHAT WITH {}", c.name.to_uppercase())).small().strong().color(palette(ui).label2));
                widgets::trailing(ui, |ui| {
                    ui.add_space(4.0);
                    if widgets::plain(ui, "Clear chat").on_hover_text("Delete the chat on this PC. The file list below keeps its files, and the other device keeps its own copy.").clicked() {
                        if c.phone {
                            self.hub.bridge.clear_chat(&c.id);
                        } else {
                            self.hub.bridge.clear_peer_chat(&c.id);
                        }
                    }
                });
            });
            let used = ui.cursor().min.y - top;
            // Without a height set by the user the chat follows the window until it is AUTO_MAX high;
            // after that a taller window shows more of the notes instead.
            let auto = (viewport - used - PEEK).clamp(chat::MIN_HEIGHT, AUTO_MAX);
            let height = self.chat_h.map_or(auto, |h| h.clamp(chat::MIN_HEIGHT, MAX_CHAT));

            // a message that was deleted, or belongs to another conversation, is not answered any more
            if self.reply_mid.as_ref().is_some_and(|m| !c.notes.iter().any(|n| &n.mid == m)) {
                self.reply_mid = None;
            }
            let reply = self.reply_mid.as_deref().and_then(|m| c.notes.iter().find(|n| n.mid == m));
            // after sending, the chat jumps to the new message as soon as it is in the list
            let jump_to_end = self.jump_after.is_some_and(|n| c.notes.len() > n);
            if jump_to_end || self.jump_after.is_some_and(|n| n > c.notes.len() + 1) {
                self.jump_after = None;
            }
            let (text, hint) = if c.phone { (&mut self.phone_text, "Type or paste a note for the phone") } else { (&mut self.pc_text, "Type or paste a message") };
            let out = chat::show(ui, height, Chat { id: &c.id, jump_to_end, notes: &c.notes, text, hint, clipboard: c.phone, send_key: self.hub.settings().send_key, reply, zoom: self.chat_zoom, widths: self.bubble_w, time_24h: self.hub.settings().time_24h });
            self.chat_rect = Some(out.rect);
            if let Some((own, w)) = out.width {
                if own {
                    self.bubble_w.own = Some(w);
                } else {
                    self.bubble_w.theirs = Some(w);
                }
                self.bubble_changed = Some(std::time::Instant::now());
            }
            self.compose_action(out.compose, c);
            if let Some(a) = out.msg {
                self.message_action(ui.ctx(), a, c);
            }

            self.chat_grip(ui, height);

            // Transfers sit under the chat and push the notes further down; the chat keeps its size.
            let (phone, cid) = (c.phone, c.id.clone());
            let list = self.visible_transfers(|t| {
                if phone { matches!(t.dir, Direction::ToPc | Direction::ToPhone) && t.device_id == cid } else { matches!(t.dir, Direction::ToPeer | Direction::FromPeer) && t.device_id == cid }
            });
            match common::transfers(ui, &list) {
                Some(TransferAction::Accept(id)) => self.hub.accept_file(&id),
                Some(TransferAction::Decline(id)) => self.hub.decline_file(&id),
                Some(TransferAction::Cancel(id)) => self.hub.cancel_transfer(&id),
                Some(TransferAction::Remove(id)) => self.hub.dismiss_transfer(&id),
                None => {}
            }

            self.notes_section(ui, c);
            ui.add_space(24.0);
        });
    }

    /// A grip under the chat: drag it to make the chat taller or shorter (the page scrolls to fit),
    /// double-click to let the chat follow the window again.
    fn chat_grip(&mut self, ui: &mut Ui, height: f32) {
        let p = palette(ui);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), egui::Sense::click_and_drag());
        let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        let active = resp.hovered() || resp.dragged();
        let pill = Rect::from_center_size(rect.center(), vec2(if active { 56.0 } else { 40.0 }, 4.0));
        // the explanation appears after the pointer has rested on the grip for a moment
        let key = theme::id("chat_grip_hover");
        let now = ui.input(|i| i.time);
        let since: Option<f64> = ui.data(|d| d.get_temp(key));
        let since = if resp.hovered() && !resp.dragged() { Some(since.unwrap_or(now)) } else { None };
        ui.data_mut(|d| match since {
            Some(t) => {
                d.insert_temp(key, t);
            }
            None => d.remove::<f64>(key),
        });
        const TIP_DELAY: f64 = 1.25;
        if since.is_some_and(|t| now - t < TIP_DELAY) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        }
        if since.is_some_and(|t| now - t >= TIP_DELAY) {
            // The built-in tooltip hangs at the left edge of the (full width) grip; this one is centered under the pill.
            egui::Area::new(theme::id("chat_grip_tip"))
                .order(egui::Order::Tooltip)
                .fixed_pos(egui::pos2(pill.center().x, pill.max.y + 10.0))
                .pivot(egui::Align2::CENTER_TOP)
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(260.0);
                        ui.label("Drag to change the height of the chat. Double-click to follow the window again.");
                    });
                });
        }
        ui.painter().rect_filled(pill, CornerRadius::same(2), if active { p.label2 } else { p.outline });
        if resp.dragged() {
            self.chat_h = Some((height + resp.drag_delta().y).clamp(chat::MIN_HEIGHT, 1600.0));
        }
        if resp.double_clicked() {
            self.chat_h = None;
        }
        if resp.drag_stopped() || resp.double_clicked() {
            let h = self.chat_h;
            self.hub.change_settings(|s| s.chat_height = h);
        }
    }

    fn message_action(&mut self, ctx: &egui::Context, action: MsgAction, c: &Conv) {
        match action {
            MsgAction::Reply(mid) => {
                self.reply_mid = Some(mid);
                ctx.memory_mut(|m| m.request_focus(chat::composer_id()));
            }
            MsgAction::ClearReply => self.reply_mid = None,
            MsgAction::Copy(text) => {
                ctx.copy_text(text);
                self.hub.toast("Copied", false);
            }
            MsgAction::Delete(mid) => {
                if self.reply_mid.as_deref() == Some(mid.as_str()) {
                    self.reply_mid = None;
                }
                self.hub.delete_message(c.phone, &c.id, &mid);
            }
            MsgAction::Open(f) => self.open_file(&f, &c.notes),
            MsgAction::ShowFolder(f) => self.hub.show_file(&f),
            MsgAction::OpenLink(url) => {
                if let Err(e) = open::that_detached(&url) {
                    self.hub.toast(format!("Could not open the link: {e}"), true);
                }
            }
        }
    }

    fn compose_action(&mut self, action: Compose, c: &Conv) {
        if matches!(action, Compose::SendText) {
            self.jump_after = Some(c.notes.len());
        }
        match (action, c.phone) {
            (Compose::SendText, true) => match self.hub.send_text_reply(&c.id, &self.phone_text, self.reply_mid.as_deref()) {
                Ok(()) => {
                    self.phone_text.clear();
                    self.reply_mid = None;
                }
                Err(e) => self.hub.toast(e, true),
            },
            (Compose::SendText, false) => {
                let text = std::mem::take(&mut self.pc_text);
                self.hub.send_peer_text(&c.id, text, self.reply_mid.take(), || {});
            }
            (Compose::SendClipboard, true) => {
                let (hub, id) = (self.hub.clone(), c.id.clone());
                std::thread::spawn(move || hub.send_clipboard_to(&id));
            }
            (Compose::PickFiles, true) => self.hub.pick_and_send_files(&c.id, &c.name),
            (Compose::PickFiles, false) => self.hub.pick_and_send_peer_files(&c.id, &c.name),
            _ => {}
        }
    }

    /// The history under the transfers, with Copy, Open and Show folder. It scrolls with the page.
    pub(crate) fn notes_section(&mut self, ui: &mut Ui, c: &Conv) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("FILES").small().strong().color(palette(ui).label2));
            widgets::trailing(ui, |ui| {
                ui.add_space(4.0);
if widgets::plain(ui, "Clear list").on_hover_text("Delete the file list on this PC. The chat above keeps its messages, and the other device keeps its own copy.").clicked() {
                    if c.phone {
                        self.hub.bridge.clear_list(&c.id);
                    } else {
                        self.hub.bridge.clear_peer_list(&c.id);
                    }
                }
                if widgets::plain(ui, "Open inbox").clicked() {
                    self.hub.open_inbox();
                }
            });
        });
        let files: Vec<_> = c.list.iter().filter(|n| n.kind == bridge_core::NoteKind::File).cloned().collect();
        match common::history(ui, &files) {
            Some(HistoryAction::Copy(t)) => ui.ctx().copy_text(t),
            Some(HistoryAction::Open(f)) => self.open_file(&f, &c.list),
            Some(HistoryAction::ShowFolder(f)) => self.hub.show_file(&f),
            None => {}
        }
    }

    fn phone_row(&mut self, ui: &mut Ui, d: &Device, now: f64) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&d.name).strong());
            let online = d.last_seen.is_some_and(|t| now - t < ONLINE_WITHIN);
            let status = match d.last_seen {
                Some(_) if online => "Online".to_string(),
                Some(t) => format!("Last seen {}", common::ago(now - t)),
                None => "Not seen yet".to_string(),
            };
            widgets::status(ui, online, &status);
            widgets::trailing(ui, |ui| {
                if widgets::plain_danger(ui, "Unpair").clicked() {
                    self.sheet = Some(Sheet::UnpairPhone { id: d.id.clone(), name: d.name.clone() });
                }
                if widgets::plain(ui, "Rename").clicked() {
                    self.sheet = Some(Sheet::RenamePhone { id: d.id.clone(), text: d.name.clone() });
                }
            });
        });
    }

    fn pc_row(&mut self, ui: &mut Ui, pc: &Peer) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&pc.name).strong());
            widgets::status(ui, pc.online, if pc.online { "Online" } else { "Not seen" });
            widgets::trailing(ui, |ui| {
                if widgets::plain_danger(ui, "Unpair").clicked() {
                    self.sheet = Some(Sheet::UnpairPc { id: pc.id.clone(), name: pc.name.clone() });
                }
            });
        });
    }

    // ------------------------------------------------------------ settings

    pub(crate) fn settings_page(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        widgets::title(ui, "Settings");
        let b = self.hub.bridge.clone();

        widgets::section(ui, "THIS PC");
        widgets::grouped(ui, |g| {
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    widgets::trailing(ui, |ui| {
                        let e = ui.add(egui::TextEdit::singleline(&mut self.pc_name_edit).desired_width(220.0).char_limit(40));
                        if e.lost_focus() {
                            if self.pc_name_edit.trim().is_empty() {
                                self.pc_name_edit = b.pc_name();
                            } else {
                                b.set_pc_name(&self.pc_name_edit);
                                self.pc_name_edit = b.pc_name();
                            }
                        }
                    });
                });
            });
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Visible to other PCs");
                    widgets::trailing(ui, |ui| {
                        let mut on = b.is_discoverable();
                        if widgets::switch(ui, &mut on).changed() {
                            b.set_discoverable(on);
                        }
                    });
                });
            });
        });
        widgets::footnote(ui, "Other PCs on this network see this name and nothing else. When this is off they cannot find this PC, but PCs you already paired with can still reach it.");

        widgets::section(ui, "APPEARANCE");
        let mut s = self.hub.settings();
        let before = s.clone();
        widgets::grouped(ui, |g| {
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Highlight color");
                    widgets::trailing(ui, |ui| {
                        // the custom choice is the rightmost; the ready-made ones follow
                        let custom = s.accent == theme::Accent::Custom;
                        let resp = widgets::swatch_custom(ui, custom.then(|| theme::custom_fill(s.custom_accent)), "Choose your own color");
                        if resp.clicked() {
                            s.accent = theme::Accent::Custom;
                        }
                        egui::Popup::from_toggle_button_response(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                            custom_color_picker(ui, &mut s.custom_accent);
                            s.accent = theme::Accent::Custom;
                        });
                        for a in theme::Accent::ALL.into_iter().rev() {
                            if widgets::swatch(ui, theme::accent_fill(a), s.accent == a, a.name()).clicked() {
                                s.accent = a;
                            }
                        }
                    });
                });
            });
        });
        widgets::footnote(ui, &format!("{}. Used for the main buttons, the selected entry and links. A custom color is darkened a little if white text would be hard to read on it.", s.accent.name()));
        if s.accent != before.accent || s.custom_accent != before.custom_accent {
            theme::set_accent(s.accent, s.custom_accent);
            theme::install(ui.ctx());
        }
        ui.add_space(8.0);
        widgets::grouped(ui, |g| {
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Chat size");
                    widgets::trailing(ui, |ui| {
                        if widgets::secondary(ui, "Reset").on_hover_text("Back to 100 %").clicked() {
                            s.chat_zoom = 1.0;
                            self.chat_zoom = 1.0;
                            self.chat_zoom_saved = 1.0;
                            self.chat_zoom_changed = None;
                        }
                        ui.label(format!("{} %", (self.chat_zoom * 100.0).round()));
                    });
                });
            });
        });
        widgets::footnote(ui, "Hold Ctrl and turn the mouse wheel over the chat to change the size of the messages and the text box only. Anywhere else the wheel changes the size of the whole window.");
        ui.add_space(8.0);
        widgets::grouped(ui, |g| {
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Message width");
                    widgets::trailing(ui, |ui| {
                        if widgets::secondary(ui, "Reset").on_hover_text("Every message as wide as its text again").clicked() {
                            self.bubble_w = chat::Widths::default();
                            s.bubble_own = None;
                            s.bubble_theirs = None;
                            self.bubble_changed = None;
                        }
                        let show = |w: Option<f32>| w.map_or("automatic".to_string(), |w| format!("{} pt", w.round()));
                        ui.label(format!("mine: {} \u{b7} theirs: {}", show(self.bubble_w.own), show(self.bubble_w.theirs)));
                    });
                });
            });
        });
        widgets::footnote(ui, "Drag a message in the chat to the left or right to set the width of all your messages, or of all incoming ones. Reset makes every message as wide as its text again.");

        widgets::grouped(ui, |g| {
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Time format");
                    widgets::trailing(ui, |ui| {
                        let label = |h24: bool| if h24 { "24-hour (21:14)" } else { "12-hour (9:14 PM)" };
                        egui::ComboBox::from_id_salt("time_format").selected_text(label(s.time_24h)).show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.time_24h, true, label(true));
                            ui.selectable_value(&mut s.time_24h, false, label(false));
                        });
                    });
                });
            });
        });
        widgets::footnote(ui, "How the time under each message is written.");

        widgets::section(ui, "BEHAVIOR");
        let tray_ok = self.hub.tray_ok.load(Ordering::SeqCst);
        widgets::grouped(ui, |g| {
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled(tray_ok, egui::Label::new("Keep running in the tray when the window is closed"));
                    widgets::trailing(ui, |ui| ui.add_enabled_ui(tray_ok, |ui| widgets::switch(ui, &mut s.keep_in_tray)));
                });
            });
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Ask before receiving a file");
                    widgets::trailing(ui, |ui| {
                        let mut ask = !b.auto_accept_files();
                        if widgets::switch(ui, &mut ask).changed() {
                            b.set_auto_accept_files(!ask);
                        }
                    });
                });
            });
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Stop a file request that is not accepted after");
                    widgets::trailing(ui, |ui| {
                        ui.label("seconds");
                        let id = theme::id("offer_timeout_edit");
                        if !ui.memory(|m| m.has_focus(id)) {
                            self.offer_timeout_edit = b.offer_timeout_secs().to_string();
                        }
                        let e = ui.add(egui::TextEdit::singleline(&mut self.offer_timeout_edit).id(id).desired_width(56.0).align(egui::Align2::RIGHT_CENTER));
                        self.offer_timeout_edit.retain(|c| c.is_ascii_digit());
                        self.offer_timeout_edit.truncate(5);
                        if e.lost_focus() {
                            b.set_offer_timeout_secs(self.offer_timeout_edit.parse::<u32>().unwrap_or(0).min(86_400));
                        }
                    });
                });
            });
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Send a message with");
                    widgets::trailing(ui, |ui| {
                        egui::ComboBox::from_id_salt("send_key").selected_text(s.send_key.label()).show_ui(ui, |ui| {
                            for k in SendKey::ALL {
                                ui.selectable_value(&mut s.send_key, k, k.label());
                            }
                        });
                    });
                });
            });
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Send clipboard to phone hotkey");
                    widgets::trailing(ui, |ui| {
                        // laid out from the right: the switch first, the keys button to its left
                        widgets::switch(ui, &mut s.hotkey_enabled);
                        ui.add_enabled_ui(s.hotkey_enabled, |ui| {
                            if widgets::secondary(ui, &s.hotkey.text()).on_hover_text("Click to choose other keys").clicked() {
                                self.keep.pause_hotkey();
                                self.sheet = Some(Sheet::Hotkey { captured: s.hotkey.keys.clone(), down: Vec::new(), holding: false });
                            }
                        });
                    });
                });
            });
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Copy text from the phone to this PC's clipboard");
                    widgets::trailing(ui, |ui| widgets::switch(ui, &mut s.copy_incoming_text));
                });
            });
        });
        if !tray_ok {
            widgets::footnote(ui, "No tray icon is available on this desktop, so closing the window quits.");
        } else if cfg!(target_os = "linux") {
            widgets::footnote(ui, "Needs a system tray. GNOME shows none unless the AppIndicator extension is installed.");
            if self.hub.x11_forced.load(Ordering::SeqCst) {
                widgets::footnote(ui, "On Wayland, Bridge opens through XWayland while this is on, because Wayland has no way to hide a window.");
            } else if s.keep_in_tray && !self.hub.window_can_hide.load(Ordering::SeqCst) {
                widgets::footnote(ui, "This is a Wayland session, which cannot hide a window, so closing the window still quits. Restart Bridge and it opens through XWayland, where hiding works.");
            }
        }
        widgets::footnote(ui, "The hotkey works while Bridge is in the background. Several keys at once are possible, for example Ctrl+Alt+B or C+P. Two ordinary keys (like C+P) are watched for instead of being reserved, so the keys also reach other programs; on a Mac that needs the Accessibility permission.");
        widgets::footnote(ui, "With Enter, Shift+Enter starts a new line. With the other choices, Enter starts a new line.");
        widgets::footnote(ui, "A file you send to another PC is withdrawn when nobody accepts it within this many seconds. 0 means no limit: it waits until the other person answers (the other PC drops an unanswered request after 15 minutes anyway).");
        widgets::footnote(ui, "When asking is on, a phone or PC that wants to send a file waits until you accept it. Text from other PCs is never copied to the clipboard automatically.");
        widgets::section(ui, "NOTIFICATIONS");
        widgets::grouped(ui, |g| {
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Show a popup for new messages and files");
                    widgets::trailing(ui, |ui| widgets::switch(ui, &mut s.notify_popup));
                });
            });
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Play a sound");
                    widgets::trailing(ui, |ui| widgets::switch(ui, &mut s.notify_sound));
                });
            });
            if s.notify_sound {
                g.row(|ui| {
                    ui.horizontal(|ui| {
                        ui.label("Sound");
                        widgets::trailing(ui, |ui| {
                            if widgets::secondary(ui, "Play").clicked() {
                                crate::sound::play(s.notify_sound_kind, s.notify_volume);
                            }
                            let before_kind = s.notify_sound_kind;
                            egui::ComboBox::from_id_salt("notify_sound_kind").selected_text(s.notify_sound_kind.label()).show_ui(ui, |ui| {
                                for k in crate::sound::SoundKind::ALL {
                                    ui.selectable_value(&mut s.notify_sound_kind, k, k.label());
                                }
                            });
                            // choosing a sound plays it, so the list can be tried out
                            if s.notify_sound_kind != before_kind {
                                crate::sound::play(s.notify_sound_kind, s.notify_volume);
                            }
                        });
                    });
                });
                g.row(|ui| {
                    ui.horizontal(|ui| {
                        ui.label("Volume");
                        widgets::trailing(ui, |ui| {
                            let r = ui.add(egui::Slider::new(&mut s.notify_volume, 5..=100).suffix(" %"));
                            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                                crate::sound::play(s.notify_sound_kind, s.notify_volume);
                            }
                        });
                    });
                });
            }
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Make the icon blink");
                    widgets::trailing(ui, |ui| widgets::switch(ui, &mut s.notify_blink));
                });
            });
        });
        widgets::footnote(ui, "Only while the Bridge window is not in front. The icon blinks in the taskbar or Dock and, with a tray icon, in the tray, until you open Bridge. The sound is Bridge's own and plays on the default audio output; the popup comes from the system (on Linux notify-send).");
        if s != before {
            let hotkey_changed = s.hotkey != before.hotkey || s.hotkey_enabled != before.hotkey_enabled;
            self.hub.change_settings(|x| *x = s);
            if hotkey_changed {
                self.keep.apply_hotkey(&self.hub);
            }
        }

        widgets::section(ui, "CONNECTION");
        let hotkey = self.hub.hotkey_note.lock().unwrap_or_else(|e| e.into_inner()).clone();
        widgets::grouped(ui, |g| {
            detail(g, "Phone address", &format!("https://{}:{}", b.lan_host(), b.lan_port()));
            detail(g, "Address for other PCs", &format!("{}:{}", b.lan_host(), b.peer_port()));
            detail(g, "Hotkey", &hotkey);
            g.row(|ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Received files").small().color(p.label2));
                        ui.add(egui::Label::new(b.inbox_dir().display().to_string()).wrap());
                    });
                    widgets::trailing(ui, |ui| {
                        if widgets::plain(ui, "Open").clicked() {
                            self.hub.open_inbox();
                        }
                        if widgets::plain(ui, "Change...").clicked() {
                            self.sheet = Some(Sheet::ChooseFolder { path: b.inbox_dir(), first_run: false });
                        }
                    });
                });
            });
            detail(g, "Program data", &b.data_dir().display().to_string());
        });

        widgets::section(ui, "SECURITY");
        widgets::grouped(ui, |g| {
            g.row(|ui| {
                ui.label(RichText::new("Certificate fingerprint (SHA-256)").small().color(p.label2));
                ui.add(egui::Label::new(RichText::new(b.fingerprint()).monospace().size(12.0)).wrap());
            });
        });
        widgets::footnote(ui, "This is how other devices recognize this PC. A phone or PC that was paired will refuse to talk to anything with a different fingerprint.");

        widgets::section(ui, "ABOUT");
        widgets::grouped(ui, |g| {
            detail(g, "Version", env!("CARGO_PKG_VERSION"));
        });
        widgets::footnote(ui, "Bridge never goes online by itself. To see whether this is the newest version, compare it with the latest release on the project's GitHub page.");
    }
}

fn detail(g: &mut widgets::Rows, label: &str, value: &str) {
    g.row(|ui| {
        let p = palette(ui);
        ui.label(RichText::new(label).small().color(p.label2));
        ui.add(egui::Label::new(value).wrap());
    });
}

/// Colour wheel and a hex field for the custom highlight color.
/// Hue, saturation and value, each 0..=1, worked out directly on the sRGB numbers so what is shown is what is saved.
type Hsv = [f32; 3];

fn hsv_to_rgb([h, s, v]: Hsv) -> [u8; 3] {
    let h = (h.clamp(0.0, 1.0) * 6.0).min(5.999_999);
    let (i, f) = (h.floor(), h - h.floor());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as u8 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [(r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8]
}

fn rgb_to_hsv(rgb: [u8; 3]) -> Hsv {
    let [r, g, b] = rgb.map(|c| c as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    [h, if max == 0.0 { 0.0 } else { d / max }, max]
}

/// A colored rectangle made of a grid of vertices, one color per vertex.
fn gradient(ui: &Ui, rect: egui::Rect, cols: usize, rows: usize, color: impl Fn(f32, f32) -> [u8; 3]) {
    let mut mesh = egui::Mesh::default();
    for y in 0..=rows {
        for x in 0..=cols {
            let (u, v) = (x as f32 / cols as f32, y as f32 / rows as f32);
            let c = color(u, v);
            mesh.colored_vertex(egui::pos2(rect.left() + u * rect.width(), rect.top() + v * rect.height()), egui::Color32::from_rgb(c[0], c[1], c[2]));
        }
    }
    for y in 0..rows {
        for x in 0..cols {
            let i = (y * (cols + 1) + x) as u32;
            let w = (cols + 1) as u32;
            mesh.add_triangle(i, i + 1, i + w);
            mesh.add_triangle(i + 1, i + w + 1, i + w);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

fn marker(ui: &Ui, at: egui::Pos2) {
    ui.painter().circle_stroke(at, 8.0, egui::Stroke::new(3.0, egui::Color32::BLACK.gamma_multiply(0.35)));
    ui.painter().circle_stroke(at, 7.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
}

/// The picker of the custom highlight color: a large saturation/brightness square, a hue bar and a hex field.
/// The position follows the pointer directly (not the movement since the last frame), so it cannot drift or jump.
fn custom_color_picker(ui: &mut Ui, rgb: &mut [u8; 3]) {
    const WIDTH: f32 = 320.0;
    // Keep the hue and saturation being dragged: RGB alone loses the hue of greys and blacks.
    let hsv_id = theme::id("custom_hsv");
    let mut hsv: Hsv = ui.data(|d| d.get_temp(hsv_id)).filter(|h: &Hsv| hsv_to_rgb(*h) == *rgb).unwrap_or_else(|| rgb_to_hsv(*rgb));
    let mut changed = false;
    ui.set_width(WIDTH);

    let (square, resp) = ui.allocate_exact_size(egui::vec2(WIDTH, 220.0), egui::Sense::click_and_drag());
    let hue = hsv[0];
    gradient(ui, square, 32, 20, |u, v| hsv_to_rgb([hue, u, 1.0 - v]));
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            hsv[1] = ((pos.x - square.left()) / square.width()).clamp(0.0, 1.0);
            hsv[2] = 1.0 - ((pos.y - square.top()) / square.height()).clamp(0.0, 1.0);
            changed = true;
        }
    }
    marker(ui, egui::pos2(square.left() + hsv[1] * square.width(), square.top() + (1.0 - hsv[2]) * square.height()));

    ui.add_space(10.0);
    let (bar, resp) = ui.allocate_exact_size(egui::vec2(WIDTH, 22.0), egui::Sense::click_and_drag());
    gradient(ui, bar, 36, 1, |u, _| hsv_to_rgb([u, 1.0, 1.0]));
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            hsv[0] = ((pos.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
            changed = true;
        }
    }
    marker(ui, egui::pos2(bar.left() + hsv[0] * bar.width(), bar.center().y));

    ui.add_space(10.0);
    let mut c = hsv_to_rgb(hsv);
    let id = theme::id("custom_hex");
    ui.horizontal(|ui| {
        let (swatch, _) = ui.allocate_exact_size(egui::vec2(36.0, 26.0), egui::Sense::hover());
        ui.painter().rect_filled(swatch, egui::CornerRadius::same(6), egui::Color32::from_rgb(c[0], c[1], c[2]));
        ui.label("Hex");
        let editing = ui.memory(|m| m.has_focus(id));
        let mut text: String = if editing { ui.data(|d| d.get_temp(id)).unwrap_or_default() } else { format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]) };
        let e = ui.add(egui::TextEdit::singleline(&mut text).id(id).desired_width(90.0).char_limit(7).font(egui::TextStyle::Monospace));
        if e.changed() {
            let t = text.trim().trim_start_matches('#');
            if t.len() == 6 {
                if let Ok(v) = u32::from_str_radix(t, 16) {
                    c = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
                    hsv = rgb_to_hsv(c);
                    changed = true;
                }
            }
        }
        if e.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, text));
        }
    });
    if changed {
        *rgb = hsv_to_rgb(hsv);
        ui.data_mut(|d| d.insert_temp(hsv_id, hsv));
    }
}

//! The window shell: sidebar, drop handling, sheets (modal tasks) and the little messages in the corner.
//! The pages themselves are in `pages.rs`, the chat in `chat.rs`.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bridge_core::{Device, Direction, Note, NoteKind, OutPairState, Peer, PairingInfo, TransferInfo, TransferState};
use egui::{Align, Align2, Color32, FontId, Frame, LayerId, Layout, Margin, Order, RichText, Sense, Vec2};

use crate::shortcut::Shortcut;
use crate::common::{self, short_ago};
use crate::hub::Hub;
use crate::icons::{self, Icon};
use crate::theme::{self, palette, palette_of, Palette};
use crate::widgets::{self, SheetResult};
use crate::tray;

const SHOW_DONE_FOR: Duration = Duration::from_secs(20);
const SHOW_PROBLEMS_FOR: Duration = Duration::from_secs(180);
pub const DEFAULT_PEER_PORT: u16 = 8444;
pub const MAX_NOTES_SHOWN: usize = 100;
const SIDEBAR_WIDTH: f32 = 208.0;
pub const MIN_ZOOM: f32 = 0.6;
pub const MAX_ZOOM: f32 = 2.0;
pub const MIN_CHAT_ZOOM: f32 = 0.7;
pub const MAX_CHAT_ZOOM: f32 = 2.2;

/// What the main area shows. `Auto` means "nothing chosen yet": the first phone or PC is picked,
/// or the welcome page is shown when nothing is paired.
#[derive(Clone, PartialEq, Eq)]
pub enum Sel {
    Auto,
    Phone(String),
    Pc(String),
    Settings,
}

pub struct PairView {
    pub info: PairingInfo,
    pub started: Instant,
    pub qr: Option<qrcode::QrCode>,
    pub phones_before: usize,
}

/// A modal task. Only one is open at a time; an incoming pairing request from another PC goes first.
pub enum Sheet {
    PairPhone(PairView),
    PairPc,
    PairByAddress { host: String, port: String, error: Option<String> },
    Contacting(String),
    Outgoing { id: String, confirmed: bool },
    Message { title: String, text: String },
    RenamePhone { id: String, text: String },
    UnpairPhone { id: String, name: String },
    UnpairPc { id: String, name: String },
    /// Where received files are saved. `first_run`: shown once at the first start, cannot be cancelled.
    ChooseFolder { path: std::path::PathBuf, first_run: bool },
    /// Choosing the hotkey: the keys of the current try (in the order they were pressed), the keys held
    /// right now, and whether a hold is going on.
    Hotkey { captured: Vec<String>, down: Vec<String>, holding: bool },
}

/// Everything the screen needs from the core, read once per frame.
pub struct Data {
    pub phones: Vec<Device>,
    pub peers: Vec<Peer>,
    /// The conversation of each phone, by phone id. A phone's chat shows only its own messages.
    pub phone_notes: HashMap<String, Vec<Note>>,
    pub peer_notes: HashMap<String, Vec<Note>>,
    pub now: f64,
}

/// The open conversation (a phone or another PC) with its messages, oldest first.
pub struct Conv {
    pub phone: bool,
    pub id: String,
    pub name: String,
    /// What the chat shows (messages not cleared from the chat).
    pub notes: Vec<Note>,
    /// What the list "Notes and files" shows (entries not cleared from the list).
    pub list: Vec<Note>,
}

/// The messages of a conversation as the chat shows them, and as the list "Notes and files" shows them.
fn split_views(all: Option<&Vec<Note>>) -> (Vec<Note>, Vec<Note>) {
    let all = all.map(Vec::as_slice).unwrap_or_default();
    (all.iter().filter(|n| !n.chat_hidden).cloned().collect(), all.iter().filter(|n| !n.list_hidden).cloned().collect())
}

pub struct App {
    pub hub: Arc<Hub>,
    min_watch: crate::minimize::Watch,
    pub(crate) keep: tray::Keep,
    pub sel: Sel,
    pub search: String,
    pub phone_text: String,
    pub pc_text: String,
    pub pc_name_edit: String,
    pub sheet: Option<Sheet>,
    /// The message the text box is answering (its id).
    pub reply_mid: Option<String>,
    /// Interface scale last seen, and when it changed, so it is saved once the user stops zooming.
    /// Height of the chat set by dragging its lower edge (None: follows the window, up to a limit).
    pub(crate) chat_h: Option<f32>,
    /// Number of messages when one was sent; the chat scrolls down once the list is longer.
    pub(crate) jump_after: Option<usize>,
    zoom_seen: f32,
    zoom_changed: Option<Instant>,
    zoom_saved: f32,
    /// Size of the chat alone, saved like `zoom`; `chat_rect` is where the chat was drawn last frame.
    pub(crate) chat_zoom: f32,
    pub(crate) chat_zoom_changed: Option<Instant>,
    pub(crate) chat_zoom_saved: f32,
    /// Widths the user dragged the messages to, and when they last changed (saved shortly after the user stops).
    pub(crate) bubble_w: crate::chat::Widths,
    pub(crate) bubble_changed: Option<Instant>,
    /// The seconds typed into the "stop a file request" field in Settings.
    pub(crate) offer_timeout_edit: String,
    pub(crate) chat_rect: Option<egui::Rect>,
    /// When each transfer entered its current state, to hide old finished ones.
    seen: HashMap<String, (TransferState, Instant)>,
}

impl App {
    pub fn new(hub: Arc<Hub>, keep: tray::Keep, ask_folder: bool) -> Self {
        let pc_name_edit = hub.bridge.pc_name();
        let zoom = hub.settings().zoom;
        let chat_h = hub.settings().chat_height;
        let chat_zoom = hub.settings().chat_zoom;
        let (bubble_own, bubble_theirs) = (hub.settings().bubble_own, hub.settings().bubble_theirs);
        let sheet = ask_folder.then(|| Sheet::ChooseFolder { path: hub.bridge.inbox_dir(), first_run: true });
        App {
            hub,
            min_watch: Default::default(),
            keep,
            sel: Sel::Auto,
            search: String::new(),
            phone_text: String::new(),
            pc_text: String::new(),
            pc_name_edit,
            sheet,
            reply_mid: None,
            chat_h,
            jump_after: None,
            zoom_seen: zoom,
            zoom_changed: None,
            zoom_saved: zoom,
            chat_zoom: chat_zoom.clamp(MIN_CHAT_ZOOM, MAX_CHAT_ZOOM),
            chat_zoom_changed: None,
            chat_zoom_saved: chat_zoom,
            bubble_w: crate::chat::Widths { own: bubble_own, theirs: bubble_theirs },
            bubble_changed: None,
            offer_timeout_edit: String::new(),
            chat_rect: None,
            seen: HashMap::new(),
        }
    }

    pub fn start_phone_pairing(&mut self) {
        let info = self.hub.bridge.new_pairing();
        let qr = qrcode::QrCode::with_error_correction_level(info.url.as_bytes(), qrcode::EcLevel::M).ok();
        let phones_before = self.hub.bridge.devices().len();
        self.sheet = Some(Sheet::PairPhone(PairView { info, started: Instant::now(), qr, phones_before }));
    }

    /// Visible transfers for a page: running ones, and finished ones for a short while.
    pub fn visible_transfers(&mut self, keep: impl Fn(&TransferInfo) -> bool) -> Vec<TransferInfo> {
        let now = Instant::now();
        let all = self.hub.bridge.transfers();
        for t in &all {
            match self.seen.get(&t.id) {
                Some((s, _)) if *s == t.state => {}
                _ => {
                    self.seen.insert(t.id.clone(), (t.state, now));
                }
            }
        }
        self.seen.retain(|id, _| all.iter().any(|t| &t.id == id));
        all.into_iter()
            .rev()
            .filter(|t| keep(t))
            .filter(|t| {
                let since = self.seen.get(&t.id).map_or(Duration::ZERO, |(_, at)| at.elapsed());
                match t.state {
                    TransferState::Done => since < SHOW_DONE_FOR,
                    TransferState::Interrupted | TransferState::Failed => since < SHOW_PROBLEMS_FOR,
                    _ => true,
                }
            })
            .take(8)
            .collect()
    }

    fn snapshot(&self) -> Data {
        let b = &self.hub.bridge;
        let mut phones = b.devices();
        phones.sort_by(|a, c| a.created.total_cmp(&c.created).then_with(|| a.name.cmp(&c.name)));
        let mut peers = b.peers();
        peers.sort_by(|a, c| a.created.total_cmp(&c.created).then_with(|| a.name.cmp(&c.name)));
        let peer_notes = peers.iter().map(|p| (p.id.clone(), b.peer_notes(&p.id, MAX_NOTES_SHOWN))).collect();
        let phone_notes = phones.iter().map(|p| (p.id.clone(), b.notes(&p.id, MAX_NOTES_SHOWN))).collect();
        Data { phones, peers, phone_notes, peer_notes, now: common::now_secs() }
    }

    /// Keep the selection valid: a phone or PC that was unpaired disappears, nothing chosen picks the first one.
    fn resolve_selection(&mut self, d: &Data) {
        let valid = match &self.sel {
            Sel::Phone(id) => d.phones.iter().any(|p| &p.id == id),
            Sel::Pc(id) => d.peers.iter().any(|p| &p.id == id),
            Sel::Settings => true,
            Sel::Auto => false,
        };
        if !valid {
            self.sel = d
                .phones
                .first()
                .map(|p| Sel::Phone(p.id.clone()))
                .or_else(|| d.peers.first().map(|p| Sel::Pc(p.id.clone())))
                .unwrap_or(Sel::Auto);
        }
    }

    fn conversation(&self, d: &Data) -> Option<Conv> {
        match &self.sel {
            Sel::Phone(id) => {
                let ph = d.phones.iter().find(|p| &p.id == id)?;
                let (notes, list) = split_views(d.phone_notes.get(&ph.id));
                Some(Conv { phone: true, id: ph.id.clone(), name: ph.name.clone(), notes, list })
            }
            Sel::Pc(id) => {
                let pc = d.peers.iter().find(|p| &p.id == id)?;
                let (notes, list) = split_views(d.peer_notes.get(id));
                Some(Conv { phone: false, id: pc.id.clone(), name: pc.name.clone(), notes, list })
            }
            _ => None,
        }
    }
}

impl App {
    /// Minimize to the Dock (macOS), and watch for the window coming back (see `minimize.rs`).
    fn minimize(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        self.min_watch.minimized_by_us(Instant::now());
        ctx.request_repaint_after(Duration::from_millis(300));
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Runs even while the window is minimized: notice when it has been restored from the Dock.
        if self.min_watch.waiting() {
            let (minimized, focused) = ctx.input(|i| (i.viewport().minimized, i.viewport().focused));
            if self.min_watch.restored(Instant::now(), minimized, focused) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.request_repaint();
            } else if self.min_watch.waiting() {
                ctx.request_repaint_after(Duration::from_millis(300));
            }
        }
        // The close button only hides the window when a tray icon can bring it back.
        if ctx.input(|i| i.viewport().close_requested()) {
            // A hidden window cannot be clicked, so a close request for it comes from the system
            // (logout, shutdown). Refusing it would keep the desktop from shutting down.
            let asked_by_system = !cfg!(target_os = "macos") && self.hub.hidden.load(Ordering::SeqCst);
            let hide = !self.hub.quitting.load(Ordering::SeqCst)
                && !asked_by_system
                && self.hub.tray_ok.load(Ordering::SeqCst)
                && self.hub.window_can_hide.load(Ordering::SeqCst)
                && self.hub.settings().keep_in_tray;
            if hide {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if !cfg!(target_os = "macos") {
                    self.hub.hidden.store(true, Ordering::SeqCst);
                }
                if cfg!(target_os = "macos") {
                    // A hidden window cannot be brought back by a click on the Dock icon (the system's
                    // "reopen" request never reaches the program), so it goes to the Dock like the yellow
                    // button does. The tray icon and the Dock icon both bring it back.
                    self.minimize(ctx);
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_secs(1)); // countdowns, "seen 5 s ago", fading messages

        let data = self.snapshot();
        self.resolve_selection(&data);
        *self.hub.open_phone.lock().unwrap_or_else(|e| e.into_inner()) = if let Sel::Phone(id) = &self.sel { Some(id.clone()) } else { None };
        self.handle_drops(&ctx);
        self.handle_pasted_images(&ctx);
        self.handle_mac_window_keys(&ctx);
        self.handle_zoom(&ctx);
        self.follow_background_changes(&data);

        let p = palette_of(&ctx);
        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(SIDEBAR_WIDTH)
            .frame(Frame::new().fill(p.sidebar).inner_margin(Margin::symmetric(12, 16)))
            .show(ui, |ui| self.sidebar(ui, &data));

        let conv = self.conversation(&data);
        egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(28, 20))).show(ui, |ui| match (&self.sel, &conv) {
            (Sel::Settings, _) => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    self.settings_page(ui);
                    ui.add_space(24.0);
                });
            }
            (_, Some(c)) => self.conversation_page(ui, c, &data),
            _ => self.welcome_page(ui),
        });

        self.show_sheet(&ctx);
        self.show_toasts(&ctx);
        self.drop_overlay(&ctx);
    }
}

// ---------------------------------------------------------------- sidebar

impl App {
    fn sidebar(&mut self, ui: &mut egui::Ui, d: &Data) {
        let p = palette(ui);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("Bridge").size(17.0).strong());
        });
        ui.add_space(2.0);

        let w = ui.available_width();
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search")
                .margin(Margin { left: 30, right: 8, top: 7, bottom: 7 })
                .desired_width(w),
        );
        icons::draw(ui.painter(), Icon::Search, search.rect.left_center() + Vec2::new(16.0, 0.0), 14.0, p.label2);

        ui.add_space(4.0);
        let (rect, pair) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::click());
        let pair = pair.on_hover_cursor(egui::CursorIcon::PointingHand);
        let fill = if pair.hovered() { p.tint.gamma_multiply(0.88) } else { p.tint };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(theme::FIELD_RADIUS), fill);
        let label = ui.painter().layout_no_wrap("Pair a device".to_owned(), FontId::proportional(14.0), p.on_tint);
        let group = 12.0 + 8.0 + label.size().x;
        let x0 = rect.center().x - group / 2.0;
        icons::draw(ui.painter(), Icon::Plus, egui::pos2(x0 + 6.0, rect.center().y), 12.0, p.on_tint);
        ui.painter().galley(egui::pos2(x0 + 20.0, rect.center().y - label.size().y / 2.0), label, p.on_tint);
        let mut start = None;
        egui::Popup::menu(&pair).align(egui::RectAlign::BOTTOM_START).width(w).show(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if menu_row(ui, Icon::Phone, "Pair a phone", "Scan a QR code").clicked() {
                start = Some(false);
            }
            if menu_row(ui, Icon::Pc, "Pair a PC", "Nearby, or by address").clicked() {
                start = Some(true);
            }
        });
        match start {
            Some(false) => self.start_phone_pairing(),
            Some(true) => self.sheet = Some(Sheet::PairPc),
            None => {}
        }
        ui.add_space(2.0);

        // Footer first (it is laid out from the bottom), the lists use the space above it.
        let list_height = (ui.available_height() - 76.0).max(60.0);
        let needle = self.search.trim().to_lowercase();
        let requests = self.hub.bridge.pair_requests();
        // devices that wait for an answer about a file
        let offers: Vec<String> = self
            .hub
            .bridge
            .transfers()
            .into_iter()
            .filter(|t| t.state == TransferState::Offered && matches!(t.dir, Direction::ToPc | Direction::FromPeer))
            .map(|t| t.device_id)
            .collect();
        egui::ScrollArea::vertical().id_salt("side_list").max_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            side_section(ui, "PHONE", 0);
            let mut shown = 0;
            for ph in &d.phones {
                let notes: Vec<&Note> = d.phone_notes.get(&ph.id).map(|v| v.iter().filter(|n| !n.chat_hidden).collect()).unwrap_or_default();
                let (time, mut preview) = preview_of(&notes, d.now, ph.last_seen);
                let wants = offers.contains(&ph.id);
                if wants {
                    preview = "Wants to send a file".into();
                }
                if !needle.is_empty() && !(ph.name.to_lowercase().contains(&needle) || preview.to_lowercase().contains(&needle)) {
                    continue;
                }
                shown += 1;
                let selected = self.sel == Sel::Phone(ph.id.clone());
                let row = device_row(ui, &p, Icon::Phone, &ph.name, &time, &preview, selected, false, wants);
                if row.clicked() {
                    self.sel = Sel::Phone(ph.id.clone());
                }
                if unpair_menu(&row, &p) {
                    self.sheet = Some(Sheet::UnpairPhone { id: ph.id.clone(), name: ph.name.clone() });
                }
            }
            if shown == 0 {
                quiet_line(ui, if d.phones.is_empty() { "No phone paired" } else { "No match" });
            }

            ui.add_space(6.0);
            side_section(ui, "OTHER PCS", requests.len());
            let mut shown = 0;
            for pc in &d.peers {
                let notes: Vec<&Note> = d.peer_notes.get(&pc.id).map(|v| v.iter().filter(|n| !n.chat_hidden).collect()).unwrap_or_default();
                let (time, mut preview) = preview_of(&notes, d.now, None);
                let wants = offers.contains(&pc.id);
                if wants {
                    preview = "Wants to send a file".into();
                }
                if !needle.is_empty() && !(pc.name.to_lowercase().contains(&needle) || preview.to_lowercase().contains(&needle)) {
                    continue;
                }
                shown += 1;
                let selected = self.sel == Sel::Pc(pc.id.clone());
                let row = device_row(ui, &p, Icon::Pc, &pc.name, &time, &preview, selected, false, wants);
                if unpair_menu(&row, &p) {
                    self.sheet = Some(Sheet::UnpairPc { id: pc.id.clone(), name: pc.name.clone() });
                }
                if row.clicked() {
                    self.sel = Sel::Pc(pc.id.clone());
                }
            }
            for r in &requests {
                device_row(ui, &p, Icon::Pc, &r.name, "", "Wants to pair", false, true, false);
            }
            if shown == 0 && requests.is_empty() {
                quiet_line(ui, if d.peers.is_empty() { "No PC paired" } else { "No match" });
            }
        });

        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                widgets::status(ui, true, "Running");
            });
            let selected = self.sel == Sel::Settings;
            if settings_item(ui, &p, selected).clicked() {
                self.sel = Sel::Settings;
            }
        });
    }

    /// Ctrl + mouse wheel zooms the whole window (ctrl + plus / minus is handled by egui itself).
    /// The scale is saved shortly after the user stops.
    fn handle_zoom(&mut self, ctx: &egui::Context) {
        // One wheel notch is a big step in egui; soften it to roughly 10 %.
        let delta = ctx.input(|i| i.zoom_delta()).powf(0.25);
        if (delta - 1.0).abs() > 1e-4 {
            // With the pointer on the chat, only the chat changes size; anywhere else the whole window does.
            let on_chat = self.chat_rect.zip(ctx.input(|i| i.pointer.hover_pos())).is_some_and(|(r, p)| r.contains(p));
            if on_chat {
                self.chat_zoom = (self.chat_zoom * delta).clamp(MIN_CHAT_ZOOM, MAX_CHAT_ZOOM);
                self.chat_zoom_changed = Some(Instant::now());
            } else {
                ctx.set_zoom_factor((ctx.zoom_factor() * delta).clamp(MIN_ZOOM, MAX_ZOOM));
            }
        }
        self.chat_rect = None; // set again by the chat while it is on the page
        if self.chat_zoom_changed.is_some_and(|t| t.elapsed() > Duration::from_millis(600)) && (self.chat_zoom - self.chat_zoom_saved).abs() > 1e-4 {
            let z = self.chat_zoom;
            self.hub.change_settings(|s| s.chat_zoom = z);
            self.chat_zoom_saved = z;
            self.chat_zoom_changed = None;
        }
        if self.chat_zoom_changed.is_some() {
            ctx.request_repaint_after(Duration::from_millis(700));
        }
        if self.bubble_changed.is_some_and(|t| t.elapsed() > Duration::from_millis(500)) {
            let w = self.bubble_w;
            self.hub.change_settings(|s| {
                s.bubble_own = w.own;
                s.bubble_theirs = w.theirs;
            });
            self.bubble_changed = None;
        }
        if self.bubble_changed.is_some() {
            ctx.request_repaint_after(Duration::from_millis(600));
        }
        let z = ctx.zoom_factor();
        if (z - self.zoom_seen).abs() > 1e-4 {
            self.zoom_seen = z;
            self.zoom_changed = Some(Instant::now());
        }
        if self.zoom_changed.is_some_and(|t| t.elapsed() > Duration::from_millis(600)) && (z - self.zoom_saved).abs() > 1e-4 {
            self.hub.change_settings(|s| s.zoom = z);
            self.zoom_saved = z;
            self.zoom_changed = None;
        }
        if self.zoom_changed.is_some() {
            ctx.request_repaint_after(Duration::from_millis(700));
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if dropped.is_empty() {
            return;
        }
        match self.sel.clone() {
            Sel::Pc(id) => self.hub.send_peer_files(&id, dropped),
            Sel::Phone(id) => self.hub.send_files(&id, dropped),
            _ => self.hub.toast("Open the phone or a PC first, then drop the files.", true),
        }
    }

    /// Ctrl+V / Cmd+V with a picture on the clipboard (a screenshot): save it as a file and send it to the
    /// open conversation, the same as a dropped file.
    fn handle_pasted_images(&mut self, ctx: &egui::Context) {
        let images: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| if let egui::Event::PasteImage(img) = e { Some(img.clone()) } else { None })
                .collect()
        });
        for img in images {
            let target = self.sel.clone();
            if !matches!(target, Sel::Pc(_) | Sel::Phone(_)) {
                self.hub.toast("Open the phone or a PC first, then paste the picture.", true);
                continue;
            }
            match self.hub.save_pasted_image(&img) {
                Ok(path) => match target {
                    Sel::Pc(id) => self.hub.send_peer_files(&id, vec![path]),
                    Sel::Phone(id) => self.hub.send_files(&id, vec![path]),
                    _ => {}
                },
                Err(e) => self.hub.toast(format!("Could not use the pasted picture: {e}"), true),
            }
        }
    }

    /// The Mac shortcuts for the window that the window system does not provide by itself here.
    fn handle_mac_window_keys(&mut self, ctx: &egui::Context) {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (minimize, close) = ctx.input_mut(|i| {
            (i.consume_key(egui::Modifiers::COMMAND, egui::Key::M), i.consume_key(egui::Modifiers::COMMAND, egui::Key::W))
        });
        if minimize {
            self.minimize(ctx);
        }
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let target = match &self.sel {
            Sel::Pc(id) => {
                let name = self.hub.bridge.peers().into_iter().find(|p| &p.id == id).map(|p| p.name).unwrap_or_default();
                format!("Drop files to send them to {name}")
            }
            Sel::Phone(_) => "Drop files to send them to the phone".to_string(),
            _ => "Open the phone or a PC to send files".to_string(),
        };
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, theme::id("drop_overlay")));
        let rect = ctx.content_rect();
        painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
        painter.text(rect.center(), Align2::CENTER_CENTER, target, FontId::proportional(22.0), Color32::WHITE);
    }

    /// Things that happen without a click: a phone finished pairing, the other PC answered, a PC vanished.
    fn follow_background_changes(&mut self, d: &Data) {
        let bridge = &self.hub.bridge;

        if let Some(r) = self.hub.take_pair_result() {
            match (r, matches!(self.sheet, Some(Sheet::Contacting(_)))) {
                (Ok(o), true) => self.sheet = Some(Sheet::Outgoing { id: o.id, confirmed: false }),
                (Ok(o), false) => bridge.pair_cancel(&o.id), // the user cancelled while we were connecting
                (Err(e), true) => self.sheet = Some(Sheet::Message { title: "Could not pair".into(), text: e }),
                (Err(_), false) => {}
            }
        }

        if let Some(Sheet::PairPhone(v)) = &self.sheet {
            if d.phones.len() > v.phones_before {
                if let Some(newest) = d.phones.iter().max_by(|a, b| a.created.total_cmp(&b.created)) {
                    self.sel = Sel::Phone(newest.id.clone());
                }
                self.sheet = None;
                self.hub.toast("Phone paired", false);
            }
        }

        if let Some(Sheet::Outgoing { id, .. }) = &self.sheet {
            match bridge.outgoing_pairs().into_iter().find(|o| &o.id == id) {
                Some(o) if o.state == OutPairState::Done => {
                    if let Some(pc) = d.peers.iter().find(|p| p.name == o.peer_name) {
                        self.sel = Sel::Pc(pc.id.clone());
                    }
                    self.sheet = None;
                    self.hub.toast(format!("Paired with {}", o.peer_name), false);
                }
                None => self.sheet = None,
                _ => {}
            }
        }

    }

    fn show_sheet(&mut self, ctx: &egui::Context) {
        // A request from another PC always comes first: it needs an answer and it is time limited.
        if let Some(req) = self.hub.bridge.pair_requests().into_iter().next() {
            let (mut accept, mut decline) = (false, false);
            let r = widgets::sheet(ctx, "incoming_pair", &format!("\u{201C}{}\u{201D} wants to pair", req.name), |ui| {
                widgets::footnote(ui, "Another PC on this network asks to exchange messages and files with this PC. Check that it shows the same code as below.");
                ui.add_space(6.0);
                code_box(ui, &req.code);
                widgets::footnote(ui, "If the codes differ, choose Decline. Anyone you accept can send you messages and files, but cannot see your phone, your history or anything else.");
                widgets::sheet_buttons(ui, |ui| {
                    accept = widgets::primary(ui, "Codes match").clicked();
                    decline = widgets::secondary(ui, "Decline").clicked();
                });
            });
            if accept {
                self.hub.bridge.pair_accept(&req.id);
            } else if decline || matches!(r, SheetResult::Dismissed) {
                self.hub.bridge.pair_decline(&req.id);
            }
            return;
        }

        // This PC's user confirmed the code; the other PC's user has not yet.
        if let Some(req) = self.hub.bridge.pair_waiting().into_iter().next() {
            let mut cancel = false;
            let r = widgets::sheet(ctx, "waiting_pair", &format!("Pair with \u{201C}{}\u{201D}", req.name), |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Waiting for {} to confirm the code.", req.name));
                });
                widgets::sheet_buttons(ui, |ui| cancel = widgets::secondary(ui, "Cancel").clicked());
            });
            if cancel || matches!(r, SheetResult::Dismissed) {
                self.hub.bridge.pair_decline(&req.id);
            }
            return;
        }

        let Some(sheet) = self.sheet.take() else { return };
        self.sheet = match sheet {
            Sheet::PairPhone(v) => self.sheet_pair_phone(ctx, v),
            Sheet::PairPc => self.sheet_pair_pc(ctx),
            Sheet::PairByAddress { host, port, error } => self.sheet_pair_address(ctx, host, port, error),
            Sheet::Contacting(name) => {
                let mut cancel = false;
                let r = widgets::sheet(ctx, "contacting", "Pair a PC", |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Contacting {name}..."));
                    });
                    widgets::sheet_buttons(ui, |ui| cancel = widgets::secondary(ui, "Cancel").clicked());
                });
                if cancel || matches!(r, SheetResult::Dismissed) { None } else { Some(Sheet::Contacting(name)) }
            }
            Sheet::Outgoing { id, confirmed } => self.sheet_outgoing(ctx, id, confirmed),
            Sheet::Message { title, text } => {
                let mut close = false;
                let r = widgets::sheet(ctx, "message", &title, |ui| {
                    ui.add(egui::Label::new(&text).wrap());
                    widgets::sheet_buttons(ui, |ui| close = widgets::primary(ui, "OK").clicked());
                });
                if close || matches!(r, SheetResult::Dismissed) { None } else { Some(Sheet::Message { title, text }) }
            }
            Sheet::ChooseFolder { path, first_run } => self.sheet_folder(ctx, path, first_run),
            Sheet::Hotkey { captured, down, holding } => self.sheet_hotkey(ctx, captured, down, holding),
            Sheet::RenamePhone { id, mut text } => {
                let (mut save, mut cancel) = (false, false);
                let r = widgets::sheet(ctx, "rename_phone", "Rename phone", |ui| {
                    let e = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY).char_limit(40));
                    if !e.has_focus() && !e.lost_focus() {
                        e.request_focus();
                    }
                    let enter = e.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    widgets::sheet_buttons(ui, |ui| {
                        save = widgets::primary_enabled(ui, !text.trim().is_empty(), "Save").clicked() || (enter && !text.trim().is_empty());
                        cancel = widgets::secondary(ui, "Cancel").clicked();
                    });
                });
                if save {
                    self.hub.bridge.rename_device(&id, &text);
                    None
                } else if cancel || matches!(r, SheetResult::Dismissed) {
                    None
                } else {
                    Some(Sheet::RenamePhone { id, text })
                }
            }
            Sheet::UnpairPhone { id, name } => {
                let (mut yes, mut cancel) = (false, false);
                let r = widgets::sheet(ctx, "unpair_phone", &format!("Unpair {name}?"), |ui| {
                    widgets::footnote(ui, "This PC forgets the phone. To use it again it has to be paired with a new QR code.");
                    widgets::sheet_buttons(ui, |ui| {
                        yes = widgets::destructive(ui, "Unpair").clicked();
                        cancel = widgets::secondary(ui, "Cancel").clicked();
                    });
                });
                if yes {
                    self.hub.bridge.unpair(&id);
                    self.hub.toast("Phone unpaired", false);
                    None
                } else if cancel || matches!(r, SheetResult::Dismissed) {
                    None
                } else {
                    Some(Sheet::UnpairPhone { id, name })
                }
            }
            Sheet::UnpairPc { id, name } => {
                let (mut yes, mut cancel) = (false, false);
                let r = widgets::sheet(ctx, "unpair_pc", &format!("Unpair {name}?"), |ui| {
                    widgets::footnote(ui, "Both PCs forget each other and the messages between them are removed on both sides. To exchange anything again, pair them again.");
                    widgets::sheet_buttons(ui, |ui| {
                        yes = widgets::destructive(ui, "Unpair").clicked();
                        cancel = widgets::secondary(ui, "Cancel").clicked();
                    });
                });
                if yes {
                    self.hub.bridge.unpair_peer(&id);
                    self.sel = Sel::Auto;
                    self.hub.toast("PC unpaired", false);
                    None
                } else if cancel || matches!(r, SheetResult::Dismissed) {
                    None
                } else {
                    Some(Sheet::UnpairPc { id, name })
                }
            }
        };
    }

    /// The hotkey chooser. It shows the keys being pressed as they are pressed; the combination stays after
    /// they are released, until the next one starts.
    fn sheet_hotkey(&mut self, ctx: &egui::Context, mut captured: Vec<String>, mut down: Vec<String>, mut holding: bool) -> Option<Sheet> {
        let now = ctx.input(|i| Shortcut::names_down(&i.keys_down, &i.modifiers));
        down.retain(|k| now.contains(k));
        for k in &now {
            if !down.contains(k) {
                down.push(k.clone());
            }
        }
        if down.is_empty() {
            holding = false;
        } else {
            if !holding {
                holding = true;
                captured.clear();
            }
            for k in &down {
                if !captured.contains(k) {
                    captured.push(k.clone());
                }
            }
        }
        // typing must not reach the rest of the window (and Escape must not close the sheet by accident)
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))));

        let shown = Shortcut { keys: captured.clone() };
        let valid = shown.is_valid();
        let (mut accept, mut cancel, mut reset) = (false, false, false);
        widgets::sheet(ctx, "hotkey", "Send clipboard to phone hotkey", |ui| {
            let p = palette(ui);
            ui.add_space(2.0);
            ui.label("Press and hold the keys you want to use together.");
            ui.add_space(6.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, egui::CornerRadius::same(theme::CARD_RADIUS), p.field);
            let text = if captured.is_empty() { "Press keys...".to_string() } else { crate::shortcut::text_of(&captured) };
            let color = if captured.is_empty() { p.label2 } else { p.label };
            let galley = ui.painter().layout(text, egui::FontId::proportional(24.0), color, rect.width() - 24.0);
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                reset = widgets::secondary(ui, "Reset").on_hover_text("Back to Ctrl+Alt+B").clicked();
                if !valid && !captured.is_empty() {
                    ui.label(RichText::new("Use at least two keys, for example Ctrl+B.").small().color(p.red));
                }
            });
            widgets::sheet_buttons(ui, |ui| {
                accept = widgets::primary_enabled(ui, valid, "Accept").clicked();
                cancel = widgets::secondary(ui, "Cancel").clicked();
            });
        });
        if reset {
            return Some(Sheet::Hotkey { captured: Shortcut::default().keys, down, holding: false });
        }
        if accept {
            self.hub.change_settings(|s| s.hotkey = shown);
        }
        if accept || cancel {
            self.keep.apply_hotkey(&self.hub);
            return None;
        }
        Some(Sheet::Hotkey { captured, down, holding })
    }

    fn sheet_folder(&mut self, ctx: &egui::Context, mut path: std::path::PathBuf, first_run: bool) -> Option<Sheet> {
        if let Some(dir) = self.hub.take_folder_pick() {
            path = dir;
        }
        let (mut browse, mut done, mut cancel) = (false, false, false);
        let r = widgets::sheet(ctx, "choose_folder", "Folder for received files", |ui| {
            let p = palette(ui);
            ui.add_space(2.0);
            ui.label("Choose where files from your phone and other PCs are saved.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                egui::Frame::new().fill(p.field).corner_radius(egui::CornerRadius::same(theme::FIELD_RADIUS)).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
                    ui.set_width(ui.available_width() - 150.0);
                    ui.add(egui::Label::new(RichText::new(path.display().to_string()).monospace().size(12.5)).truncate());
                });
                browse = widgets::secondary(ui, "Choose folder...").clicked();
            });
            widgets::footnote(ui, "The folder is created when the first file arrives. You can change it later in Settings.");
            widgets::sheet_buttons(ui, |ui| {
                done = widgets::primary(ui, if first_run { "Continue" } else { "Use this folder" }).clicked();
                if !first_run {
                    cancel = widgets::secondary(ui, "Cancel").clicked();
                }
            });
        });
        if browse {
            self.hub.pick_folder(path.clone());
        }
        // Escape on the first start keeps the suggested folder; in Settings it cancels.
        let dismissed = matches!(r, SheetResult::Dismissed);
        if done || (first_run && dismissed) {
            self.hub.set_inbox(path);
            return None;
        }
        if cancel || dismissed { None } else { Some(Sheet::ChooseFolder { path, first_run }) }
    }

    fn sheet_pair_phone(&mut self, ctx: &egui::Context, v: PairView) -> Option<Sheet> {
        let left = v.info.expires_in.saturating_sub(v.started.elapsed());
        let (mut renew, mut close) = (false, false);
        let r = widgets::sheet(ctx, "pair_phone", "Pair a phone", |ui| {
            if left.is_zero() {
                widgets::footnote(ui, "This code has expired.");
            } else {
                ui.vertical_centered(|ui| {
                    if let Some(qr) = &v.qr {
                        common::draw_qr(ui, qr, 230.0);
                    }
                });
                widgets::footnote(ui, "In the Bridge app on the phone, scan this QR code. It carries this PC's certificate fingerprint, so the phone only ever talks to this PC.");
                ui.horizontal(|ui| {
                    ui.label("Code");
                    ui.label(RichText::new(&v.info.code).monospace().strong());
                    ui.label(RichText::new(format!("expires in {}:{:02}", left.as_secs() / 60, left.as_secs() % 60)).small().color(palette(ui).label2));
                });
            }
            widgets::sheet_buttons(ui, |ui| {
                if left.is_zero() {
                    renew = widgets::primary(ui, "New code").clicked();
                } else {
                    close = widgets::primary(ui, "Done").clicked();
                    renew = widgets::secondary(ui, "New code").clicked();
                }
            });
        });
        if renew {
            self.start_phone_pairing();
            self.sheet.take()
        } else if close || matches!(r, SheetResult::Dismissed) {
            None
        } else {
            Some(Sheet::PairPhone(v))
        }
    }

    fn sheet_pair_pc(&mut self, ctx: &egui::Context) -> Option<Sheet> {
        let nearby: Vec<_> = self.hub.bridge.nearby().into_iter().filter(|n| !n.paired).collect();
        let (fill, card) = (palette_of(ctx).field, palette_of(ctx).card);
        let (mut cancel, mut address) = (false, false);
        let mut pick = None;
        let r = widgets::sheet(ctx, "pair_pc", "Pair a PC", |ui| {
            widgets::footnote(ui, "PCs on this network that run Bridge. They only see this PC's name until you pair.");
            widgets::section(ui, "NEARBY");
            widgets::grouped_with(ui, fill, |g| {
                if nearby.is_empty() {
                    g.row(|ui| {
                        widgets::footnote(
                            ui,
                            if self.hub.bridge.is_discoverable() {
                                "No other PC found. Is Bridge running there, on the same network?"
                            } else {
                                "Searching works only while this PC is visible. Turn on Visible to other PCs in Settings."
                            },
                        )
                    });
                }
                for n in &nearby {
                    g.row(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(&n.name);
                            widgets::trailing(ui, |ui| {
                                if widgets::secondary_on(ui, card, "Pair").clicked() {
                                    pick = Some(n.clone());
                                }
                            });
                        });
                    });
                }
            });
            widgets::sheet_buttons(ui, |ui| {
                cancel = widgets::secondary(ui, "Cancel").clicked();
                address = widgets::plain(ui, "Pair by address...").clicked();
            });
        });
        if let Some(n) = pick {
            let ip = n.addrs.iter().find(|a| a.is_ipv4()).or(n.addrs.first()).copied();
            if let Some(ip) = ip {
                self.hub.begin_pair(ip, n.port);
                return Some(Sheet::Contacting(n.name));
            }
        }
        if address {
            return Some(Sheet::PairByAddress { host: String::new(), port: DEFAULT_PEER_PORT.to_string(), error: None });
        }
        if cancel || matches!(r, SheetResult::Dismissed) { None } else { Some(Sheet::PairPc) }
    }

    fn sheet_pair_address(&mut self, ctx: &egui::Context, mut host: String, mut port: String, mut error: Option<String>) -> Option<Sheet> {
        let (mut go, mut cancel) = (false, false);
        let r = widgets::sheet(ctx, "pair_address", "Pair by address", |ui| {
            widgets::footnote(ui, "Use this when the other PC does not show up under Nearby. Enter the address shown in its Settings.");
            ui.horizontal(|ui| {
                ui.label("Address");
                ui.add(egui::TextEdit::singleline(&mut host).hint_text("192.168.1.20").desired_width(170.0));
                ui.label("Port");
                ui.add(egui::TextEdit::singleline(&mut port).desired_width(60.0));
            });
            if let Some(e) = &error {
                ui.label(RichText::new(e).small().color(palette(ui).red));
            }
            widgets::sheet_buttons(ui, |ui| {
                go = widgets::primary_enabled(ui, !host.trim().is_empty(), "Pair").clicked();
                cancel = widgets::secondary(ui, "Cancel").clicked();
            });
        });
        if go {
            match (host.trim().parse::<std::net::IpAddr>(), port.trim().parse::<u16>()) {
                (Ok(ip), Ok(p)) if p != 0 => {
                    self.hub.begin_pair(ip, p);
                    return Some(Sheet::Contacting(ip.to_string()));
                }
                (Err(_), _) => error = Some("Enter an IP address such as 192.168.1.20.".into()),
                _ => error = Some("Enter a port number, normally 8444.".into()),
            }
        }
        if cancel || matches!(r, SheetResult::Dismissed) { None } else { Some(Sheet::PairByAddress { host, port, error }) }
    }

    fn sheet_outgoing(&mut self, ctx: &egui::Context, id: String, mut confirmed: bool) -> Option<Sheet> {
        let o = self.hub.bridge.outgoing_pairs().into_iter().find(|o| o.id == id)?;
        let (mut confirm, mut cancel, mut close) = (false, false, false);
        let name = o.peer_name.clone();
        let r = widgets::sheet(ctx, "outgoing_pair", &format!("Pair with \u{201C}{name}\u{201D}"), |ui| {
            let p = palette(ui);
            match o.state {
                OutPairState::Waiting | OutPairState::PeerAccepted => {
                    let accepted = o.state == OutPairState::PeerAccepted;
                    let line = match (accepted, confirmed) {
                        (false, false) => format!("Waiting for {name} to accept. It should show the code below."),
                        (false, true) => format!("You confirmed. Waiting for {name} to accept."),
                        (true, false) => format!("{name} accepted. Do both screens show this same code?"),
                        (true, true) => "Finishing...".to_string(),
                    };
                    widgets::footnote(ui, &line);
                    ui.add_space(6.0);
                    code_box(ui, &o.code);
                    widgets::footnote(ui, "If the codes differ, someone else on the network may be in the way. Choose Cancel.");
                    widgets::sheet_buttons(ui, |ui| {
                        confirm = widgets::primary_enabled(ui, !confirmed, "Codes match").clicked();
                        cancel = widgets::secondary(ui, "Cancel").clicked();
                    });
                }
                OutPairState::Declined => {
                    ui.label(format!("{name} declined."));
                    widgets::sheet_buttons(ui, |ui| close = widgets::primary(ui, "OK").clicked());
                }
                OutPairState::Failed | OutPairState::Done => {
                    ui.label(RichText::new(o.error.clone().unwrap_or_else(|| "Pairing did not finish.".into())).color(p.red));
                    widgets::sheet_buttons(ui, |ui| close = widgets::primary(ui, "OK").clicked());
                }
            }
        });
        if confirm {
            self.hub.bridge.pair_confirm(&id);
            confirmed = true;
        }
        if cancel || matches!(r, SheetResult::Dismissed) {
            self.hub.bridge.pair_cancel(&id);
            None
        } else if close {
            self.hub.bridge.pair_cancel(&id); // removes the finished record
            None
        } else {
            Some(Sheet::Outgoing { id, confirmed })
        }
    }

    fn show_toasts(&self, ctx: &egui::Context) {
        let toasts = self.hub.live_toasts();
        if toasts.is_empty() {
            return;
        }
        let p = palette_of(ctx);
        egui::Area::new(theme::id("toasts")).anchor(Align2::RIGHT_BOTTOM, [-16.0, -16.0]).order(Order::Tooltip).show(ctx, |ui| {
            for t in toasts.iter().rev().take(4) {
                Frame::new()
                    .fill(p.card)
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(14, 10))
                    .stroke(egui::Stroke::new(1.0, p.separator))
                    .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: egui::Color32::from_black_alpha(60) })
                    .show(ui, |ui| {
                        ui.set_min_width(240.0);
                        ui.set_max_width(340.0);
                        let text = RichText::new(&t.text);
                        ui.label(if t.is_error { text.color(p.red) } else { text });
                    });
            }
        });
    }
}

/// The six digits both people compare. Large, spaced, readable from a distance.
fn code_box(ui: &mut egui::Ui, code: &str) {
    let p = palette(ui);
    Frame::new().fill(p.field).corner_radius(egui::CornerRadius::same(theme::FIELD_RADIUS)).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| ui.label(RichText::new(code).size(32.0).monospace().strong()));
    });
}

/// Section caption in the sidebar, with an optional count badge.
fn side_section(ui: &mut egui::Ui, text: &str, badge: usize) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new(text).small().strong().color(p.label2));
        if badge > 0 {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(18.0, 18.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 9.0, p.tint);
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, badge.to_string(), FontId::proportional(12.0), p.on_tint);
            });
        }
    });
}

fn quiet_line(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        widgets::footnote(ui, text);
    });
}

/// Last activity of a conversation: how long ago, and a one-line preview.
fn preview_of(notes: &[&Note], now: f64, fallback_seen: Option<f64>) -> (String, String) {
    match notes.last() {
        Some(n) => {
            let text = match (&n.kind, &n.text, &n.file) {
                (NoteKind::File, _, Some(f)) => format!("{} · {}", f.name, common::size(f.size)),
                (_, Some(t), _) => t.split_whitespace().collect::<Vec<_>>().join(" "),
                _ => String::new(),
            };
            (short_ago(now - n.ts), text)
        }
        None => (fallback_seen.map(|t| short_ago(now - t)).unwrap_or_default(), "No messages yet".to_string()),
    }
}

/// One device in the sidebar: round icon, name, time of last activity and a preview line.
/// Selected: tint fill with white text. `pending` draws an outlined icon (a request, not yet paired).
#[allow(clippy::too_many_arguments)]
/// Right-click menu of a device in the sidebar. True when "Unpair" was chosen.
fn unpair_menu(row: &egui::Response, p: &Palette) -> bool {
    let mut chosen = false;
    row.context_menu(|ui| {
        ui.set_min_width(110.0);
        let button = egui::Button::new(RichText::new("Unpair").color(p.red)).frame(false);
        if ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], button).clicked() {
            chosen = true;
            ui.close();
        }
    });
    chosen
}

fn device_row(ui: &mut egui::Ui, p: &Palette, icon: Icon, name: &str, time: &str, preview: &str, selected: bool, pending: bool, alert: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let radius = egui::CornerRadius::same(theme::FIELD_RADIUS);
    if selected {
        ui.painter().rect_filled(rect, radius, p.tint);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.field_hover);
    }
    let (fg, fg2) = if selected { (p.on_tint, p.on_tint) } else { (p.label, p.label2) };
    let painter = ui.painter_at(rect);
    let c = egui::pos2(rect.min.x + 8.0 + 16.0, rect.center().y);
    painter.circle_filled(c, 16.0, if selected { Color32::from_white_alpha(56) } else { p.field });
    if pending {
        painter.circle_stroke(c, 15.0, egui::Stroke::new(1.5, p.label2));
    }
    icons::draw(&painter, icon, c, 16.0, fg);
    let left = rect.min.x + 8.0 + 32.0 + 8.0;
    let right = rect.max.x - 8.0;
    let time_w = if time.is_empty() {
        0.0
    } else {
        let g = painter.layout_no_wrap(time.to_owned(), FontId::proportional(12.0), fg2);
        let w = g.size().x;
        painter.galley(egui::pos2(right - w, rect.min.y + 9.0), g, fg2);
        w + 6.0
    };
    painter.galley(egui::pos2(left, rect.min.y + 7.0), widgets::one_line(&painter, name, 14.0, fg, right - left - time_w), fg);
    let sub_color = if (pending || alert) && !selected { p.tint_text } else { fg2 };
    painter.galley(egui::pos2(left, rect.min.y + 26.0), widgets::one_line(&painter, preview, 12.0, sub_color, right - left), sub_color);
    resp
}

fn settings_item(ui: &mut egui::Ui, p: &Palette, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let radius = egui::CornerRadius::same(theme::FIELD_RADIUS);
    if selected {
        ui.painter().rect_filled(rect, radius, p.tint);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.field_hover);
    }
    let color = if selected { p.on_tint } else { p.label };
    icons::draw(ui.painter(), Icon::Gear, rect.left_center() + Vec2::new(18.0, 0.0), 16.0, color);
    ui.painter().text(rect.left_center() + Vec2::new(36.0, 0.0), Align2::LEFT_CENTER, "Settings", FontId::proportional(14.0), color);
    resp
}

/// A row in the "Pair a device" menu.
fn menu_row(ui: &mut egui::Ui, icon: Icon, title: &str, sub: &str) -> egui::Response {
    let p = palette(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(theme::FIELD_RADIUS), p.field_hover);
    }
    let c = egui::pos2(rect.min.x + 18.0, rect.center().y);
    ui.painter().circle_filled(c, 16.0, p.field);
    icons::draw(ui.painter(), icon, c, 16.0, p.label);
    ui.painter().text(egui::pos2(rect.min.x + 44.0, rect.min.y + 15.0), Align2::LEFT_CENTER, title, FontId::proportional(14.0), p.label);
    ui.painter().text(egui::pos2(rect.min.x + 44.0, rect.min.y + 33.0), Align2::LEFT_CENTER, sub, FontId::proportional(12.0), p.label2);
    resp
}

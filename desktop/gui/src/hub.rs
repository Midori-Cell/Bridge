//! Everything the window, the tray icon and the hotkey share: the running bridge, settings,
//! clipboard access and the little messages shown in the corner of the window.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use bridge_core::{Bridge, Direction, Event, FileRef, NoteError, NoteKind, OutgoingPair, PeerError, TransferState};
use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "gui-settings.json";
const TOAST_LIFETIME: Duration = Duration::from_secs(6);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Closing the window keeps the program running in the tray.
    pub keep_in_tray: bool,
    /// Text arriving from the phone is also put on this PC's clipboard.
    pub copy_incoming_text: bool,
    /// Interface scale (ctrl + mouse wheel, ctrl + plus / minus).
    pub zoom: f32,
    /// Size of the chat alone (ctrl + mouse wheel over the chat); `zoom` scales the whole window.
    pub chat_zoom: f32,
    /// Width of all my messages / of all incoming messages in the chat, set by dragging one of them sideways
    /// (points at normal size). `None`: as wide as the text.
    pub bubble_own: Option<f32>,
    pub bubble_theirs: Option<f32>,
    /// Times in the chat as 21:14 (true) or 9:14 PM (false).
    pub time_24h: bool,
    /// Which key sends the message in the text box.
    pub send_key: SendKey,
    /// Height of the chat in points, set by dragging its lower edge. `None`: follows the window.
    pub chat_height: Option<f32>,
    /// Where received files are saved. `None` until the user was asked on first start.
    pub inbox_dir: Option<PathBuf>,
    /// The global hotkey sends the clipboard to the phone.
    pub hotkey_enabled: bool,
    /// The keys of that hotkey.
    pub hotkey: crate::shortcut::Shortcut,
    /// The folder was confirmed in the window that asks for it (once, at the first start).
    pub inbox_confirmed: bool,
    /// The user's own highlight color, used when `accent` is `Custom`.
    pub custom_accent: [u8; 3],
    /// The highlight color of the whole program.
    pub accent: crate::theme::Accent,
}

/// The key (combination) that sends a message. The other combinations insert a new line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendKey {
    #[default]
    CtrlEnter,
    ShiftEnter,
    AltEnter,
    /// Enter sends, Shift+Enter inserts a new line.
    Enter,
}

impl SendKey {
    pub const ALL: [SendKey; 4] = [SendKey::Enter, SendKey::CtrlEnter, SendKey::ShiftEnter, SendKey::AltEnter];

    pub fn label(self) -> &'static str {
        match self {
            SendKey::Enter => "Enter",
            SendKey::CtrlEnter => if cfg!(target_os = "macos") { "Cmd+Enter" } else { "Ctrl+Enter" },
            SendKey::ShiftEnter => "Shift+Enter",
            SendKey::AltEnter => if cfg!(target_os = "macos") { "Option+Enter" } else { "Alt+Enter" },
        }
    }

    /// Does this combination of modifiers (with Enter) send?
    pub fn matches(self, m: &egui::Modifiers) -> bool {
        match self {
            SendKey::Enter => !m.command && !m.shift && !m.alt,
            SendKey::CtrlEnter => m.command && !m.shift && !m.alt,
            SendKey::ShiftEnter => m.shift && !m.command && !m.alt,
            SendKey::AltEnter => m.alt && !m.command && !m.shift,
        }
    }
}

impl Settings {
    /// Read the settings file in `data_dir`; defaults when there is none.
    pub fn load(data_dir: &Path) -> Settings {
        std::fs::read(data_dir.join(SETTINGS_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
}

impl Default for Settings {
    fn default() -> Self {
        // A tray icon is dependable on Windows and macOS. On Linux it depends on the desktop
        // (GNOME shows none without an extension), so closing quits there until the user opts in.
        Self { keep_in_tray: !cfg!(target_os = "linux"), copy_incoming_text: false, zoom: 1.0, chat_zoom: 1.0, bubble_own: None, bubble_theirs: None, time_24h: true, send_key: SendKey::default(), chat_height: None, inbox_dir: None, hotkey_enabled: true, hotkey: Default::default(), inbox_confirmed: false, custom_accent: [0x2E, 0xA0, 0x6B], accent: crate::theme::Accent::default() }
    }
}

#[derive(Clone)]
pub struct Toast {
    pub text: String,
    pub is_error: bool,
    pub at: Instant,
}

pub struct Hub {
    pub bridge: Arc<Bridge>,
    rt: tokio::runtime::Handle,
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    clipboard: Mutex<Option<arboard::Clipboard>>,
    toasts: Mutex<Vec<Toast>>,
    ctx: OnceLock<egui::Context>,
    /// A tray icon exists, so hiding the window cannot strand the user.
    pub tray_ok: AtomicBool,
    /// The hotkey was registered; otherwise `hotkey_note` says why not.
    pub hotkey_note: Mutex<String>,
    pub quitting: AtomicBool,
    /// The phone whose chat is open in the window, if any (the tray and the hotkey send the clipboard to it).
    pub open_phone: Mutex<Option<String>>,
    /// This window system can hide a window. False on a native Wayland session, where the program
    /// cannot take its window away, so closing has to quit instead.
    pub window_can_hide: AtomicBool,
    /// On a Wayland session the window was started through XWayland, because that can hide it.
    pub x11_forced: AtomicBool,
    /// The close button hid the window (to the tray). A close request that arrives now cannot come
    /// from a click, but from the system (logging out, shutting down), and must not be refused.
    pub hidden: AtomicBool,
    /// Result of the last "start pairing with another PC" request, picked up by the window.
    pair_result: Mutex<Option<Result<OutgoingPair, String>>>,
    folder_pick: Mutex<Option<PathBuf>>,
    /// Id of the registered global hotkey (0: none), so the event handler knows which key is ours.
    pub hotkey_id: std::sync::atomic::AtomicU32,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Hub {
    pub fn new(bridge: Arc<Bridge>, rt: tokio::runtime::Handle) -> Arc<Hub> {
        let settings_path = bridge.data_dir().join(SETTINGS_FILE);
        let settings = Settings::load(bridge.data_dir());
        Arc::new(Hub {
            bridge,
            rt,
            settings: Mutex::new(settings),
            settings_path,
            clipboard: Mutex::new(None),
            toasts: Mutex::new(Vec::new()),
            ctx: OnceLock::new(),
            tray_ok: AtomicBool::new(false),
            hotkey_note: Mutex::new(String::new()),
            quitting: AtomicBool::new(false),
            open_phone: Mutex::new(None),
            window_can_hide: AtomicBool::new(true),
            x11_forced: AtomicBool::new(false),
            hidden: AtomicBool::new(false),
            pair_result: Mutex::new(None),
            folder_pick: Mutex::new(None),
            hotkey_id: std::sync::atomic::AtomicU32::new(0),
        })
    }

    pub fn set_ctx(&self, ctx: egui::Context) {
        let _ = self.ctx.set(ctx);
    }

    pub fn repaint(&self) {
        if let Some(c) = self.ctx.get() {
            c.request_repaint();
        }
    }

    // ------------------------------------------------------------ settings

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn change_settings(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = lock(&self.settings);
        f(&mut s);
        if let Ok(json) = serde_json::to_vec_pretty(&*s) {
            if let Err(e) = std::fs::write(&self.settings_path, json) {
                tracing::warn!("could not save settings: {e}");
            }
        }
    }

    // ------------------------------------------------------------ toasts

    pub fn toast(&self, text: impl Into<String>, is_error: bool) {
        lock(&self.toasts).push(Toast { text: text.into(), is_error, at: Instant::now() });
        self.repaint();
    }

    /// Messages that are still young enough to show; drops the rest.
    pub fn live_toasts(&self) -> Vec<Toast> {
        let mut t = lock(&self.toasts);
        t.retain(|x| x.at.elapsed() < TOAST_LIFETIME);
        t.clone()
    }

    // ------------------------------------------------------------ clipboard

    pub fn read_clipboard_text(&self) -> Result<String, String> {
        let mut cb = lock(&self.clipboard);
        if cb.is_none() {
            *cb = Some(arboard::Clipboard::new().map_err(|e| format!("Clipboard not available: {e}"))?);
        }
        let text = cb.as_mut().map(|c| c.get_text()).ok_or("Clipboard not available")?;
        text.map_err(|_| "The clipboard holds no text.".to_string())
    }

    pub fn write_clipboard_text(&self, text: &str) -> Result<(), String> {
        let mut cb = lock(&self.clipboard);
        if cb.is_none() {
            *cb = Some(arboard::Clipboard::new().map_err(|e| format!("Clipboard not available: {e}"))?);
        }
        // The Clipboard object stays alive in `self`; on Linux the data disappears when it is dropped.
        cb.as_mut().ok_or("Clipboard not available")?.set_text(text.to_string()).map_err(|e| e.to_string())
    }

    // ------------------------------------------------------------ sending

    /// `reply_to`: the id of the message this one answers. The note goes to that one phone only.
    pub fn send_text_reply(&self, phone: &str, text: &str, reply_to: Option<&str>) -> Result<(), String> {
        self.bridge.send_text_reply(phone, text, reply_to).map(|_| ()).map_err(|e| match e {
            NoteError::Empty => "Nothing to send.".to_string(),
            NoteError::TooLong => "That text is too long.".to_string(),
        })
    }

    /// Send whatever text is on the clipboard to one phone (the button in its chat).
    pub fn send_clipboard_to(&self, phone: &str) {
        let name = self.bridge.devices().into_iter().find(|d| d.id == phone).map(|d| d.name).unwrap_or_else(|| "the phone".into());
        match self.read_clipboard_text().and_then(|t| self.send_text_reply(phone, &t, None).map(|_| t)) {
            Ok(t) => self.toast(format!("Clipboard sent to {name} ({} characters)", t.chars().count()), false),
            Err(e) => self.toast(e, true),
        }
    }

    /// The tray menu and the hotkey: send the clipboard to the phone whose chat is open, otherwise to the
    /// phone that was in touch most recently. Always one phone, and the message says which.
    pub fn send_clipboard(&self) {
        let phones = self.bridge.devices();
        let open = lock(&self.open_phone).clone();
        let chosen = open
            .and_then(|id| phones.iter().find(|d| d.id == id))
            .or_else(|| phones.iter().max_by(|a, b| a.last_seen.unwrap_or(0.0).total_cmp(&b.last_seen.unwrap_or(0.0))));
        match chosen {
            Some(d) => self.send_clipboard_to(&d.id.clone()),
            None => self.toast("No phone is paired yet.", true),
        }
    }

    /// Offer files to one phone. Hashing big files takes a moment, so this runs in the background.
    pub fn send_files(self: &Arc<Self>, phone: &str, paths: Vec<PathBuf>) {
        for path in paths {
            let (me, phone) = (self.clone(), phone.to_string());
            self.rt.spawn(async move {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match me.bridge.send_file(&phone, &path).await {
                    Ok(f) => me.toast(format!("{} is ready on the phone", f.name), false),
                    Err(e) => me.toast(format!("Could not send {name}: {e}"), true),
                }
            });
        }
    }

    /// Save a pasted picture (a screenshot, say) as a PNG file in the program data folder. The file must stay,
    /// because the phone or the other PC fetches it from there when it accepts the transfer.
    pub fn save_pasted_image(&self, img: &egui::ColorImage) -> Result<PathBuf, String> {
        let dir = self.bridge.data_dir().join("pasted");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stamp = crate::common::utc_stamp(std::time::SystemTime::now());
        let mut path = dir.join(format!("Pasted image {stamp}.png"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("Pasted image {stamp} ({n}).png"));
            n += 1;
        }
        let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.size[0] as u32, img.size[1] as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().and_then(|mut w| w.write_image_data(&rgba)).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Show a file picker without blocking the window.
    pub fn pick_and_send_files(self: &Arc<Self>, phone: &str, phone_name: &str) {
        let (me, phone, title) = (self.clone(), phone.to_string(), format!("Send files to {phone_name}"));
        std::thread::spawn(move || {
            if let Some(paths) = rfd::FileDialog::new().set_title(title).pick_files() {
                me.send_files(&phone, paths);
            }
        });
    }

    // ------------------------------------------------------------ other PCs

    pub fn send_peer_text(self: &Arc<Self>, peer: &str, text: String, reply_to: Option<String>, on_ok: impl FnOnce() + Send + 'static) {
        let me = self.clone();
        let peer = peer.to_string();
        self.rt.spawn(async move {
            match me.bridge.send_peer_text_reply(&peer, &text, reply_to.as_deref()).await {
                Ok(_) => on_ok(),
                Err(e) => me.toast(e.to_string(), true),
            }
            me.repaint();
        });
    }

    pub fn send_peer_files(self: &Arc<Self>, peer: &str, paths: Vec<PathBuf>) {
        for path in paths {
            let me = self.clone();
            let peer = peer.to_string();
            self.rt.spawn(async move {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match me.bridge.send_peer_file(&peer, &path).await {
                    // the transfer card already says that the file was declined or cancelled
                    Ok(_) | Err(PeerError::Cancelled | PeerError::Declined | PeerError::Queued) => {}
                    Err(e) => me.toast(format!("Could not send {name}: {e}"), true),
                }
            });
        }
    }

    pub fn pick_and_send_peer_files(self: &Arc<Self>, peer: &str, pc_name: &str) {
        let (me, peer, title) = (self.clone(), peer.to_string(), format!("Send files to {pc_name}"));
        std::thread::spawn(move || {
            if let Some(paths) = rfd::FileDialog::new().set_title(title).pick_files() {
                me.send_peer_files(&peer, paths);
            }
        });
    }

    /// Ask another PC to pair. The answer arrives through `take_pair_result`.
    pub fn begin_pair(self: &Arc<Self>, host: std::net::IpAddr, port: u16) {
        let me = self.clone();
        self.rt.spawn(async move {
            let r = me.bridge.pair_start(host, port).await.map_err(|e| e.to_string());
            *lock(&me.pair_result) = Some(r);
            me.repaint();
        });
    }

    pub fn take_pair_result(&self) -> Option<Result<OutgoingPair, String>> {
        lock(&self.pair_result).take()
    }

    /// Use this folder for received files from now on and remember it. Files already received stay put.
    pub fn set_inbox(&self, dir: PathBuf) {
        self.bridge.set_inbox_dir(dir.clone());
        self.change_settings(|s| {
            s.inbox_dir = Some(dir);
            s.inbox_confirmed = true;
        });
    }

    /// Show the system folder picker (on its own thread); the answer is picked up with `take_folder_pick`.
    pub fn pick_folder(self: &Arc<Self>, start: PathBuf) {
        let me = self.clone();
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new().set_title("Folder for received files");
            if let Some(dir) = start.ancestors().find(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            if let Some(dir) = dialog.pick_folder() {
                *lock(&me.folder_pick) = Some(dir);
            }
            me.repaint();
        });
    }

    pub fn take_folder_pick(&self) -> Option<PathBuf> {
        lock(&self.folder_pick).take()
    }

    pub fn open_inbox(&self) {
        let dir = self.bridge.inbox_dir();
        let _ = std::fs::create_dir_all(&dir);
        open_folder(&dir);
    }

    // ------------------------------------------------------------ messages and files

    /// Delete a message for both sides. A received file is deleted from the inbox too.
    pub fn delete_message(self: &Arc<Self>, phone: bool, conv_id: &str, mid: &str) {
        if phone {
            self.bridge.delete_note(conv_id, mid);
            return;
        }
        let (me, peer, mid) = (self.clone(), conv_id.to_string(), mid.to_string());
        self.rt.spawn(async move {
            me.bridge.delete_peer_note(&peer, &mid).await;
            me.repaint();
        });
    }

    /// Open a file from the chat with the program the system chooses. Programs and scripts are not
    /// started from here: their folder opens instead, so nothing runs by accident.
    pub fn open_file(&self, f: &FileRef) {
        let Some(path) = self.existing_path(f) else { return };
        if is_runnable(&f.name) {
            self.toast("Programs are not started from here. Showing the folder instead.", false);
            reveal(&path);
        } else if let Err(e) = open::that_detached(&path) {
            self.toast(format!("Could not open {}: {e}", f.name), true);
        }
    }

    /// Show the folder a file is in, with the file selected where the system can do that.
    pub fn show_file(&self, f: &FileRef) {
        if let Some(path) = self.existing_path(f) {
            reveal(&path);
        }
    }

    fn existing_path(&self, f: &FileRef) -> Option<PathBuf> {
        let Some(path) = f.local_path.as_deref().map(PathBuf::from) else {
            self.toast("This PC does not know where that file is.", true);
            return None;
        };
        if !path.exists() {
            self.toast(format!("{} is no longer there. It was moved or deleted.", f.name), true);
            return None;
        }
        Some(path)
    }

    pub fn accept_file(self: &Arc<Self>, id: &str) {
        let (me, id) = (self.clone(), id.to_string());
        self.rt.spawn(async move {
            if !me.bridge.accept_file(&id).await {
                me.toast("That file is no longer being offered.", true);
            }
            me.repaint();
        });
    }

    pub fn decline_file(&self, id: &str) {
        self.bridge.decline_file(id);
    }

    pub fn cancel_transfer(&self, id: &str) {
        self.bridge.cancel_transfer(id);
    }

    /// Take a finished, declined or cancelled transfer off the list (on both PCs when it was with another PC).
    pub fn dismiss_transfer(self: &Arc<Self>, id: &str) {
        let (me, id) = (self.clone(), id.to_string());
        self.rt.spawn(async move {
            me.bridge.dismiss_transfer(&id).await;
            me.repaint();
        });
    }

    // ------------------------------------------------------------ window control

    pub fn show_window(&self) {
        self.hidden.store(false, Ordering::SeqCst);
        if let Some(c) = self.ctx.get() {
            c.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            c.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            c.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    /// Ask the window to close, as the close button does. Debug builds use it to test that path.
    #[cfg(debug_assertions)]
    pub fn request_close(&self) {
        if let Some(c) = self.ctx.get() {
            c.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Really quit (as opposed to the close button, which may only hide the window).
    pub fn quit(&self) {
        if let Some(c) = self.ctx.get() {
            c.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        self.quit_quietly();
    }

    /// Quit without bringing a hidden window back first (the system asks the program to end).
    pub fn quit_quietly(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        if let Some(c) = self.ctx.get() {
            c.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // If the window loop is asleep and never sees the request, do not hang around.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(3));
            std::process::exit(0);
        });
    }

    // ------------------------------------------------------------ background events

    /// Reacts to bridge events even while the window is hidden: wakes the window, copies incoming text.
    pub fn spawn_event_task(self: &Arc<Self>) {
        let me = self.clone();
        let mut rx = self.bridge.events();
        self.rt.spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match rx.recv().await {
                    Ok(Event::NoteAdded(n)) if n.src != "pc" => {
                        match (n.kind, n.text.as_deref(), n.file.as_ref()) {
                            (NoteKind::Text, Some(text), _) if me.settings().copy_incoming_text => {
                                match me.write_clipboard_text(text) {
                                    Ok(()) => me.toast(format!("Text from {} copied to the clipboard", n.src), false),
                                    Err(e) => me.toast(e, true),
                                }
                            }
                            (NoteKind::Text, _, _) => me.toast(format!("Note from {}", n.src), false),
                            (NoteKind::File, _, Some(f)) => me.toast(format!("{} received from {}", f.name, n.src), false),
                            _ => {}
                        }
                    }
                    Ok(Event::PeerNote { note, .. }) if note.src != "pc" => match (note.kind, note.file.as_ref()) {
                        (NoteKind::File, Some(f)) => me.toast(format!("{} received from {}", f.name, note.src), false),
                        _ => me.toast(format!("Message from {}", note.src), false),
                    },
                    Ok(Event::Transfer(t)) if t.state == TransferState::Offered && matches!(t.dir, Direction::ToPc | Direction::FromPeer) => {
                        // Someone wants to send a file: the answer needs a person.
                        me.show_window();
                        me.toast(format!("{} wants to send {}", t.device, t.name), false);
                    }
                    Ok(Event::PairRequest(r)) => {
                        // Someone asks to pair: bring the window up, the decision needs a person.
                        me.show_window();
                        me.toast(format!("{} wants to pair", r.name), false);
                    }
                    Ok(_) => {}
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                }
                me.repaint();
            }
        });
    }
}

/// Programs and scripts that a double click would start.
fn is_runnable(name: &str) -> bool {
    const RUNNABLE: [&str; 33] = [
        "exe", "com", "bat", "cmd", "msi", "scr", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "jar", "lnk", "reg", "dll", "cpl",
        "sh", "bash", "command", "app", "desktop", "appimage", "run", "pif", "gadget", "msc", "py", "pl", "workflow",
    ];
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    RUNNABLE.contains(&ext.as_str())
}

/// Open the folder of `path` and select the file there (Windows and macOS); elsewhere just open the folder.
fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Some(dir) = path.parent() {
            open_folder(dir);
        }
    }
}

pub fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    if let Err(e) = open::that_detached(path) {
        tracing::warn!("could not open {}: {e}", path.display());
    }
}

//! Tray icon with a small menu, and the global hotkey. Both keep working while the window is hidden.
//!
//! Linux: the tray library needs a GTK main loop, so it lives on its own thread.
//! Windows and macOS: the tray is created on the main thread, right after the window system starts.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use global_hotkey::hotkey::HotKey;
use std::sync::atomic::AtomicBool;
use crate::shortcut::Plan;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::hub::Hub;
use crate::icon;

/// Things that must stay alive for the tray and hotkey to keep working.
#[derive(Default)]
pub struct Keep {
    hotkey: Option<GlobalHotKeyManager>,
    /// The key registered with the system right now.
    registered: Option<HotKey>,
    /// Set to stop the thread that watches the keyboard (shortcuts with two ordinary keys, or only modifiers).
    watcher: Option<Arc<AtomicBool>>,
    _tray: Option<TrayIcon>,
}

impl Keep {
    /// Set up the hotkey from the settings (or none, when it is turned off), replacing the previous one.
    /// Must be called from the main thread.
    pub fn apply_hotkey(&mut self, hub: &Arc<Hub>) {
        let note = |s: String| *hub.hotkey_note.lock().unwrap_or_else(|e| e.into_inner()) = s;
        self.pause_hotkey();
        hub.hotkey_id.store(0, Ordering::SeqCst);
        let s = hub.settings();
        if !s.hotkey_enabled {
            note("Turned off in Settings".into());
            return;
        }
        let text = s.hotkey.text();
        match s.hotkey.plan() {
            None => note(format!("{text} is not a valid shortcut")),
            Some(Plan::System(key)) => {
                let Some(manager) = &self.hotkey else { return };
                match manager.register(key) {
                    Ok(()) => {
                        hub.hotkey_id.store(key.id(), Ordering::SeqCst);
                        self.registered = Some(key);
                        note(format!("{text} sends the clipboard to the phone"));
                    }
                    Err(e) => note(format!("{text} is unavailable: {e}")),
                }
            }
            Some(Plan::Watch(groups)) => {
                let stop = Arc::new(AtomicBool::new(false));
                self.watcher = Some(stop.clone());
                note(format!("{text} sends the clipboard to the phone"));
                watch_keyboard(hub.clone(), groups, text, stop);
            }
        }
    }

    /// Release the hotkey for a moment (while the user chooses a new one, so pressing it does not trigger it).
    pub fn pause_hotkey(&mut self) {
        if let (Some(manager), Some(old)) = (&self.hotkey, self.registered.take()) {
            let _ = manager.unregister(old);
        }
        if let Some(stop) = self.watcher.take() {
            stop.store(true, Ordering::SeqCst);
        }
    }
}

/// Watch the keyboard state (about 30 times a second) and send the clipboard when exactly the keys of the
/// shortcut go down together. Unlike a system hotkey this does not keep the keys from other programs.
/// On a Mac it needs the Accessibility permission, on Linux an X11 session.
fn watch_keyboard(hub: Arc<Hub>, groups: Vec<Vec<device_query::Keycode>>, text: String, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let note = |s: String| *hub.hotkey_note.lock().unwrap_or_else(|e| e.into_inner()) = s;
        let state = match std::panic::catch_unwind(device_query::DeviceState::checked_new) {
            Ok(Some(s)) => s,
            _ => {
                note(if cfg!(target_os = "macos") {
                    format!("{text} needs the Accessibility permission (System Settings > Privacy & Security), or a shortcut with one ordinary key")
                } else {
                    format!("{text} cannot be watched here (needs an X11 session), or use a shortcut with one ordinary key")
                });
                return;
            }
        };
        let all: Vec<device_query::Keycode> = groups.iter().flatten().copied().collect();
        let mut was_down = false;
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(30));
            let keys = state.query_keymap();
            let now_down = groups.iter().all(|g| g.iter().any(|k| keys.contains(k))) && keys.iter().all(|k| all.contains(k));
            if now_down && !was_down {
                let h = hub.clone();
                std::thread::spawn(move || h.send_clipboard());
            }
            was_down = now_down;
        }
    });
}

pub fn start(hub: &Arc<Hub>) -> Keep {
    let mut keep = Keep::default();
    install_event_handlers(hub);

    #[cfg(target_os = "linux")]
    {
        let ok = linux_tray_thread(hub.clone());
        hub.tray_ok.store(ok, Ordering::SeqCst);
    }
    #[cfg(not(target_os = "linux"))]
    {
        match build_tray() {
            Ok(t) => {
                hub.tray_ok.store(true, Ordering::SeqCst);
                keep._tray = Some(t);
            }
            Err(e) => tracing::warn!("no tray icon: {e}"),
        }
    }

    keep.hotkey = create_hotkey_manager(hub);
    keep.apply_hotkey(hub);
    keep
}

fn install_event_handlers(hub: &Arc<Hub>) {
    let h = hub.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| match e.id.0.as_str() {
        "show" => h.show_window(),
        "clip" => {
            let h2 = h.clone();
            std::thread::spawn(move || h2.send_clipboard());
        }
        "quit" => h.quit(),
        _ => {}
    }));
    let h = hub.clone();
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            h.show_window();
        }
    }));
}

fn build_tray() -> Result<TrayIcon, String> {
    let menu = Menu::new();
    let items = [
        MenuItem::with_id("show", "Open Bridge", true, None),
        MenuItem::with_id("clip", "Send clipboard to phone", true, None),
    ];
    for i in &items {
        menu.append(i).map_err(|e| e.to_string())?;
    }
    menu.append(&PredefinedMenuItem::separator()).map_err(|e| e.to_string())?;
    menu.append(&MenuItem::with_id("quit", "Quit", true, None)).map_err(|e| e.to_string())?;

    let icon = tray_icon::Icon::from_rgba(icon::rgba(64), 64, 64).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Bridge")
        .with_icon(icon)
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn linux_tray_thread(_hub: Arc<Hub>) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Err(e) = gtk::init() {
            tracing::warn!("no tray icon (GTK): {e}");
            let _ = tx.send(false);
            return;
        }
        match build_tray() {
            Ok(tray) => {
                let _ = tx.send(true);
                gtk::main(); // runs for the life of the program
                drop(tray);
            }
            Err(e) => {
                tracing::warn!("no tray icon: {e}");
                let _ = tx.send(false);
            }
        }
    });
    rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap_or(false)
}

fn create_hotkey_manager(hub: &Arc<Hub>) -> Option<GlobalHotKeyManager> {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            *hub.hotkey_note.lock().unwrap_or_else(|e| e.into_inner()) = format!("Hotkey unavailable: {e}");
            return None;
        }
    };
    let h = hub.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        let id = h.hotkey_id.load(Ordering::SeqCst);
        if id != 0 && e.id == id && e.state == HotKeyState::Pressed {
            let h2 = h.clone();
            std::thread::spawn(move || h2.send_clipboard());
        }
    }));
    Some(manager)
}

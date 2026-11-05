//! Tray icon with a small menu, and the global hotkey. Both keep working while the window is hidden.
//!
//! Linux: the icon speaks the StatusNotifierItem protocol directly (KDE, GNOME with the AppIndicator extension,
//! XFCE, Cinnamon ...). Unlike an AppIndicator it reports a left click, so a click on the icon brings the window
//! back and a right click opens the menu. It runs on its own thread and needs no GTK.
//! Windows and macOS: the tray is created on the main thread, right after the window system starts.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use global_hotkey::hotkey::HotKey;
use std::sync::atomic::AtomicBool;
use crate::shortcut::Plan;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
#[cfg(not(target_os = "linux"))]
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
#[cfg(not(target_os = "linux"))]
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
    #[cfg(not(target_os = "linux"))]
    _tray: Option<TrayIcon>,
    #[cfg(target_os = "linux")]
    _tray: Option<ksni::blocking::Handle<LinuxTray>>,
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

    #[cfg(target_os = "linux")]
    {
        match linux_tray(hub.clone()) {
            Ok(handle) => {
                hub.tray_ok.store(true, Ordering::SeqCst);
                keep._tray = Some(handle);
            }
            Err(e) => tracing::warn!("no tray icon: {e}"),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        install_event_handlers(hub);
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

#[cfg(not(target_os = "linux"))]
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

#[cfg(not(target_os = "linux"))]
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

/// The tray icon on Linux. Left click (the "activate" action) shows the window, the menu opens on right click.
#[cfg(target_os = "linux")]
pub struct LinuxTray {
    hub: Arc<Hub>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxTray {
    // false: a click on the icon is reported to `activate` instead of opening the menu
    const MENU_ON_ACTIVATE: bool = false;

    fn id(&self) -> String {
        "bridge-app".into()
    }

    fn title(&self) -> String {
        "Bridge".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Bridge".into(), ..Default::default() }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // the tray wants ARGB, the icon is RGBA
        let mut data = icon::rgba(64);
        for px in data.chunks_exact_mut(4) {
            px.rotate_right(1);
        }
        vec![ksni::Icon { width: 64, height: 64, data }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.hub.show_window();
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.hub.show_window();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem { label: "Open Bridge".into(), activate: Box::new(|t: &mut Self| t.hub.show_window()), ..Default::default() }.into(),
            StandardItem {
                label: "Send clipboard to phone".into(),
                activate: Box::new(|t: &mut Self| {
                    let h = t.hub.clone();
                    std::thread::spawn(move || h.send_clipboard());
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem { label: "Quit".into(), activate: Box::new(|t: &mut Self| t.hub.quit()), ..Default::default() }.into(),
        ]
    }

    // When the panel that shows the icon goes away (it restarts, or the user removes the applet) the window
    // must not be hidden any more, or nobody could bring it back.
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.hub.tray_ok.store(false, Ordering::SeqCst);
        true
    }

    fn watcher_online(&self) {
        self.hub.tray_ok.store(true, Ordering::SeqCst);
    }
}

#[cfg(target_os = "linux")]
fn linux_tray(hub: Arc<Hub>) -> Result<ksni::blocking::Handle<LinuxTray>, String> {
    use ksni::blocking::TrayMethods;
    LinuxTray { hub }.spawn().map_err(|e| e.to_string())
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

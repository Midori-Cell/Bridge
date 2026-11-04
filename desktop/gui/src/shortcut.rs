//! The keyboard shortcut that sends the clipboard to the phone. It can be changed in Settings.
//!
//! A shortcut is any set of keys held together: modifier keys (Ctrl, Alt, Shift, Cmd) and ordinary keys.
//! Modifiers plus exactly one ordinary key (for example Ctrl+Alt+B) are registered with the system, which is
//! the dependable way and keeps the key from reaching other programs. Everything else (two ordinary keys such
//! as C+P, or only modifiers) is recognised by watching the keyboard state, see `tray.rs`.

use egui::Key;
use global_hotkey::hotkey::{Code, HotKey, Modifiers as HkMods};
use device_query::Keycode;
use serde::{Deserialize, Serialize};

/// (name in the settings file, key as egui reports it, key for the system hotkey, key when watching the keyboard)
const KEYS: &[(&str, Key, Code, Keycode)] = &[
    ("A", Key::A, Code::KeyA, Keycode::A),
    ("B", Key::B, Code::KeyB, Keycode::B),
    ("C", Key::C, Code::KeyC, Keycode::C),
    ("D", Key::D, Code::KeyD, Keycode::D),
    ("E", Key::E, Code::KeyE, Keycode::E),
    ("F", Key::F, Code::KeyF, Keycode::F),
    ("G", Key::G, Code::KeyG, Keycode::G),
    ("H", Key::H, Code::KeyH, Keycode::H),
    ("I", Key::I, Code::KeyI, Keycode::I),
    ("J", Key::J, Code::KeyJ, Keycode::J),
    ("K", Key::K, Code::KeyK, Keycode::K),
    ("L", Key::L, Code::KeyL, Keycode::L),
    ("M", Key::M, Code::KeyM, Keycode::M),
    ("N", Key::N, Code::KeyN, Keycode::N),
    ("O", Key::O, Code::KeyO, Keycode::O),
    ("P", Key::P, Code::KeyP, Keycode::P),
    ("Q", Key::Q, Code::KeyQ, Keycode::Q),
    ("R", Key::R, Code::KeyR, Keycode::R),
    ("S", Key::S, Code::KeyS, Keycode::S),
    ("T", Key::T, Code::KeyT, Keycode::T),
    ("U", Key::U, Code::KeyU, Keycode::U),
    ("V", Key::V, Code::KeyV, Keycode::V),
    ("W", Key::W, Code::KeyW, Keycode::W),
    ("X", Key::X, Code::KeyX, Keycode::X),
    ("Y", Key::Y, Code::KeyY, Keycode::Y),
    ("Z", Key::Z, Code::KeyZ, Keycode::Z),
    ("0", Key::Num0, Code::Digit0, Keycode::Key0),
    ("1", Key::Num1, Code::Digit1, Keycode::Key1),
    ("2", Key::Num2, Code::Digit2, Keycode::Key2),
    ("3", Key::Num3, Code::Digit3, Keycode::Key3),
    ("4", Key::Num4, Code::Digit4, Keycode::Key4),
    ("5", Key::Num5, Code::Digit5, Keycode::Key5),
    ("6", Key::Num6, Code::Digit6, Keycode::Key6),
    ("7", Key::Num7, Code::Digit7, Keycode::Key7),
    ("8", Key::Num8, Code::Digit8, Keycode::Key8),
    ("9", Key::Num9, Code::Digit9, Keycode::Key9),
    ("F1", Key::F1, Code::F1, Keycode::F1),
    ("F2", Key::F2, Code::F2, Keycode::F2),
    ("F3", Key::F3, Code::F3, Keycode::F3),
    ("F4", Key::F4, Code::F4, Keycode::F4),
    ("F5", Key::F5, Code::F5, Keycode::F5),
    ("F6", Key::F6, Code::F6, Keycode::F6),
    ("F7", Key::F7, Code::F7, Keycode::F7),
    ("F8", Key::F8, Code::F8, Keycode::F8),
    ("F9", Key::F9, Code::F9, Keycode::F9),
    ("F10", Key::F10, Code::F10, Keycode::F10),
    ("F11", Key::F11, Code::F11, Keycode::F11),
    ("F12", Key::F12, Code::F12, Keycode::F12),
    ("Space", Key::Space, Code::Space, Keycode::Space),
    ("Up", Key::ArrowUp, Code::ArrowUp, Keycode::Up),
    ("Down", Key::ArrowDown, Code::ArrowDown, Keycode::Down),
    ("Left", Key::ArrowLeft, Code::ArrowLeft, Keycode::Left),
    ("Right", Key::ArrowRight, Code::ArrowRight, Keycode::Right),
    ("Home", Key::Home, Code::Home, Keycode::Home),
    ("End", Key::End, Code::End, Keycode::End),
    ("PageUp", Key::PageUp, Code::PageUp, Keycode::PageUp),
    ("PageDown", Key::PageDown, Code::PageDown, Keycode::PageDown),
    ("Insert", Key::Insert, Code::Insert, Keycode::Insert),
    ("Delete", Key::Delete, Code::Delete, Keycode::Delete),
];

/// Modifier names, in the order they are written.
pub const MODS: [&str; 4] = ["Ctrl", "Alt", "Shift", "Cmd"];

/// The keys held together, in the order they were pressed, for example `["Ctrl", "Alt", "B"]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Raw")]
pub struct Shortcut {
    pub keys: Vec<String>,
}

/// The settings file of the previous version stored modifiers as flags and one key code.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Raw {
    keys: Vec<String>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
    key: String,
}

impl From<Raw> for Shortcut {
    fn from(r: Raw) -> Shortcut {
        if !r.keys.is_empty() {
            return Shortcut { keys: r.keys };
        }
        let legacy = r.key.trim_start_matches("Key").trim_start_matches("Digit").to_string();
        if legacy.is_empty() || !KEYS.iter().any(|k| k.0 == legacy) {
            return Shortcut::default();
        }
        let mut keys: Vec<String> = Vec::new();
        for (on, name) in [(r.ctrl, "Ctrl"), (r.alt, "Alt"), (r.shift, "Shift"), (r.meta, "Cmd")] {
            if on {
                keys.push(name.into());
            }
        }
        keys.push(legacy);
        Shortcut { keys }
    }
}

impl Default for Shortcut {
    fn default() -> Self {
        Shortcut { keys: vec!["Ctrl".into(), "Alt".into(), "B".into()] }
    }
}

/// How a shortcut is recognised.
pub enum Plan {
    /// Registered with the system.
    System(HotKey),
    /// Watched for in the keyboard state: one group per key, any key of a group counts.
    Watch(Vec<Vec<Keycode>>),
}

fn is_mod(name: &str) -> bool {
    MODS.contains(&name)
}

impl Shortcut {
    /// The keys that are down right now, for recording: modifiers first as they are held, then the ordinary keys.
    pub fn names_down(down: &std::collections::HashSet<Key>, m: &egui::Modifiers) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (on, name) in [(m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.mac_cmd, "Cmd")] {
            if on {
                out.push(name.into());
            }
        }
        for (name, key, _, _) in KEYS {
            if down.contains(key) {
                out.push((*name).into());
            }
        }
        out
    }

    /// At least two keys, so a single letter is never taken away from typing.
    pub fn is_valid(&self) -> bool {
        self.keys.len() >= 2 && self.keys.iter().all(|k| is_mod(k) || KEYS.iter().any(|x| x.0 == k)) && {
            let mut seen = std::collections::HashSet::new();
            self.keys.iter().all(|k| seen.insert(k))
        }
    }

    pub fn plan(&self) -> Option<Plan> {
        if !self.is_valid() {
            return None;
        }
        let mods: Vec<&String> = self.keys.iter().filter(|k| is_mod(k)).collect();
        let main: Vec<&String> = self.keys.iter().filter(|k| !is_mod(k)).collect();
        if main.len() == 1 && !mods.is_empty() {
            let code = KEYS.iter().find(|k| k.0 == main[0])?.2;
            let mut m = HkMods::empty();
            for name in &mods {
                m |= match name.as_str() {
                    "Ctrl" => HkMods::CONTROL,
                    "Alt" => HkMods::ALT,
                    "Shift" => HkMods::SHIFT,
                    _ => HkMods::META,
                };
            }
            return Some(Plan::System(HotKey::new(Some(m), code)));
        }
        let mut groups: Vec<Vec<Keycode>> = Vec::new();
        for name in &self.keys {
            groups.push(match name.as_str() {
                "Ctrl" => vec![Keycode::LControl, Keycode::RControl],
                "Alt" => vec![Keycode::LAlt, Keycode::RAlt, Keycode::LOption, Keycode::ROption],
                "Shift" => vec![Keycode::LShift, Keycode::RShift],
                "Cmd" => vec![Keycode::Command, Keycode::RCommand, Keycode::LMeta, Keycode::RMeta],
                other => vec![KEYS.iter().find(|k| k.0 == other)?.3],
            });
        }
        Some(Plan::Watch(groups))
    }

    /// For example "Ctrl+Alt+B" (on a Mac "Control+Option+B", with "Cmd" for the Command key).
    pub fn text(&self) -> String {
        text_of(&self.keys)
    }
}

/// The keys joined with "+", named the way the keyboard of this system names them.
pub fn text_of(keys: &[String]) -> String {
    let mac = cfg!(target_os = "macos");
    keys.iter()
        .map(|k| match (k.as_str(), mac) {
            ("Ctrl", true) => "Control".to_string(),
            ("Alt", true) => "Option".to_string(),
            _ => k.clone(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

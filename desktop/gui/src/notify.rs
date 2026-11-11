//! Message notifications: a sound and a small popup from the system. (The icon in the taskbar, Dock or tray that
//! asks for attention is handled by `hub.rs` and `tray.rs`.) No extra libraries: each system has a command for it.
//!
//! Linux: `notify-send` and the sound theme (`canberra-gtk-play`, or `paplay`). macOS: `osascript` and `afplay`.
//! Windows: a small PowerShell script that shows a toast. Where a command is missing nothing happens.

use std::process::{Command, Stdio};

/// Shorten a message for the popup: its first line, at most `max` characters.
pub fn preview(text: &str, max: usize) -> String {
    let line = text.trim().lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        return line.to_string();
    }
    let cut: String = line.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Show a popup and/or play the sound. Runs on its own thread, so the caller never waits.
pub fn alert(title: String, body: String, popup: bool, sound: bool) {
    if !popup && !sound {
        return;
    }
    std::thread::spawn(move || run(&title, &body, popup, sound));
}

fn quiet(cmd: &mut Command) -> Option<bool> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().ok().map(|s| s.success())
}

#[cfg(target_os = "linux")]
fn run(title: &str, body: &str, popup: bool, sound: bool) {
    if popup {
        let _ = quiet(Command::new("notify-send").args(["--app-name=Bridge", "--icon=bridge", "--expire-time=6000", "--", title, body]));
    }
    if sound {
        let tried = [
            ("canberra-gtk-play", vec!["--id=message-new-instant", "--description=Bridge"]),
            ("paplay", vec!["/usr/share/sounds/freedesktop/stereo/message-new-instant.oga"]),
            ("pw-play", vec!["/usr/share/sounds/freedesktop/stereo/message-new-instant.oga"]),
        ];
        for (cmd, args) in tried {
            if quiet(Command::new(cmd).args(args)) == Some(true) {
                break;
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn run(title: &str, body: &str, popup: bool, sound: bool) {
    // AppleScript string: backslash and quote are escaped, so a message cannot end the string
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ");
    if popup {
        let tail = if sound { " sound name \"Glass\"" } else { "" };
        let script = format!("display notification \"{}\" with title \"{}\"{}", esc(body), esc(title), tail);
        let _ = quiet(Command::new("osascript").args(["-e", &script]));
    } else if sound {
        let _ = quiet(Command::new("afplay").arg("/System/Library/Sounds/Glass.aiff"));
    }
}

#[cfg(windows)]
fn run(title: &str, body: &str, popup: bool, sound: bool) {
    use std::os::windows::process::CommandExt;
    let xml_escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
    // The text travels in environment variables, so it is never part of the script.
    let toast = format!(
        "<toast><visual><binding template='ToastGeneric'><text>{}</text><text>{}</text></binding></visual>{}</toast>",
        xml_escape(title),
        xml_escape(body),
        if sound { "" } else { "<audio silent='true'/>" }
    );
    let script = if popup {
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
         [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null; \
         $x = New-Object Windows.Data.Xml.Dom.XmlDocument; $x.LoadXml($env:BRIDGE_TOAST); \
         $t = [Windows.UI.Notifications.ToastNotification]::new($x); \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe').Show($t); \
         Start-Sleep -Milliseconds 1500"
    } else {
        "[System.Media.SystemSounds]::Asterisk.Play(); Start-Sleep -Milliseconds 1200"
    };
    let _ = quiet(
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
            .env("BRIDGE_TOAST", toast)
            .creation_flags(0x0800_0000), // no console window
    );
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn run(_title: &str, _body: &str, _popup: bool, _sound: bool) {}

#[cfg(test)]
mod tests {
    use super::preview;

    #[test]
    fn preview_takes_the_first_line_and_shortens_it() {
        assert_eq!(preview("  hello\nworld ", 20), "hello");
        assert_eq!(preview("abcdefghij", 5), "abcd…");
        assert_eq!(preview("äöüäöü", 6), "äöüäöü");
        assert_eq!(preview("", 5), "");
    }
}

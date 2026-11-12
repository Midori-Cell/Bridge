//! The popup of a message notification, shown by the system. (The sound is Bridge's own, in `sound.rs`; the icon
//! that asks for attention is handled by `hub.rs` and `tray.rs`.) Each system has a command for the popup:
//! Linux `notify-send`, macOS `osascript`, Windows a small PowerShell script that shows a toast. Where the command
//! is missing nothing happens.

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

/// Show a popup from the system. Runs on its own thread, so the caller never waits. (The sound is Bridge's own,
/// see `sound.rs`; the popup is always silent.)
pub fn popup(title: String, body: String) {
    std::thread::spawn(move || run(&title, &body));
}

fn quiet(cmd: &mut Command) -> Option<bool> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().ok().map(|s| s.success())
}

#[cfg(target_os = "linux")]
fn run(title: &str, body: &str) {
    // the hint asks the notification server not to play its own sound
    let _ = quiet(Command::new("notify-send").args(["--app-name=Bridge", "--icon=bridge", "--expire-time=6000", "--hint=boolean:suppress-sound:true", "--", title, body]));
}

#[cfg(target_os = "macos")]
fn run(title: &str, body: &str) {
    // AppleScript string: backslash and quote are escaped, so a message cannot end the string
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ");
    let script = format!("display notification \"{}\" with title \"{}\"", esc(body), esc(title));
    let _ = quiet(Command::new("osascript").args(["-e", &script]));
}

#[cfg(windows)]
fn run(title: &str, body: &str) {
    use std::os::windows::process::CommandExt;
    let xml_escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
    // The text travels in an environment variable, so it is never part of the script. The toast is silent.
    let toast = format!(
        "<toast><visual><binding template='ToastGeneric'><text>{}</text><text>{}</text></binding></visual><audio silent='true'/></toast>",
        xml_escape(title),
        xml_escape(body),
    );
    let script = "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
         [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null; \
         $x = New-Object Windows.Data.Xml.Dom.XmlDocument; $x.LoadXml($env:BRIDGE_TOAST); \
         $t = [Windows.UI.Notifications.ToastNotification]::new($x); \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe').Show($t); \
         Start-Sleep -Milliseconds 1500";
    let _ = quiet(
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
            .env("BRIDGE_TOAST", toast)
            .creation_flags(0x0800_0000), // no console window
    );
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn run(_title: &str, _body: &str) {}

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

//! Helpers used by several screens: time and size text, QR drawing, history and transfer lists.

use std::time::{SystemTime, UNIX_EPOCH};

use bridge_core::{Direction, FileRef, Note, NoteKind, TransferInfo, TransferState};
use egui::{Color32, RichText, Sense, Ui, Vec2};

use crate::icons::Icon;
use crate::theme::palette;
use crate::widgets;

pub fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// "2026-11-05 14-03-22" (UTC), usable in a file name.
pub fn utc_stamp(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}-{:02}-{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn ago(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..=4 => "just now".to_string(),
        5..=59 => format!("{s} s ago"),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

/// Very short form for the sidebar: "now", "2 min", "3 h", "4 d".
pub fn short_ago(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{} min", s / 60),
        3600..=86_399 => format!("{} h", s / 3600),
        _ => format!("{} d", s / 86_400),
    }
}

pub fn size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (mut v, mut i) = (n as f64, 0);
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

/// Draw a QR code as black squares on white, with the quiet border scanners need.
pub fn draw_qr(ui: &mut Ui, code: &qrcode::QrCode, side: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, Color32::WHITE);
    let width = code.width();
    let quiet = 3.0;
    let module = side / (width as f32 + 2.0 * quiet);
    let at = |i: usize| (i as f32 + quiet) * module;
    for (i, c) in code.to_colors().iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (x, y) = (i % width, i / width);
            // round to whole pixels so neighbouring squares leave no hairline gaps
            let min = egui::pos2((rect.min.x + at(x)).round(), (rect.min.y + at(y)).round());
            let max = egui::pos2((rect.min.x + at(x + 1)).round(), (rect.min.y + at(y + 1)).round());
            painter.rect_filled(egui::Rect::from_min_max(min, max), 0.0, Color32::BLACK);
        }
    }
}

pub enum Compose {
    Nothing,
    SendText,
    SendClipboard,
    PickFiles,
}

/// What the user did to a message in the chat.
pub enum MsgAction {
    /// Answer this message (by id).
    Reply(String),
    Copy(String),
    Delete(String),
    Open(FileRef),
    ShowFolder(FileRef),
    /// Stop answering a message.
    ClearReply,
    /// A web address in a message was clicked.
    OpenLink(String),
}

pub enum HistoryAction {
    Copy(String),
    Open(FileRef),
    ShowFolder(FileRef),
}

/// Newest first. Text or file icon at the left, then who and when, then the content.
pub fn history(ui: &mut Ui, notes: &[Note]) -> Option<HistoryAction> {
    let p = palette(ui);
    let now = now_secs();
    let mut action = None;
    if notes.is_empty() {
        widgets::grouped(ui, |g| g.row(|ui| widgets::footnote(ui, "No files yet.")));
        return None;
    }
    widgets::grouped(ui, |g| {
        for n in notes.iter().rev() {
            g.row(|ui| {
                ui.horizontal_top(|ui| {
                    widgets::icon_badge(ui, if n.kind == NoteKind::File { Icon::File } else { Icon::Text });
                    ui.vertical(|ui| {
                        let who = if n.src == "pc" { "You".to_string() } else { n.src.clone() };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{who}  \u{b7}  {}", ago(now - n.ts))).small().color(p.label2));
                            widgets::trailing(ui, |ui| match (&n.kind, &n.text, &n.file) {
                                (NoteKind::Text, Some(text), _) => {
                                    if widgets::plain(ui, "Copy").clicked() {
                                        action = Some(HistoryAction::Copy(text.clone()));
                                    }
                                }
                                (NoteKind::File, _, Some(f)) if f.local_path.is_some() => {
                                    if widgets::plain(ui, "Show folder").clicked() {
                                        action = Some(HistoryAction::ShowFolder(f.clone()));
                                    }
                                    if widgets::plain(ui, "Open").clicked() {
                                        action = Some(HistoryAction::Open(f.clone()));
                                    }
                                }
                                _ => {}
                            });
                        });
                        match (&n.kind, &n.text, &n.file) {
                            (NoteKind::Text, Some(text), _) => {
                                let shown: String = text.chars().take(1000).collect();
                                ui.add(egui::Label::new(shown).wrap());
                            }
                            (NoteKind::File, _, Some(f)) => {
                                let what = match f.dir {
                                    Direction::ToPc | Direction::FromPeer => "Received",
                                    Direction::ToPhone => "Offered",
                                    Direction::ToPeer => "Sent",
                                };
                                ui.add(egui::Label::new(format!("{what} file: {} ({})", f.name, size(f.size))).wrap());
                            }
                            _ => {}
                        }
                    });
                });
            });
        }
    });
    action
}

pub enum TransferAction {
    Accept(String),
    Decline(String),
    Cancel(String),
    Remove(String),
}

fn incoming(t: &TransferInfo) -> bool {
    matches!(t.dir, Direction::ToPc | Direction::FromPeer)
}

/// Transfers that need attention or are running. Offers wait here for a decision. Nothing is drawn
/// when the list is empty.
pub fn transfers(ui: &mut Ui, list: &[TransferInfo]) -> Option<TransferAction> {
    if list.is_empty() {
        return None;
    }
    let p = palette(ui);
    let mut action = None;
    widgets::section(ui, "TRANSFERS");
    widgets::grouped(ui, |g| {
        for t in list {
            g.row(|ui| {
                let who = if t.device.is_empty() { "the phone" } else { &t.device };
                let way = if incoming(t) { format!("from {who}") } else { format!("to {who}") };
                // the first line says what is going on, the second which file
                let (title, sub, sub_color) = match t.state {
                    TransferState::Offered if incoming(t) => (format!("{who} wants to send a file"), format!("{}  \u{b7}  {}", t.name, size(t.size)), p.label2),
                    TransferState::Offered => (format!("Waiting for {who} to accept"), format!("{}  \u{b7}  {}", t.name, size(t.size)), p.label2),
                    TransferState::Queued => (format!("Waiting for {who} to be online"), format!("{}  \u{b7}  {}  \u{b7}  Sent when {who} is back", t.name, size(t.size)), p.orange),
                    TransferState::Hashing => (format!("Preparing {}", t.name), size(t.size), p.label2),
                    TransferState::Active => (t.name.clone(), format!("{way}  \u{b7}  {} of {}", size(t.done), size(t.size)), p.label2),
                    TransferState::Done => (t.name.clone(), format!("{way}  \u{b7}  {}  \u{b7}  Done", size(t.size)), p.green),
                    TransferState::Interrupted => (t.name.clone(), format!("Interrupted at {}. It can be resumed.", size(t.done)), p.orange),
                    TransferState::Failed => (t.name.clone(), format!("Failed: {}", t.error.as_deref().unwrap_or("unknown reason")), p.red),
                    TransferState::Declined => (t.error.clone().unwrap_or_else(|| "Declined".into()), t.name.clone(), p.label2),
                    TransferState::Cancelled => (t.error.clone().unwrap_or_else(|| "Cancelled".into()), t.name.clone(), p.label2),
                };
                ui.horizontal(|ui| {
                    widgets::icon_badge(ui, Icon::File);
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
                        ui.add(egui::Label::new(RichText::new(sub).small().color(sub_color)).truncate());
                    });
                    widgets::trailing(ui, |ui| match t.state {
                        TransferState::Offered if incoming(t) => {
                            if widgets::primary(ui, "Accept").clicked() {
                                action = Some(TransferAction::Accept(t.id.clone()));
                            }
                            if widgets::secondary(ui, "Decline").clicked() {
                                action = Some(TransferAction::Decline(t.id.clone()));
                            }
                        }
                        TransferState::Offered | TransferState::Queued | TransferState::Hashing | TransferState::Active if t.dir != Direction::ToPhone => {
                            if widgets::secondary(ui, "Cancel").clicked() {
                                action = Some(TransferAction::Cancel(t.id.clone()));
                            }
                        }
                        TransferState::Declined | TransferState::Cancelled | TransferState::Failed => {
                            let remove = widgets::plain(ui, "Remove").on_hover_text("Remove from the list (also on the other PC)").clicked();
                            if remove {
                                action = Some(TransferAction::Remove(t.id.clone()));
                            }
                        }
                        _ => {}
                    });
                });
                match t.state {
                    TransferState::Hashing => {
                        ui.add(egui::ProgressBar::new(0.0).desired_height(6.0).fill(p.tint).animate(true));
                    }
                    TransferState::Active => {
                        let f = if t.size == 0 { 1.0 } else { t.done as f32 / t.size as f32 };
                        ui.add(egui::ProgressBar::new(f).desired_height(6.0).fill(p.tint));
                    }
                    _ => {}
                }
            });
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        assert_eq!(ago(1.0), "just now");
        assert_eq!(ago(30.0), "30 s ago");
        assert_eq!(ago(120.0), "2 min ago");
        assert_eq!(ago(7200.0), "2 h ago");
        assert_eq!(ago(200_000.0), "2 d ago");
        assert_eq!(ago(-5.0), "just now");
    }

    #[test]
    fn short_times() {
        assert_eq!(short_ago(10.0), "now");
        assert_eq!(short_ago(150.0), "2 min");
        assert_eq!(short_ago(7300.0), "2 h");
        assert_eq!(short_ago(200_000.0), "2 d");
    }

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(55_050_240), "52.5 MB");
    }
}

/// A web address inside a message, as character positions (not bytes) so they match egui's text cursors.
#[derive(Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub start: usize,
    pub end: usize,
    /// Always starts with http:// or https://.
    pub url: String,
}

/// Find `http://`, `https://` and `www.` addresses in a message. Punctuation that ends a sentence is not part of the address.
pub fn find_links(text: &str) -> Vec<LinkSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let boundary = i == 0 || chars[i - 1].is_whitespace() || "([{<\"'".contains(chars[i - 1]);
        if boundary {
            let head: String = chars[i..].iter().take(8).collect::<String>().to_lowercase();
            let scheme = if head.starts_with("https://") {
                8
            } else if head.starts_with("http://") {
                7
            } else if head.starts_with("www.") {
                0
            } else {
                usize::MAX
            };
            if scheme != usize::MAX {
                let mut j = i;
                while j < chars.len() && !chars[j].is_whitespace() {
                    j += 1;
                }
                while j > i {
                    let c = chars[j - 1];
                    let token = &chars[i..j];
                    let open = |ch: char| token.iter().filter(|&&x| x == ch).count();
                    let trim = ".,;:!?'\"*>".contains(c) || (c == ')' && open('(') < open(')')) || (c == ']' && open('[') < open(']')) || (c == '}' && open('{') < open('}'));
                    if !trim {
                        break;
                    }
                    j -= 1;
                }
                let prefix = if scheme == 0 { 4 } else { scheme };
                if j > i + prefix {
                    let text: String = chars[i..j].iter().collect();
                    let url = if scheme == 0 { format!("https://{text}") } else { text };
                    out.push(LinkSpan { start: i, end: j, url });
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod stamp_tests {
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn formats_dates() {
        assert_eq!(super::utc_stamp(UNIX_EPOCH), "1970-01-01 00-00-00");
        assert_eq!(super::utc_stamp(UNIX_EPOCH + Duration::from_secs(1_762_351_402)), "2025-11-05 14-03-22");
        assert_eq!(super::utc_stamp(UNIX_EPOCH + Duration::from_secs(1_709_251_199)), "2024-02-29 23-59-59");
    }
}

#[cfg(test)]
mod link_tests {
    use super::find_links;

    #[test]
    fn finds_addresses_and_leaves_punctuation_out() {
        let l = find_links("see https://example.com/a?b=1, and (www.test.org/x). Done");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].url, "https://example.com/a?b=1");
        assert_eq!((l[0].start, l[0].end), (4, 29));
        assert_eq!(l[1].url, "https://www.test.org/x");
    }

    #[test]
    fn keeps_balanced_parentheses_and_ignores_plain_words() {
        assert_eq!(find_links("https://en.wikipedia.org/wiki/Rust_(language)")[0].url, "https://en.wikipedia.org/wiki/Rust_(language)");
        assert!(find_links("http:// www. nothing here, xhttps://no").is_empty());
    }

    #[test]
    fn positions_count_characters() {
        let l = find_links("\u{e4}\u{f6} https://a.de ok");
        assert_eq!((l[0].start, l[0].end), (3, 15));
    }
}

//! The conversation view: a transcript of message bubbles with a bordered top edge, and under it the
//! composer (text box, Files, Clipboard, Send) on the same card. Everything is sized from the space
//! it is given, so the chat grows and shrinks with the window.

use bridge_core::{Direction, FileRef, Note, NoteKind};
use egui::{
    pos2, vec2, Align, Color32, CornerRadius, Frame, Id, Layout, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextStyle, Ui,
    UiBuilder, Vec2,
};

use crate::common::{ago, find_links, now_secs, size, Compose, LinkSpan, MsgAction};
use crate::hub::SendKey;
use crate::icons::{self, Icon};
use crate::theme::{self, palette, Palette, CARD_RADIUS, FIELD_RADIUS};
use crate::widgets;

/// Text box (64) + gap + buttons (32) + padding above and below.
pub const COMPOSER_H: f32 = 64.0 + 8.0 + 32.0 + 20.0;
/// Extra height of the "Replying to" strip above the text box.
const REPLY_STRIP: f32 = 28.0 + 6.0;
/// Below this the chat would be too small to read; the page scrolls instead.
pub const MIN_HEIGHT: f32 = COMPOSER_H + REPLY_STRIP + 90.0;

pub struct Chat<'a> {
    /// Id of the conversation, so each one keeps its own scroll position.
    pub id: &'a str,
    /// Scroll to the newest message (after sending one).
    pub jump_to_end: bool,
    pub notes: &'a [Note],
    pub text: &'a mut String,
    pub hint: &'a str,
    pub clipboard: bool,
    pub send_key: SendKey,
    /// The message the text box is answering, if any.
    pub reply: Option<&'a Note>,
    /// Size of everything in the chat (messages and text box), 1.0 is normal. Set with ctrl + mouse wheel
    /// over the chat, independently of the zoom of the rest of the window.
    pub zoom: f32,
    /// Width of all messages of mine / of the other side, in points at normal size. `None`: as wide as the text.
    /// Set by dragging a message sideways.
    pub widths: Widths,
    /// Show times as 21:14 (true) or 9:14 PM (false).
    pub time_24h: bool,
}

/// The width the user dragged the messages to, separately for the own and the incoming ones.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Widths {
    pub own: Option<f32>,
    pub theirs: Option<f32>,
}

/// Narrowest a message can be dragged to (points at normal size).
pub const MIN_BUBBLE_W: f32 = 120.0;

pub struct ChatOut {
    /// A message was dragged: (is it one of mine, new width in points at normal size). Every message of that side follows.
    pub width: Option<(bool, f32)>,
    pub compose: Compose,
    pub msg: Option<MsgAction>,
    /// Where the chat is on screen, so the window knows when ctrl + wheel is meant for it.
    pub rect: Rect,
}

/// Id of the text box, so the window can focus it after "Reply".
pub fn composer_id() -> Id {
    theme::id("composer_edit")
}

pub fn show(ui: &mut Ui, height: f32, chat: Chat) -> ChatOut {
    let p = palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height.max(MIN_HEIGHT)), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(CARD_RADIUS), p.card);
    let composer_h = COMPOSER_H + if chat.reply.is_some() { REPLY_STRIP } else { 0.0 };
    let transcript_rect = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - composer_h));
    let composer_rect = Rect::from_min_max(pos2(rect.min.x, transcript_rect.max.y), rect.max);

    // Border on the top and the sides only: draw a full rounded rectangle that reaches below the
    // transcript and clip it, so the bottom edge (and its corners) are cut off.
    let clipped = ui.painter().with_clip_rect(transcript_rect);
    let tall = Rect::from_min_max(transcript_rect.min, transcript_rect.max + vec2(0.0, 24.0));
    clipped.rect(tall, CornerRadius::same(CARD_RADIUS), p.chat, Stroke::new(1.5, p.outline), StrokeKind::Inside);

    // The scrolling part reaches to the border at the top and to the text box at the bottom, so the
    // edge fades sit exactly on the edges where messages are cut off. The padding is inside the content.
    let scroll_area = Rect::from_min_max(transcript_rect.min + vec2(16.0, 1.5), transcript_rect.max - vec2(16.0, 0.0));
    let (msg, width) = transcript(ui, transcript_rect, scroll_area, &chat, p);
    let (compose, clear) = composer(ui, composer_rect.shrink2(vec2(14.0, 10.0)), chat, p);
    // The mouse wheel over the messages belongs to the chat: whatever it did not use (it was at its top or
    // bottom, or there is nothing to scroll) must not scroll the page behind it. Over the text box and over
    // the right eighth of the window the wheel scrolls the page.
    if wheel_zone(ui, transcript_rect) {
        ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
    }
    ChatOut { compose, msg: msg.or(clear.then_some(MsgAction::ClearReply)), rect, width }
}

/// Is the pointer where the mouse wheel scrolls the chat: on the messages, but not in the strip along
/// the right edge of the window (one eighth of its width), which is kept free for scrolling the page.
fn wheel_zone(ui: &Ui, transcript: Rect) -> bool {
    let window = ui.ctx().content_rect();
    let right = window.max.x - window.width() / 8.0;
    ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| transcript.contains(p) && p.x < right)
}

fn transcript(ui: &mut Ui, outer: Rect, area: Rect, chat: &Chat, p: Palette) -> (Option<MsgAction>, Option<(bool, f32)>) {
    let notes = chat.notes;
    let z = chat.zoom;
    let mut action = None;
    let mut width = None;
    if ui.input(|i| i.pointer.primary_pressed()) {
        ui.data_mut(|d| d.remove::<Selection>(selection_id()));
    }
    ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
        let memo = theme::id(("chat_content_height", chat.id));
        let last: f32 = ui.data(|d| d.get_temp(memo)).unwrap_or(0.0);
        let view = area.height();
        // egui's own edge fade is a plain rectangle in the panel color; it is switched off here and
        // replaced by `edge_fades`, which follows the rounded corners of the transcript.
        ui.spacing_mut().scroll.fade.strength = 0.0;
        let wheel = wheel_zone(ui, outer);
        let out = ScrollArea::vertical()
            .scroll_source(egui::scroll_area::ScrollSource { mouse_wheel: wheel, ..Default::default() })
            .id_salt(("chat_scroll", chat.id)).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0 * z;
            // a little room at the right so the red x on the corner of a file is not cut off
            ui.set_max_width(ui.available_width() - 6.0);
            let avail = ui.available_width();
            let max_w = (avail * 0.62).max(200.0);
            let top = ui.cursor().min.y;
            // Short conversations sit at the bottom, next to the text box, like in a messenger.
            let pad = (view - last).max(0.0);
            ui.add_space(pad + 14.0 * z);
            if notes.is_empty() {
                ui.vertical_centered(|ui| widgets::footnote(ui, "No messages yet. Type a note below, or drop files on the window."));
            }
            let now = now_secs();
            let today = crate::clock::local(now).map(|t| t.date_naive());
            let mut prev_day = None;
            for n in notes {
                // a line with the day above the first message of each day
                if n.ts > 0.0 {
                    if let Some(day) = crate::clock::local(n.ts).map(|t| t.date_naive()) {
                        if prev_day != Some(day) {
                            day_line(ui, &p, &crate::clock::day_label(day, today.unwrap_or(day)), avail, z);
                            prev_day = Some(day);
                        }
                    }
                }
                let own = n.src == "pc";
                // a width the user dragged to is the width of every message of that side (but never wider than the chat)
                let fixed = (if own { chat.widths.own } else { chat.widths.theirs }).map(|w| (w * z).clamp(MIN_BUBBLE_W * z, avail.max(MIN_BUBBLE_W * z)));
                ui.with_layout(Layout::top_down(if own { Align::Max } else { Align::Min }), |ui| {
                    let (a, w) = message(ui, &p, own, fixed.unwrap_or(max_w), fixed.is_some(), avail, n, now, z, chat.time_24h);
                    if a.is_some() {
                        action = a;
                    }
                    if let Some(w) = w {
                        width = Some((own, w));
                    }
                });
            }
            ui.add_space(14.0 * z);
            if chat.jump_to_end {
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }
            let h = ui.cursor().min.y - top - pad;
            if (h - last).abs() > 0.5 {
                ui.data_mut(|d| d.insert_temp(memo, h));
                ui.ctx().request_repaint();
            }
        });
        let overflow = (out.content_size.y - out.inner_rect.height()).max(0.0);
        edge_fades(ui, outer, out.state.clamped_offset().y, overflow - out.state.clamped_offset().y, p);
    });
    (action, width)
}

/// A thin line across the chat with the day in the middle.
fn day_line(ui: &mut Ui, p: &Palette, label: &str, width: f32, z: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 18.0 * z), Sense::hover());
    let text = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.0 * z), p.label2);
    let half = text.size().x / 2.0 + 10.0 * z;
    let (cx, cy) = (rect.center().x, rect.center().y);
    let stroke = Stroke::new(1.0, p.outline.gamma_multiply(0.45));
    ui.painter().hline(rect.min.x..=(cx - half).max(rect.min.x), cy, stroke);
    ui.painter().hline((cx + half).min(rect.max.x)..=rect.max.x, cy, stroke);
    ui.painter().galley(pos2(cx - text.size().x / 2.0, cy - text.size().y / 2.0), text, p.label2);
}

/// The small time at the bottom right of a message, if the message has one.
fn time_galley(painter: &egui::Painter, ts: f64, h24: bool, pending: bool, z: f32, color: Color32) -> Option<std::sync::Arc<egui::Galley>> {
    if pending {
        return Some(painter.layout_no_wrap("Waiting to send".to_owned(), egui::FontId::proportional(10.5 * z), color));
    }
    if ts <= 0.0 {
        return None;
    }
    let t = crate::clock::local(ts)?;
    Some(painter.layout_no_wrap(crate::clock::time_text(&t, h24), egui::FontId::proportional(10.5 * z), color))
}

/// Soft fade at the top (when scrolled down) and bottom (when more is below) of the transcript.
/// The color is the transcript background, so bubbles just dissolve into it. The top fade is cut
/// to the rounded top corners of the transcript instead of being a plain rectangle.
fn edge_fades(ui: &Ui, transcript: Rect, above: f32, below: f32, p: Palette) {
    const FADE: f32 = 22.0;
    const BORDER: f32 = 1.5;
    let painter = ui.painter();
    let (x0, x1) = (transcript.min.x + BORDER, transcript.max.x - BORDER);
    if above > 0.5 {
        let strength = (above / FADE).clamp(0.0, 1.0) * 0.95;
        let radius = CARD_RADIUS as f32 - BORDER;
        let top = transcript.min.y + BORDER;
        for i in 0..FADE as i32 {
            let y = i as f32;
            // how far the rounded corner reaches into this row
            let inset = if y < radius { radius - (radius * radius - (radius - y - 0.5).powi(2)).max(0.0).sqrt() } else { 0.0 };
            let alpha = strength * (1.0 - (y + 0.5) / FADE);
            let row = Rect::from_min_max(pos2(x0 + inset, top + y), pos2(x1 - inset, top + y + 1.0));
            painter.rect_filled(row, 0.0, p.chat.gamma_multiply(alpha));
        }
    }
    if below > 0.5 {
        let strength = (below / FADE).clamp(0.0, 1.0) * 0.95;
        let bottom = transcript.max.y;
        for i in 0..FADE as i32 {
            let y = i as f32;
            let alpha = strength * (1.0 - (y + 0.5) / FADE);
            let row = Rect::from_min_max(pos2(x0, bottom - y - 1.0), pos2(x1, bottom - y));
            painter.rect_filled(row, 0.0, p.chat.gamma_multiply(alpha));
        }
    }
}

fn corners_for(own: bool, z: f32) -> CornerRadius {
    let (big, small) = ((18.0 * z).round() as u8, (6.0 * z).round() as u8);
    if own { CornerRadius { nw: big, ne: big, sw: big, se: small } } else { CornerRadius { nw: big, ne: big, sw: small, se: big } }
}

/// Whether the drag that is going on over this message resizes it (Ctrl, on a Mac Cmd, was held when it started)
/// or marks text. Decided once, when the drag starts, so letting go of the key in the middle changes nothing.
fn drag_is_resize(ui: &Ui, resp: &egui::Response) -> bool {
    let id = resp.id.with("resize-drag");
    if resp.drag_started() {
        let held = ui.input(|i| i.modifiers.command);
        ui.data_mut(|d| d.insert_temp(id, held));
    }
    resp.dragged() && ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false)
}

/// Dragging a message sideways while Ctrl (Cmd on a Mac) is held changes the width of all messages of its side.
/// The free edge of the bubble (the left one for mine, the right one for the others) follows the pointer. Returns
/// the new width in points at normal size. With the key held, the pointer shows the resize cursor near that edge.
fn bubble_drag(ui: &Ui, resp: &egui::Response, own: bool, avail: f32, z: f32) -> Option<f32> {
    let resizing = drag_is_resize(ui, resp);
    let edge = if own { resp.rect.min.x } else { resp.rect.max.x };
    let key = ui.input(|i| i.modifiers.command);
    let near = key && ui.input(|i| i.pointer.hover_pos()).is_some_and(|pos| resp.rect.contains(pos) && (pos.x - edge).abs() < 10.0 * z);
    if near || resizing {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let dx = if resizing { resp.drag_delta().x } else { 0.0 };
    (dx != 0.0).then(|| ((resp.rect.width() + if own { -dx } else { dx }) / z).clamp(MIN_BUBBLE_W, (avail / z).max(MIN_BUBBLE_W)))
}

/// One message: the bubble plus whatever the mouse can do with it. `fixed`: the bubble is exactly `max_w` wide
/// (the user chose a width); otherwise it is as wide as its text, up to `max_w`. Returns what was clicked and
/// the new width when the bubble is being dragged.
#[allow(clippy::too_many_arguments)]
fn message(ui: &mut Ui, p: &Palette, own: bool, max_w: f32, fixed: bool, avail: f32, n: &Note, now: f64, z: f32, h24: bool) -> (Option<MsgAction>, Option<f32>) {
    let (fill, fg, fg2) = if own { (p.tint, p.on_tint, p.on_tint) } else { (p.bubble, p.label, p.label2) };
    if let (NoteKind::File, Some(f)) = (&n.kind, &n.file) {
        let picture = picture_bubble(ui, p, corners_for(own, z), own, max_w, z, f, n.ts, h24);
        let (click, resp) = match picture {
            Some(r) => r,
            None => file_bubble(ui, p, corners_for(own, z), fill, fg, fg2, own, max_w, fixed, z, f, n.ts, h24),
        };
        let action = click.map(|a| match a {
            FileClick::Open => MsgAction::Open(f.clone()),
            FileClick::Folder => MsgAction::ShowFolder(f.clone()),
            FileClick::Delete => MsgAction::Delete(n.mid.clone()),
        });
        return (action, bubble_drag(ui, &resp, own, avail, z));
    }
    let (resp, link) = text_bubble(ui, p, own, max_w, fixed, z, n, fill, fg, fg2, h24);
    let width = bubble_drag(ui, &resp, own, avail, z);
    let resp = if resp.dragged() { resp } else { resp.on_hover_text(crate::clock::local(n.ts).map(|t| crate::clock::full_text(&t, h24)).unwrap_or_else(|| ago(now - n.ts))) };
    let mut action = None;
    if let Some(url) = link {
        action = Some(MsgAction::OpenLink(url));
    } else if resp.double_clicked() {
        action = Some(MsgAction::Reply(n.mid.clone()));
    }
    // Reply, copy and delete appear beside the bubble while the mouse is on it.
    let buttons = [(Icon::Reply, "Reply (or double-click the message)", false), (Icon::Copy, "Copy the text", false), (Icon::Trash, "Delete for both", true)];
    if let Some(i) = toolbar(ui, p, resp.rect, own, &buttons, n.id, z) {
        action = Some(match i {
            0 => MsgAction::Reply(n.mid.clone()),
            1 => MsgAction::Copy(n.text.clone().unwrap_or_default()),
            _ => MsgAction::Delete(n.mid.clone()),
        });
    }
    (action, width)
}

/// A small pill of icon buttons next to a bubble, shown while the pointer is on the bubble or the pill.
/// Returns the index of the clicked button.
fn toolbar(ui: &mut Ui, p: &Palette, bubble: Rect, own: bool, buttons: &[(Icon, &str, bool)], salt: u64, z: f32) -> Option<usize> {
    let btn = 28.0 * z;
    let w = btn * buttons.len() as f32 + 6.0 * z;
    let y = bubble.min.y + 2.0 * z;
    let x = if own { bubble.min.x - 6.0 * z - w } else { bubble.max.x + 6.0 * z };
    let bar = Rect::from_min_size(pos2(x, y), vec2(w, btn + 4.0 * z));
    if !ui.rect_contains_pointer(bubble.union(bar).expand(2.0)) {
        return None;
    }
    ui.painter().rect(bar, CornerRadius::same((14.0 * z).round() as u8), p.card, Stroke::new(1.0, p.outline.gamma_multiply(0.6)), StrokeKind::Inside);
    let mut clicked = None;
    for (i, (icon, tip, danger)) in buttons.iter().enumerate() {
        let r = Rect::from_min_size(bar.min + vec2((3.0 * z) + btn * i as f32, 2.0 * z), vec2(btn, btn));
        let resp = ui.interact(r, theme::id(("msg_tool", salt, i)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(*tip);
        if resp.hovered() {
            ui.painter().circle_filled(r.center(), btn / 2.0 - 1.0, p.field_hover);
        }
        icons::draw(ui.painter(), *icon, r.center(), 16.0 * z, if *danger { p.red } else { p.label });
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// The marked part of one message (character positions in the text shown). Only one message has a selection.
#[derive(Clone)]
struct Selection {
    mid: String,
    from: usize,
    to: usize,
}

fn selection_id() -> egui::Id {
    crate::theme::id("chat_selection")
}

/// A text message, drawn by hand so the bubble is only as wide as its text and so a quote of the
/// message it answers can sit inside it. `z` is the size of the chat.
#[allow(clippy::too_many_arguments)]
fn text_bubble(ui: &mut Ui, p: &Palette, own: bool, max_w: f32, fixed: bool, z: f32, n: &Note, fill: Color32, fg: Color32, fg2: Color32, h24: bool) -> (egui::Response, Option<String>) {
    let inner_max = max_w - 28.0 * z;
    let shown: String = n.text.as_deref().unwrap_or("").chars().take(2000).collect();
    let mut body_font = TextStyle::Body.resolve(ui.style());
    body_font.size *= z;
    let painter = ui.painter().clone();
    let links = find_links(&shown);
    let body = painter.layout_job(body_job(&shown, &links, body_font, fg, if own { Color32::WHITE } else { p.tint_text }, inner_max));

    // the quote: who wrote it and its first line
    let quote = n.reply.as_ref().map(|r| {
        let who = if r.src == "pc" { "You".to_string() } else { r.src.clone() };
        let preview: String = r.preview.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = widgets::one_line(&painter, &who, 12.0 * z, fg, inner_max - 22.0 * z);
        let line = widgets::one_line(&painter, &preview, 12.0 * z, fg2, inner_max - 22.0 * z);
        let w = name.size().x.max(line.size().x) + 22.0 * z;
        let h = 5.0 * z + name.size().y + z + line.size().y + 5.0 * z;
        (name, line, w, h)
    });

    // the time sits at the bottom right, a little further right and lower than the text
    let time = time_galley(&painter, n.ts, h24, n.pending, z, if own { fg2.gamma_multiply(0.9) } else { fg2 });
    let time_w = time.as_ref().map_or(0.0, |t| t.size().x - 3.0 * z);
    let time_h = time.as_ref().map_or(0.0, |t| t.size().y + 1.0 * z);
    let bottom = if time.is_some() { 4.0 * z } else { 9.0 * z };
    let inner_w = if fixed { inner_max } else { body.size().x.max(quote.as_ref().map_or(0.0, |q| q.2)).max(time_w) };
    let quote_h = quote.as_ref().map_or(0.0, |q| q.3 + 6.0 * z);
    let (rect, resp) = ui.allocate_exact_size(vec2(inner_w + 28.0 * z, 9.0 * z + quote_h + body.size().y + time_h + bottom), Sense::click_and_drag());
    painter.rect_filled(rect, corners_for(own, z), fill);
    let mut y = rect.min.y + 9.0 * z;
    if let Some((name, line, _, h)) = quote {
        let q = Rect::from_min_size(pos2(rect.min.x + 14.0 * z, y), vec2(inner_w, h));
        painter.rect_filled(q, CornerRadius::same((8.0 * z).round() as u8), if own { Color32::from_white_alpha(40) } else { p.field });
        painter.rect_filled(Rect::from_min_size(q.min, vec2(3.0 * z, h)), CornerRadius::same((2.0 * z).round() as u8), if own { Color32::WHITE } else { p.tint });
        let ny = q.min.y + 5.0 * z;
        let line_y = ny + name.size().y + z;
        painter.galley(pos2(q.min.x + 11.0 * z, ny), name, fg);
        painter.galley(pos2(q.min.x + 11.0 * z, line_y), line, fg2);
        y += h + 6.0 * z;
    }
    let origin = pos2(rect.min.x + 14.0 * z, y);
    let sel_id = selection_id();
    let resizing = drag_is_resize(ui, &resp);
    // mark text by dragging over it (a drag with Ctrl / Cmd held resizes instead)
    let index_at = |pos: egui::Pos2| -> usize { body.cursor_from_pos(pos - origin).index.into() };
    if resp.drag_started() && !resizing {
        if let Some(start) = ui.input(|i| i.pointer.press_origin()) {
            let at = index_at(start);
            ui.data_mut(|d| d.insert_temp(sel_id, Selection { mid: n.mid.clone(), from: at, to: at }));
        }
    }
    if resp.dragged() && !resizing {
        if let (Some(pos), Some(mut sel)) = (ui.input(|i| i.pointer.interact_pos()), ui.data(|d| d.get_temp::<Selection>(sel_id))) {
            if sel.mid == n.mid {
                sel.to = index_at(pos);
                ui.data_mut(|d| d.insert_temp(sel_id, sel));
            }
        }
    }
    let selection = ui.data(|d| d.get_temp::<Selection>(sel_id)).filter(|sel| sel.mid == n.mid && sel.from != sel.to);
    let hovered_link = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|pos| resp.contains_pointer() && Rect::from_min_size(origin, body.size()).contains(*pos))
        .and_then(|pos| {
            let index: usize = body.cursor_from_pos(pos - origin).index.into();
            links.iter().find(|l| l.start <= index && index < l.end).map(|l| l.url.clone())
        });
    let mut galley = body;
    if let Some(sel) = &selection {
        let mut visuals = ui.visuals().clone();
        visuals.selection.bg_fill = if own { Color32::from_white_alpha(90) } else { p.tint.gamma_multiply(0.5) };
        visuals.selection.stroke.color = fg;
        let range = egui::text_selection::CCursorRange::two(egui::text::CCursor::new(sel.from), egui::text::CCursor::new(sel.to));
        egui::text_selection::visuals::paint_text_selection(&mut galley, &visuals, &range, None);
        // Ctrl+C / Cmd+C copies the marked text (unless the text box has the keyboard)
        if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) && !ui.ctx().egui_wants_keyboard_input() {
            let (lo, hi) = (sel.from.min(sel.to), sel.from.max(sel.to));
            ui.ctx().copy_text(shown.chars().skip(lo).take(hi - lo).collect());
        }
    }
    let over_body = resp.contains_pointer() && ui.input(|i| i.pointer.hover_pos()).is_some_and(|pos| Rect::from_min_size(origin, galley.size()).contains(pos));
    painter.galley(origin, galley, fg);
    if let Some(t) = time {
        painter.galley(pos2(rect.max.x - 11.0 * z - t.size().x, rect.max.y - 4.0 * z - t.size().y), t, fg2);
    }
    if hovered_link.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    } else if over_body && !ui.input(|i| i.modifiers.command) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    let clicked = if resp.clicked() { hovered_link } else { None };
    (resp, clicked)
}

/// The text of a message with its web addresses underlined in the link color.
fn body_job(text: &str, links: &[LinkSpan], font: egui::FontId, color: Color32, link_color: Color32, wrap: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let chars: Vec<char> = text.chars().collect();
    let part = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
    let mut at = 0;
    for l in links {
        if l.start > at {
            job.append(&part(at, l.start), 0.0, TextFormat::simple(font.clone(), color));
        }
        let mut format = TextFormat::simple(font.clone(), link_color);
        format.underline = Stroke::new(1.0, link_color);
        job.append(&part(l.start, l.end), 0.0, format);
        at = l.end;
    }
    if at < chars.len() {
        job.append(&part(at, chars.len()), 0.0, TextFormat::simple(font, color));
    }
    job
}

enum FileClick {
    Open,
    Folder,
    Delete,
}

/// A file: icon, name and size, drawn by hand so the bubble is only as wide as its text. Click opens it.
/// While the pointer is on it, a folder button and a red x (delete) appear. `z` is the size of the chat.
#[allow(clippy::too_many_arguments)]
fn file_bubble(ui: &mut Ui, p: &Palette, corners: CornerRadius, fill: Color32, fg: Color32, fg2: Color32, own: bool, max_w: f32, fixed: bool, z: f32, f: &FileRef, ts: f64, h24: bool) -> (Option<FileClick>, egui::Response) {
    let reserved = 40.0 * z; // room at the right for the folder button
    let sub = match f.dir {
        Direction::ToPc | Direction::FromPeer => format!("{} \u{b7} saved to the inbox", size(f.size)),
        Direction::ToPhone => format!("{} \u{b7} offered to the phone", size(f.size)),
        Direction::ToPeer => format!("{} \u{b7} sent", size(f.size)),
    };
    let text_max = (max_w - (14.0 + 36.0 + 10.0 + 40.0) * z).max(60.0 * z);
    let name = widgets::one_line(ui.painter(), &f.name, 14.0 * z, fg, text_max);
    let sub = widgets::one_line(ui.painter(), &sub, 12.0 * z, fg2, text_max);
    let text_w = name.size().x.max(sub.size().x);
    let time = time_galley(ui.painter(), ts, h24, false, z, if own { fg2.gamma_multiply(0.9) } else { fg2 });
    let time_w = time.as_ref().map_or(0.0, |t| t.size().x + 25.0 * z);
    let width = if fixed { max_w } else { ((14.0 + 36.0 + 10.0) * z + text_w + reserved).max(time_w) };
    let height = if time.is_some() { (36.0 + 18.0 + 10.0) * z } else { (36.0 + 18.0) * z };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let openable = f.local_path.is_some();
    let resp = if openable && !resp.dragged() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    let painter = ui.painter().clone();
    painter.rect_filled(rect, corners, fill);
    let icon = Rect::from_min_size(rect.min + vec2(14.0 * z, 9.0 * z), vec2(36.0 * z, 36.0 * z));
    painter.rect_filled(icon, CornerRadius::same((FIELD_RADIUS as f32 * z).round() as u8), if own { Color32::from_white_alpha(56) } else { p.field });
    icons::draw(&painter, Icon::File, icon.center(), 18.0 * z, fg);
    let x = icon.max.x + 10.0 * z;
    painter.galley(pos2(x, rect.min.y + 10.0 * z), name, fg);
    painter.galley(pos2(x, rect.min.y + 29.0 * z), sub, fg2);
    if let Some(t) = time {
        painter.galley(pos2(rect.max.x - 11.0 * z - t.size().x, rect.max.y - 4.0 * z - t.size().y), t, fg2);
    }
    let hover_fill = if own { Color32::from_white_alpha(56) } else { p.field };
    let (action, _) = file_buttons(ui, &painter, p, rect, &resp, z, f, openable, fg, hover_fill);
    (action, resp)
}

/// The folder button and the red x that appear while the pointer is on a file or picture, and the click that
/// opens it. Returns what was clicked, and whether the pointer is on one of the two buttons.
#[allow(clippy::too_many_arguments)]
fn file_buttons(ui: &mut Ui, painter: &egui::Painter, p: &Palette, rect: Rect, resp: &egui::Response, z: f32, f: &FileRef, openable: bool, icon_color: Color32, hover_fill: Color32) -> (Option<FileClick>, bool) {
    // The two small buttons sit on top of the bubble. Their places are known up front so that the tooltip
    // of the bubble ("Open") is only shown where the pointer is not on one of them.
    let folder_r = Rect::from_center_size(pos2(rect.max.x - 24.0 * z, rect.min.y + 27.0 * z), vec2(28.0 * z, 28.0 * z));
    // The red x sits on the rounded top right corner of the bubble, half inside and half outside.
    let delete_r = Rect::from_center_size(pos2(rect.max.x - 5.3 * z, rect.min.y + 5.3 * z), vec2(20.0 * z, 20.0 * z));
    let on_buttons = ui.input(|i| i.pointer.hover_pos()).is_some_and(|pos| delete_r.contains(pos) || (openable && folder_r.contains(pos)));
    let mut action = if resp.clicked() && openable && !on_buttons { Some(FileClick::Open) } else { None };
    if ui.rect_contains_pointer(rect) || ui.rect_contains_pointer(delete_r) {
        let id = f.id.as_str();
        if openable {
            let r = folder_r;
            let b = ui.interact(r, theme::id(("file_folder", id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Show in folder");
            if b.hovered() {
                painter.circle_filled(r.center(), 14.0 * z, hover_fill);
            }
            icons::draw(painter, Icon::Folder, r.center(), 17.0 * z, icon_color);
            if b.clicked() {
                action = Some(FileClick::Folder);
            }
        }
        let tip = match f.dir {
            Direction::ToPc | Direction::FromPeer => "Delete the file (it is removed from the inbox) and the message",
            Direction::ToPeer => "Remove the message (your original file stays where it is)",
            Direction::ToPhone => "Withdraw the file (your original stays where it is)",
        };
        let r = delete_r;
        let b = ui.interact(r, theme::id(("file_delete", id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
        painter.circle_filled(r.center(), 9.0 * z, if b.hovered() { Color32::from_rgb(0xB0, 0x00, 0x14) } else { p.red.linear_multiply(1.0) });
        painter.circle_stroke(r.center(), 9.0 * z, Stroke::new(1.5, p.chat));
        icons::draw(painter, Icon::Close, r.center(), 10.0 * z, Color32::WHITE);
        if b.clicked() {
            action = Some(FileClick::Delete);
        }
    }
    if openable && !on_buttons {
        resp.clone().on_hover_text("Open");
    }
    (action, on_buttons)
}

/// A picture file, shown as the picture itself (a little blurred while only the preview is there, for example
/// when the file was removed from the inbox). `None` when the file is not a picture or there is nothing to show.
#[allow(clippy::too_many_arguments)]
fn picture_bubble(ui: &mut Ui, p: &Palette, corners: CornerRadius, own: bool, max_w: f32, z: f32, f: &FileRef, ts: f64, h24: bool) -> Option<(Option<FileClick>, egui::Response)> {
    if !bridge_core::thumb::is_picture_name(&f.name) {
        return None;
    }
    let path = f.local_path.as_deref().map(std::path::Path::new).filter(|p| p.is_file());
    if path.is_none() && f.thumb.is_none() {
        return None;
    }
    if path.is_some() && f.thumb.is_none() && crate::pictures::failed(ui.ctx(), &f.id) {
        return None; // cannot be shown as a picture: it is an ordinary file bubble then
    }
    let known = crate::pictures::known_aspect(ui.ctx(), &f.id).unwrap_or(vec2(4.0, 3.0));
    let edge = (260.0 * z).min(max_w);
    let fit = |a: Vec2| {
        let s = edge / a.x.max(a.y).max(1.0);
        vec2((a.x * s).max(70.0 * z), (a.y * s).max(48.0 * z))
    };
    // only pictures that are on screen are decoded
    let visible = ui.is_rect_visible(Rect::from_min_size(ui.cursor().min, fit(known)));
    let pic = crate::pictures::show(ui.ctx(), &f.id, path, f.thumb.as_deref(), visible);
    let size = fit(pic.as_ref().map_or(known, |x| x.aspect));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let openable = path.is_some();
    let resp = if openable && !resp.dragged() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    let painter = ui.painter().clone();
    match &pic {
        Some(pic) => {
            egui::Image::new(egui::load::SizedTexture::new(pic.id, size)).corner_radius(corners).paint_at(ui, rect);
            if pic.blurred {
                painter.rect_filled(rect, corners, Color32::from_white_alpha(30));
            }
        }
        None => {
            painter.rect_filled(rect, corners, if own { p.tint } else { p.bubble });
            icons::draw(&painter, Icon::File, rect.center(), 22.0 * z, if own { p.on_tint } else { p.label2 });
        }
    }
    if let Some(t) = time_galley(&painter, ts, h24, false, z, Color32::WHITE) {
        let pill = Rect::from_min_size(pos2(rect.max.x - 8.0 * z - t.size().x - 12.0 * z, rect.max.y - 6.0 * z - t.size().y - 6.0 * z), t.size() + vec2(12.0 * z, 6.0 * z));
        painter.rect_filled(pill, CornerRadius::same((9.0 * z).round() as u8), Color32::from_black_alpha(120));
        painter.galley(pill.min + vec2(6.0 * z, 3.0 * z), t, Color32::WHITE);
    }
    let (action, _) = file_buttons(ui, &painter, p, rect, &resp, z, f, openable, Color32::WHITE, Color32::from_black_alpha(130));
    Some((action, resp))
}

fn composer(ui: &mut Ui, area: Rect, chat: Chat, p: Palette) -> (Compose, bool) {
    let mut action = Compose::Nothing;
    let mut clear_reply = false;
    // Fixed rectangles: the "replying to" strip, the text box, and the buttons under it.
    let strip_h = if chat.reply.is_some() { REPLY_STRIP } else { 0.0 };
    let field = Rect::from_min_size(area.min + vec2(0.0, strip_h), vec2(area.width(), 64.0));
    let buttons = Rect::from_min_size(pos2(area.min.x, field.max.y + 8.0), vec2(area.width(), 32.0));

    if let Some(n) = chat.reply {
        let strip = Rect::from_min_size(area.min, vec2(area.width(), 28.0));
        ui.painter().rect_filled(strip, CornerRadius::same(FIELD_RADIUS), p.field);
        ui.painter().rect_filled(Rect::from_min_size(strip.min, vec2(3.0, strip.height())), CornerRadius::same(2), p.tint);
        let who = if n.src == "pc" { "yourself".to_string() } else { n.src.clone() };
        let preview = match (&n.text, &n.file) {
            (Some(t), _) => t.split_whitespace().collect::<Vec<_>>().join(" "),
            (None, Some(f)) => f.name.clone(),
            _ => String::new(),
        };
        let label = widgets::one_line(ui.painter(), &format!("Replying to {who}:  {preview}"), 12.0, p.label2, strip.width() - 52.0);
        ui.painter().galley(pos2(strip.min.x + 12.0, strip.center().y - label.size().y / 2.0), label, p.label2);
        let x = Rect::from_center_size(pos2(strip.max.x - 16.0, strip.center().y), vec2(24.0, 24.0));
        let r = ui.interact(x, theme::id("reply_clear"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Do not reply (Esc)");
        if r.hovered() {
            ui.painter().circle_filled(x.center(), 11.0, p.field_hover);
        }
        icons::draw(ui.painter(), Icon::Close, x.center(), 11.0, p.label2);
        clear_reply = r.clicked();
    }

    ui.painter().rect_filled(field, CornerRadius::same(FIELD_RADIUS), p.field);

    // The send key is taken out of the input before the text box sees it, so it never types a new line.
    let mut key_send = false;
    if ui.memory(|m| m.has_focus(composer_id())) {
        key_send = ui.input_mut(|i| {
            let at = i.events.iter().position(
                |e| matches!(e, egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat: false, modifiers, .. } if chat.send_key.matches(modifiers)),
            );
            at.map(|n| i.events.remove(n)).is_some()
        });
    }
    ui.scope_builder(UiBuilder::new().max_rect(field.shrink2(vec2(12.0, 10.0))), |ui| {
        ScrollArea::vertical().id_salt("composer_scroll").auto_shrink([false, false]).show(ui, |ui| {
            // the text you type follows the size of the chat too
            if let Some(body) = ui.style_mut().text_styles.get_mut(&TextStyle::Body) {
                body.size *= chat.zoom;
            }
            let edit = ui.add(
                egui::TextEdit::multiline(chat.text)
                    .id(composer_id())
                    .frame(Frame::NONE)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(chat.hint),
            );
            if edit.has_focus() && chat.reply.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                clear_reply = true;
            }
        });
    });

    ui.scope_builder(UiBuilder::new().max_rect(buttons).layout(Layout::left_to_right(Align::Center)), |ui| {
        let can_send = !chat.text.trim().is_empty();
        if widgets::secondary(ui, "Files").on_hover_text("Choose files to send. You can also drop them on the window.").clicked() {
            action = Compose::PickFiles;
        }
        if chat.clipboard && widgets::secondary(ui, "Clipboard").on_hover_text("Send the text you copied").clicked() {
            action = Compose::SendClipboard;
        }
        widgets::trailing(ui, |ui| {
            if widgets::primary_enabled(ui, can_send, "Send").on_hover_text(chat.send_key.label()).clicked() || (key_send && can_send) {
                action = Compose::SendText;
            }
            ui.add_space(6.0);
            ui.label(RichText::new("or drop files anywhere").small().color(p.label2));
        });
    });
    (action, clear_reply)
}

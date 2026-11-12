//! The large view of a picture, inside the window: a click on a picture in the chat opens it here instead of in
//! the system's picture viewer. Mouse wheel (or pinch) zooms around the pointer, dragging moves a zoomed picture,
//! a double-click switches between "fit" and the real size, ← and → go to the other pictures of the conversation,
//! Esc or a click beside the picture closes it.

use bridge_core::FileRef;
use egui::{pos2, vec2, Align2, Color32, Context, FontId, Rect, Sense, Stroke, Vec2};

use crate::icons::{self, Icon};

/// Most a picture can be enlarged, relative to fitting the window (or to its real size, whichever is more).
const MAX_ZOOM: f32 = 8.0;

/// Keys and wheel for the viewer, taken out of the input before the page below is drawn (the page would scroll
/// with the wheel, and the text box would see Esc and the arrows).
#[derive(Default, Clone, Copy)]
pub struct Input {
    pub wheel: f32,
    pub esc: bool,
    pub left: bool,
    pub right: bool,
}

impl Input {
    pub fn take(ctx: &Context) -> Self {
        ctx.input_mut(|i| {
            let wheel = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta = Vec2::ZERO;
            Input {
                wheel,
                esc: i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                left: i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft),
                right: i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight),
            }
        })
    }
}

pub struct Viewer {
    /// The pictures of the conversation that are on this PC, oldest first.
    pics: Vec<FileRef>,
    index: usize,
    /// 1.0: the picture fits the window (or is shown at its real size if it is smaller).
    zoom: f32,
    /// Where the middle of the picture is, from the middle of the free area.
    offset: Vec2,
    first_frame: bool,
    /// When the view opened and (once asked to) when it began to close: it fades and grows in, and fades and shrinks out.
    opened: Option<f64>,
    closing: Option<f64>,
}

pub enum ViewerAction {
    Close,
    /// Open with the system's program.
    Open(FileRef),
    Folder(FileRef),
}

impl Viewer {
    /// `None` when `current` is not among `pics`.
    pub fn new(pics: Vec<FileRef>, current: &str) -> Option<Self> {
        let index = pics.iter().position(|f| f.id == current)?;
        Some(Viewer { pics, index, zoom: 1.0, offset: Vec2::ZERO, first_frame: true, opened: None, closing: None })
    }

    fn go(&mut self, step: isize) {
        let n = self.pics.len() as isize;
        let next = self.index as isize + step;
        if (0..n).contains(&next) {
            self.index = next as usize;
            self.zoom = 1.0;
            self.offset = Vec2::ZERO;
        }
    }

    pub fn show(&mut self, ctx: &Context, input: Input) -> Option<ViewerAction> {
        let screen = ctx.content_rect();
        let f = self.pics[self.index].clone();
        let path = std::path::PathBuf::from(f.local_path.clone().unwrap_or_default());
        let ahead: Vec<(String, std::path::PathBuf)> = [self.index.checked_sub(1), Some(self.index + 1)]
            .into_iter()
            .flatten()
            .filter_map(|i| self.pics.get(i))
            .map(|g| (g.id.clone(), std::path::PathBuf::from(g.local_path.clone().unwrap_or_default())))
            .collect();
        let ahead_refs: Vec<(&str, &std::path::Path)> = ahead.iter().map(|(i, p)| (i.as_str(), p.as_path())).collect();
        let large = crate::pictures::large(ctx, &f.id, &path, &ahead_refs);
        // until the large one is decoded, the chat's picture (or its preview) stands in
        let small = if large.is_none() { crate::pictures::show(ctx, &f.id, Some(&path), f.thumb.as_deref(), true) } else { None };
        let failed = large.is_none() && crate::pictures::large_failed(ctx, &f.id);

        // opening and closing: `appear` goes from 0 to 1 and, when closing, back to 0
        const FADE: f64 = 0.18;
        let now = ctx.input(|i| i.time);
        let opened = *self.opened.get_or_insert(now);
        let appear = match self.closing {
            Some(since) => 1.0 - ((now - since) / FADE).clamp(0.0, 1.0),
            None => ((now - opened) / FADE).clamp(0.0, 1.0),
        } as f32;
        let appear = 1.0 - (1.0 - appear) * (1.0 - appear); // ease out
        if self.closing.is_some() && appear <= 0.0 {
            return Some(ViewerAction::Close);
        }
        let animating = self.closing.is_some() || appear < 1.0;
        if animating {
            ctx.request_repaint();
        }
        let input = if self.closing.is_some() { Input::default() } else { input };

        let mut action = None;
        egui::Area::new(crate::theme::id("picture_viewer"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_clip_rect(screen);
                ui.set_opacity(appear);
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
                if self.first_frame {
                    resp.request_focus();
                    self.first_frame = false;
                }
                // arrows and Esc are for the viewer, not for moving the keyboard focus elsewhere
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(resp.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
                });
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(232));

                let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 56.0));
                let area = Rect::from_min_max(pos2(rect.min.x + 24.0, bar.max.y + 4.0), pos2(rect.max.x - 24.0, rect.max.y - 34.0));
                let ppp = ctx.pixels_per_point();

                // the size of the picture at zoom 1: fitted into the free area, never larger than its real size
                let (tex, px) = match (&large, &small) {
                    (Some((t, px)), _) => (Some(*t), Some(*px)),
                    (None, Some(pic)) => (Some(pic.id), Some(pic.aspect)),
                    _ => (None, None),
                };
                let real = px.map(|px| px / ppp);
                let base = real.map(|real| {
                    let fit = (area.width() / real.x).min(area.height() / real.y);
                    // the chat's picture is at most 640 pixels: until the large one is there it is just fitted
                    let exact = large.is_some() || px.is_some_and(|p| p.x.max(p.y) < 640.0);
                    if exact { real * fit.min(1.0) } else { real * fit }
                });
                // how much "real size" is, relative to zoom 1
                let one_to_one = match (base, real) {
                    (Some(b), Some(r)) if large.is_some() && b.x > 0.0 => (r.x / b.x).max(1.0),
                    _ => 1.0,
                };
                let max_zoom = MAX_ZOOM.max(one_to_one * 2.0);

                // zoom with the wheel or pinch, around the pointer
                let hover = resp.hover_pos();
                let pinch = ui.input(|i| i.zoom_delta());
                let factor = (input.wheel / 200.0).exp() * pinch;
                if hover.is_some() && (factor - 1.0).abs() > 1e-4 {
                    let at = hover.unwrap_or(area.center());
                    self.zoom_to((self.zoom * factor).clamp(1.0, max_zoom), at, area);
                }
                if resp.double_clicked() {
                    let at = hover.unwrap_or(area.center());
                    let target = if self.zoom > 1.01 { 1.0 } else if one_to_one > 1.05 { one_to_one } else { 2.5 };
                    self.zoom_to(target, at, area);
                }
                if resp.dragged() && self.zoom > 1.0 {
                    self.offset += resp.drag_delta();
                }

                if let Some(base) = base {
                    let shown = base * self.zoom;
                    let room = ((shown - area.size()) / 2.0).max(Vec2::ZERO);
                    self.offset = self.offset.clamp(-room, room);
                    let pic_rect = Rect::from_center_size(area.center() + self.offset, shown * (0.94 + 0.06 * appear));
                    if let Some(tex) = tex {
                        let clip = ui.painter_at(area.expand2(vec2(24.0, 0.0)).intersect(rect));
                        clip.image(tex, pic_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    if self.zoom > 1.0 {
                        if resp.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        } else if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                        }
                    }
                    // a click beside the picture closes the view
                    if resp.clicked() && hover.is_some_and(|h| !pic_rect.contains(h) && !bar.contains(h)) {
                        action = Some(ViewerAction::Close);
                    }
                } else if failed {
                    painter.text(area.center(), Align2::CENTER_CENTER, "This picture cannot be shown here.", FontId::proportional(16.0), Color32::from_white_alpha(220));
                    if resp.clicked() {
                        action = Some(ViewerAction::Close);
                    }
                } else {
                    painter.text(area.center(), Align2::CENTER_CENTER, "Loading...", FontId::proportional(16.0), Color32::from_white_alpha(180));
                }
                if large.is_none() && !failed {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                }

                // top bar: name, position in the conversation, size; Open, Show in folder, Close
                painter.rect_filled(bar, 0.0, Color32::from_black_alpha(120));
                let mut info = crate::common::size(f.size);
                if let (Some((_, px)), true) = (&large, one_to_one >= 1.0) {
                    info = format!("{} × {} · {info}", px.x as u32, px.y as u32);
                }
                if self.pics.len() > 1 {
                    info = format!("{} of {} · {info}", self.index + 1, self.pics.len());
                }
                let name_w = (bar.width() - 300.0).max(80.0);
                let name = ui.painter().layout(f.name.clone(), FontId::proportional(15.0), Color32::WHITE, name_w);
                painter.galley(pos2(bar.min.x + 20.0, bar.min.y + 9.0), name, Color32::WHITE);
                painter.text(pos2(bar.min.x + 20.0, bar.max.y - 9.0), Align2::LEFT_BOTTOM, info, FontId::proportional(12.0), Color32::from_white_alpha(170));

                let mut x = bar.max.x - 30.0;
                let button = |ui: &mut egui::Ui, x: f32, salt: &str, tip: &str| {
                    let r = Rect::from_center_size(pos2(x, bar.center().y), vec2(34.0, 34.0));
                    let b = ui.interact(r, crate::theme::id(("viewer_button", salt)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
                    if b.hovered() {
                        ui.painter().circle_filled(r.center(), 17.0, Color32::from_white_alpha(40));
                    }
                    (r, b.clicked())
                };
                let (r, close) = button(ui, x, "close", "Close (Esc)");
                icons::draw(ui.painter(), Icon::Close, r.center(), 16.0, Color32::WHITE);
                x -= 42.0;
                let (r, folder) = button(ui, x, "folder", "Show in folder");
                icons::draw(ui.painter(), Icon::Folder, r.center(), 18.0, Color32::WHITE);
                x -= 42.0;
                let (r, open) = button(ui, x, "open", "Open with the system's viewer");
                icons::draw(ui.painter(), Icon::File, r.center(), 18.0, Color32::WHITE);
                if close {
                    action = Some(ViewerAction::Close);
                } else if folder {
                    action = Some(ViewerAction::Folder(f.clone()));
                } else if open {
                    action = Some(ViewerAction::Open(f.clone()));
                }

                // the other pictures of the conversation
                if self.pics.len() > 1 {
                    for (step, cx) in [(-1isize, rect.min.x + 34.0), (1, rect.max.x - 34.0)] {
                        let target = self.index as isize + step;
                        if !(0..self.pics.len() as isize).contains(&target) {
                            continue;
                        }
                        let r = Rect::from_center_size(pos2(cx, area.center().y), vec2(44.0, 44.0));
                        let b = ui.interact(r, crate::theme::id(("viewer_step", step)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                        let fill = if b.hovered() { Color32::from_black_alpha(200) } else { Color32::from_black_alpha(130) };
                        ui.painter().circle(r.center(), 22.0, fill, Stroke::new(1.0, Color32::from_white_alpha(60)));
                        let c = r.center();
                        let d = step as f32 * 5.0;
                        let stroke = Stroke::new(2.2, Color32::WHITE);
                        ui.painter().line_segment([c + vec2(-d, -9.0), c + vec2(d, 0.0)], stroke);
                        ui.painter().line_segment([c + vec2(d, 0.0), c + vec2(-d, 9.0)], stroke);
                        if b.clicked() {
                            self.go(step);
                        }
                    }
                }

                let hint = if self.pics.len() > 1 { "Scroll to zoom · drag to move · double-click: real size · ← → other pictures · Esc closes" } else { "Scroll to zoom · drag to move · double-click: real size · Esc closes" };
                painter.text(pos2(rect.center().x, rect.max.y - 12.0), Align2::CENTER_BOTTOM, hint, FontId::proportional(12.0), Color32::from_white_alpha(120));

                if input.esc {
                    action = Some(ViewerAction::Close);
                } else if input.left {
                    self.go(-1);
                } else if input.right {
                    self.go(1);
                }
            });
        if self.closing.is_some() {
            return None; // nothing else is done while the view fades out
        }
        if matches!(action, Some(ViewerAction::Close)) {
            self.closing = Some(now);
            ctx.request_repaint();
            return None;
        }
        action
    }

    /// Change the zoom so that the point under `at` stays where it is.
    fn zoom_to(&mut self, zoom: f32, at: egui::Pos2, area: Rect) {
        let from_center = at - area.center();
        self.offset = from_center - (from_center - self.offset) * (zoom / self.zoom);
        self.zoom = zoom;
        if zoom <= 1.0 {
            self.offset = Vec2::ZERO;
        }
    }
}

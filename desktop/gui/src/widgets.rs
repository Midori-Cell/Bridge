//! Small building blocks that give every screen the same look: cards with inset rows, three kinds of
//! buttons, a status label, section titles, and the modal sheet.

use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, Response, RichText, Sense, Stroke, Ui, Vec2};

use crate::theme::{palette, CAPSULE, CARD_RADIUS, CONTROL_HEIGHT};

/// Page title (Title style).
pub fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).heading().strong());
}

/// Explanatory text under a group: smaller and quieter, still readable.
pub fn footnote(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().color(palette(ui).label2)).wrap());
}

/// Small caption above a group of rows.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(text).small().strong().color(palette(ui).label2));
    });
}

/// Caption at the very top of a page: no extra space above it.
pub fn section_first(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(text).small().strong().color(palette(ui).label2));
    });
}

/// Rows grouped inside one rounded card, separated by hairlines that start after the left padding.
pub struct Rows<'a> {
    ui: &'a mut Ui,
    first: bool,
}

pub fn grouped<R>(ui: &mut Ui, add: impl FnOnce(&mut Rows) -> R) -> R {
    let fill = palette(ui).card;
    grouped_with(ui, fill, add)
}

/// Same, with another fill (inside a sheet the card color is already taken by the sheet itself).
pub fn grouped_with<R>(ui: &mut Ui, fill: Color32, add: impl FnOnce(&mut Rows) -> R) -> R {
    let frame = Frame::new().fill(fill).corner_radius(CornerRadius::same(CARD_RADIUS));
    frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(&mut Rows { ui, first: true })
        })
        .inner
}

impl Rows<'_> {
    pub fn row<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        let p = palette(self.ui);
        if !self.first {
            let (rect, _) = self.ui.allocate_exact_size(Vec2::new(self.ui.available_width(), 1.0), Sense::hover());
            let inset = 14.0;
            self.ui.painter().hline(rect.min.x + inset..=rect.max.x, rect.center().y, Stroke::new(1.0, p.separator));
        }
        self.first = false;
        Frame::new()
            .inner_margin(Margin::symmetric(14, 10))
            .show(self.ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                add(ui)
            })
            .inner
    }
}

fn capsule() -> CornerRadius {
    CornerRadius::same(CAPSULE)
}

/// The one main action of a screen or sheet.
pub fn primary(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(
        egui::Button::new(RichText::new(text).color(p.on_tint).strong())
            .fill(p.tint)
            .corner_radius(capsule())
            .min_size(Vec2::new(0.0, CONTROL_HEIGHT)),
    )
}

pub fn primary_enabled(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    ui.add_enabled_ui(enabled, |ui| primary(ui, text)).inner
}

/// Everything else: quiet filled capsule with normal text.
pub fn secondary(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(text).corner_radius(capsule()).min_size(Vec2::new(0.0, CONTROL_HEIGHT)))
}

/// Quiet capsule with a given fill, for buttons that sit on a field-colored card inside a sheet.
pub fn secondary_on(ui: &mut Ui, fill: Color32, text: &str) -> Response {
    ui.add(egui::Button::new(text).fill(fill).corner_radius(capsule()).min_size(Vec2::new(0.0, CONTROL_HEIGHT)))
}

/// Destructive action: red text, never the primary fill.
pub fn destructive(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(egui::Button::new(RichText::new(text).color(p.red)).corner_radius(capsule()).min_size(Vec2::new(0.0, CONTROL_HEIGHT)))
}

/// Text-only action inside a row.
pub fn plain(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(egui::Button::new(RichText::new(text).color(p.tint_text)).frame(false).min_size(Vec2::new(0.0, CONTROL_HEIGHT)))
}

/// Text-only destructive action inside a row.
pub fn plain_danger(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(egui::Button::new(RichText::new(text).color(p.red)).frame(false).min_size(Vec2::new(0.0, CONTROL_HEIGHT)))
}

/// Online or not. Shape and word say it, color only adds to it.
pub fn status(ui: &mut Ui, online: bool, text: &str) {
    let p = palette(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
    if online {
        ui.painter().circle_filled(rect.center(), 4.5, p.green);
    } else {
        ui.painter().circle_stroke(rect.center(), 4.0, Stroke::new(1.5, p.label2));
    }
    ui.label(RichText::new(text).small().color(if online { p.green } else { p.label2 }));
}

/// Right-aligned group of controls inside a row.
pub fn trailing<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::right_to_left(Align::Center), add).inner
}

pub enum SheetResult {
    Open,
    /// Escape was pressed: treat like the visible Cancel button.
    Dismissed,
}

/// A modal sheet: dims everything behind it, has a title and its own buttons.
/// Clicking the dim layer does nothing (the sheet asks for a decision), Escape cancels.
pub fn sheet(ctx: &egui::Context, id: &str, title_text: &str, add: impl FnOnce(&mut Ui)) -> SheetResult {
    let p = crate::theme::palette_of(ctx);
    let frame = Frame::new()
        .fill(p.card)
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(22))
        .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(90) });
    let modal = egui::Modal::new(crate::theme::id(id)).frame(frame).backdrop_color(Color32::from_black_alpha(if p.card == crate::theme::DARK.card { 170 } else { 110 }));
    let resp = modal.show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(RichText::new(title_text).size(18.0).strong());
        ui.add_space(4.0);
        add(ui);
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && resp.should_close() {
        SheetResult::Dismissed
    } else {
        SheetResult::Open
    }
}

/// Buttons at the bottom of a sheet: cancel on the left of the primary action.
pub fn sheet_buttons(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        add(ui);
    });
}

/// On/off switch. Off shows an outline, on is filled with the tint, so the state is not only a color.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let p = palette(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(42.0, 24.0), Sense::click());
    let mut resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let (track, knob, outline) = if *on { (p.tint, Color32::WHITE, Stroke::NONE) } else { (p.field, p.label2, Stroke::new(1.5, p.label2)) };
    let r = rect.height() / 2.0;
    ui.painter().rect(rect, CornerRadius::same(r as u8), track, outline, egui::StrokeKind::Inside);
    let x = if *on { rect.max.x - r } else { rect.min.x + r };
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), r - 4.0, knob);
    resp
}

/// A text line that is cut with "..." instead of wrapping.
pub fn one_line(painter: &egui::Painter, text: &str, size: f32, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), egui::FontId::proportional(size), color, width.max(1.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    painter.layout_job(job)
}

/// A small rounded square with an icon in it, to tell kinds of rows apart (text or file).
pub fn icon_badge(ui: &mut Ui, icon: crate::icons::Icon) {
    let p = palette(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(crate::theme::FIELD_RADIUS), p.field);
    crate::icons::draw(ui.painter(), icon, rect.center(), 16.0, p.label2);
}

/// A round color choice. The chosen one gets a ring, so it is not shown by color alone.
pub fn swatch(ui: &mut Ui, color: Color32, selected: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
    let p = palette(ui);
    let c = rect.center();
    ui.painter().circle_filled(c, if resp.hovered() { 12.0 } else { 11.0 }, color);
    if selected {
        ui.painter().circle_stroke(c, 14.0, Stroke::new(2.0, p.label));
    }
    resp
}

/// The "own color" choice: a ring of hues, or the chosen color with a ring when it is the one in use.
pub fn swatch_custom(ui: &mut Ui, chosen: Option<Color32>, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
    let p = palette(ui);
    let c = rect.center();
    let r = if resp.hovered() { 12.0 } else { 11.0 };
    match chosen {
        Some(color) => {
            ui.painter().circle_filled(c, r, color);
            ui.painter().circle_stroke(c, 14.0, Stroke::new(2.0, p.label));
        }
        None => {
            const WEDGES: usize = 12;
            for i in 0..WEDGES {
                let fill: Color32 = egui::epaint::Hsva::new(i as f32 / WEDGES as f32, 0.75, 0.92, 1.0).into();
                let mut pts = vec![c];
                for k in 0..=4 {
                    let a = ((i as f32 + k as f32 / 4.0) / WEDGES as f32) * std::f32::consts::TAU;
                    pts.push(c + Vec2::angled(a) * r);
                }
                ui.painter().add(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
            }
        }
    }
    resp
}

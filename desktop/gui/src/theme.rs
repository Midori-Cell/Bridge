//! Colors, type sizes and spacing. Follows the rules written down in docs/DESIGN.md:
//! one tint color for the main action, semantic colors that work in light and dark, base and
//! elevated backgrounds, and a few clear text sizes. Light or dark follows the system.

use egui::{Color32, Context, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Theme, Visuals};

#[derive(Clone, Copy)]
pub struct Palette {
    /// Base background of the content area.
    pub bg: Color32,
    /// Elevated background: cards and sheets sit on top of the base.
    pub card: Color32,
    pub sidebar: Color32,
    /// Fill of text fields and quiet buttons.
    pub field: Color32,
    pub field_hover: Color32,
    pub label: Color32,
    /// Secondary label: still readable (>= 4.5:1), just quieter.
    pub label2: Color32,
    pub separator: Color32,
    /// Fill of the one primary button and the selected sidebar item.
    pub tint: Color32,
    /// Tint as text color on a base or elevated background.
    pub tint_text: Color32,
    pub on_tint: Color32,
    pub red: Color32,
    pub green: Color32,
    pub orange: Color32,
    /// Background of the chat transcript (same as the text fields).
    pub chat: Color32,
    /// Received message bubbles.
    pub bubble: Color32,
    /// Border around the chat transcript.
    pub outline: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const LIGHT: Palette = Palette {
    bg: rgb(0xF2F2F7),
    card: rgb(0xFFFFFF),
    sidebar: rgb(0xE6E6EC),
    field: rgb(0xEFEFF4),
    field_hover: rgb(0xE3E3E9),
    label: rgb(0x1C1C1E),
    label2: rgb(0x606064),
    separator: rgb(0xE2E2E7),
    tint: rgb(0x0A6CE8),
    tint_text: rgb(0x0A6CE8),
    on_tint: rgb(0xFFFFFF),
    red: rgb(0xD70015),
    green: rgb(0x1F7A36),
    orange: rgb(0xB25000),
    chat: rgb(0xEFEFF4),
    bubble: rgb(0xFFFFFF),
    outline: rgb(0x9A9AA1),
};

pub const DARK: Palette = Palette {
    bg: rgb(0x0E0E10),
    card: rgb(0x1C1C1E),
    sidebar: rgb(0x161618),
    field: rgb(0x2C2C2E),
    field_hover: rgb(0x3A3A3C),
    label: rgb(0xF2F2F7),
    label2: rgb(0xA1A1A6),
    separator: rgb(0x38383A),
    tint: rgb(0x0A6CE8),
    tint_text: rgb(0x4DA3FF),
    on_tint: rgb(0xFFFFFF),
    red: rgb(0xFF6B6B),
    green: rgb(0x4CD27A),
    orange: rgb(0xFFB340),
    chat: rgb(0x2C2C2E),
    bubble: rgb(0x1C1C1E),
    outline: rgb(0x636366),
};

/// Concentric corners: a card (12) holds rows and fields (8), buttons are capsules.
pub const CARD_RADIUS: u8 = 12;
pub const FIELD_RADIUS: u8 = 8;
pub const CAPSULE: u8 = 16;

/// Height of a clickable control on the desktop.
pub const CONTROL_HEIGHT: f32 = 32.0;

pub fn palette(ui: &egui::Ui) -> Palette {
    themed(ui.visuals().dark_mode)
}

pub fn palette_of(ctx: &Context) -> Palette {
    themed(ctx.global_style().visuals.dark_mode)
}

/// The light or dark palette with the user's highlight color put in.
fn themed(dark: bool) -> Palette {
    let (tint, text_light, text_dark) = current_colors();
    let mut p = if dark { DARK } else { LIGHT };
    p.tint = tint;
    p.tint_text = if dark { text_dark } else { text_light };
    p
}

/// The highlight color: the one tint used for the primary button, the selected row and links.
/// Each has white text on it at a contrast of at least 4.5:1, and a lighter variant for text on dark backgrounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Blue,
    Purple,
    Pink,
    Orange,
    Green,
    Teal,
    Graphite,
    /// A color of the user's own; its value is `Settings::custom_accent`.
    Custom,
}

impl Accent {
    /// The ready-made choices (without `Custom`).
    pub const ALL: [Accent; 7] = [Accent::Blue, Accent::Purple, Accent::Pink, Accent::Orange, Accent::Green, Accent::Teal, Accent::Graphite];

    pub fn name(self) -> &'static str {
        match self {
            Accent::Blue => "Blue",
            Accent::Purple => "Purple",
            Accent::Pink => "Pink",
            Accent::Orange => "Orange",
            Accent::Green => "Green",
            Accent::Teal => "Teal",
            Accent::Graphite => "Graphite",
            Accent::Custom => "Custom",
        }
    }

    /// (fill, text on light backgrounds, text on dark backgrounds)
    fn colors(self) -> (Color32, Color32, Color32) {
        match self {
            Accent::Blue => (rgb(0x0A6CE8), rgb(0x0A6CE8), rgb(0x4DA3FF)),
            Accent::Purple => (rgb(0x7B3FE4), rgb(0x7B3FE4), rgb(0xB794FF)),
            Accent::Pink => (rgb(0xC2185B), rgb(0xC2185B), rgb(0xFF7EB3)),
            Accent::Orange => (rgb(0xB45309), rgb(0xB45309), rgb(0xFFB454)),
            Accent::Green => (rgb(0x1E7A3A), rgb(0x1E7A3A), rgb(0x4CD27A)),
            Accent::Teal => (rgb(0x0E7490), rgb(0x0E7490), rgb(0x4FD1E8)),
            Accent::Graphite => (rgb(0x636B78), rgb(0x636B78), rgb(0xB4BCC8)),
            Accent::Custom => custom_colors(current().1),
        }
    }
}

static ACCENT: std::sync::Mutex<(Accent, [u8; 3])> = std::sync::Mutex::new((Accent::Blue, [0x2E, 0xA0, 0x6B]));

fn current() -> (Accent, [u8; 3]) {
    *ACCENT.lock().unwrap_or_else(|e| e.into_inner())
}

fn current_colors() -> (Color32, Color32, Color32) {
    current().0.colors()
}

fn luminance(c: Color32) -> f32 {
    let f = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Make any color usable as the highlight: darkened until white text on it reads at 4.5:1 (it is also the
/// link color on light backgrounds), and a lightened variant for text on dark backgrounds.
fn custom_colors(rgb: [u8; 3]) -> (Color32, Color32, Color32) {
    let mut fill = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    for _ in 0..80 {
        if contrast(Color32::WHITE, fill) >= 4.5 {
            break;
        }
        let k = |v: u8| (v as f32 * 0.95) as u8;
        fill = Color32::from_rgb(k(fill.r()), k(fill.g()), k(fill.b()));
    }
    let mut light = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    for _ in 0..80 {
        if contrast(light, DARK.card) >= 4.5 {
            break;
        }
        let k = |v: u8| (v as f32 + (255.0 - v as f32) * 0.08).round() as u8;
        light = Color32::from_rgb(k(light.r()), k(light.g()), k(light.b()));
    }
    (fill, fill, light)
}

/// The fill color of a highlight color, for the swatches in Settings.
pub fn accent_fill(a: Accent) -> Color32 {
    a.colors().0
}

/// The color of the "custom" choice as it will really be used (made readable), for its swatch.
pub fn custom_fill(rgb: [u8; 3]) -> Color32 {
    custom_colors(rgb).0
}

/// Change the highlight color (`custom` is the color used when `a` is `Accent::Custom`).
/// Call `install` afterwards so the egui styles pick it up.
pub fn set_accent(a: Accent, custom: [u8; 3]) {
    *ACCENT.lock().unwrap_or_else(|e| e.into_inner()) = (a, custom);
}

fn visuals(p: Palette, base: Visuals) -> Visuals {
    let mut v = base;
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.window_stroke = Stroke::NONE;
    v.window_corner_radius = CornerRadius::same(14);
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.card;
    v.hyperlink_color = p.tint_text;
    v.weak_text_color = Some(p.label2);
    v.selection.bg_fill = p.tint;
    v.selection.stroke = Stroke::new(1.0, p.on_tint);
    // Anything you can click shows the pointing hand.
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.menu_corner_radius = CornerRadius::same(CARD_RADIUS);

    let radius = CornerRadius::same(FIELD_RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.label);
    for (state, fill) in [(&mut w.inactive, p.field), (&mut w.hovered, p.field_hover), (&mut w.active, p.field_hover), (&mut w.open, p.field)] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0, p.label);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.noninteractive.corner_radius = radius;
    v
}

pub fn install(ctx: &Context) {
    ctx.set_visuals_of(Theme::Light, visuals(themed(false), Visuals::light()));
    ctx.set_visuals_of(Theme::Dark, visuals(themed(true), Visuals::dark()));
    ctx.all_styles_mut(|s| {
        let prop = FontFamily::Proportional;
        s.text_styles = [
            (TextStyle::Heading, FontId::new(24.0, prop.clone())), // Title
            (TextStyle::Body, FontId::new(14.0, prop.clone())),
            (TextStyle::Button, FontId::new(14.0, prop.clone())),
            (TextStyle::Small, FontId::new(12.0, prop)), // footnote, never below 11
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 6.0);
        s.spacing.interact_size.y = CONTROL_HEIGHT;
        s.spacing.window_margin = Margin::same(20);
        s.spacing.scroll.bar_width = 6.0;
    });
}

/// A stable id for a named window or area. (`Id::new` is marked deprecated in favor of salts, but
/// that is exactly the "same name, same id on every frame" we want here.)
#[allow(deprecated)]
pub fn id(source: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
    egui::Id::new(source)
}

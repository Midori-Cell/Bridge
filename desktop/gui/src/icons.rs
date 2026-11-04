//! Small line icons drawn with the painter, so no icon font is needed. They are 1.6 px strokes in the
//! label color, like the SF Symbols the design guidelines use.

use egui::{pos2, vec2, Color32, CornerRadius, Painter, Pos2, Stroke, StrokeKind};

#[derive(Clone, Copy)]
pub enum Icon {
    Phone,
    Pc,
    Gear,
    Search,
    Plus,
    File,
    /// A speech bubble: a text note.
    Text,
    Folder,
    Reply,
    Copy,
    Trash,
    Close,
}

/// Draw `icon` centered at `c`, fitting a square of `size` points.
pub fn draw(painter: &Painter, icon: Icon, c: Pos2, size: f32, color: Color32) {
    let s = size;
    let stroke = Stroke::new(1.6, color);
    let at = |x: f32, y: f32| pos2(c.x + x * s, c.y + y * s);
    match icon {
        Icon::Phone => {
            let r = egui::Rect::from_center_size(c, vec2(0.46 * s, 0.84 * s));
            painter.rect_stroke(r, CornerRadius::same((0.1 * s).round() as u8 + 1), stroke, StrokeKind::Middle);
            painter.line_segment([at(-0.06, 0.28), at(0.06, 0.28)], stroke);
        }
        Icon::Pc => {
            let r = egui::Rect::from_center_size(at(0.0, -0.1), vec2(0.84 * s, 0.52 * s));
            painter.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.line_segment([at(-0.22, 0.36), at(0.22, 0.36)], stroke);
            painter.line_segment([at(0.0, 0.16), at(0.0, 0.36)], stroke);
        }
        Icon::Gear => {
            painter.circle_stroke(c, 0.3 * s, stroke);
            painter.circle_stroke(c, 0.11 * s, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let (sin, cos) = a.sin_cos();
                painter.line_segment([c + vec2(cos, sin) * 0.3 * s, c + vec2(cos, sin) * 0.46 * s], Stroke::new(2.4, color));
            }
        }
        Icon::Search => {
            painter.circle_stroke(at(-0.08, -0.08), 0.28 * s, stroke);
            painter.line_segment([at(0.13, 0.13), at(0.42, 0.42)], stroke);
        }
        Icon::Plus => {
            let thick = Stroke::new(2.2, color);
            painter.line_segment([at(0.0, -0.36), at(0.0, 0.36)], thick);
            painter.line_segment([at(-0.36, 0.0), at(0.36, 0.0)], thick);
        }
        Icon::File => {
            let pts = vec![at(0.12, -0.42), at(-0.3, -0.42), at(-0.3, 0.42), at(0.3, 0.42), at(0.3, -0.24), at(0.12, -0.42)];
            painter.add(egui::Shape::line(pts, stroke));
            painter.line_segment([at(0.12, -0.42), at(0.12, -0.24)], stroke);
            painter.line_segment([at(0.12, -0.24), at(0.3, -0.24)], stroke);
        }
        Icon::Text => {
            let r = egui::Rect::from_center_size(at(0.0, -0.06), vec2(0.84 * s, 0.62 * s));
            painter.rect_stroke(r, CornerRadius::same((0.12 * s).round() as u8 + 1), stroke, StrokeKind::Middle);
            painter.line_segment([at(-0.2, 0.25), at(-0.28, 0.42)], stroke);
            painter.line_segment([at(-0.28, 0.42), at(-0.02, 0.25)], stroke);
            painter.line_segment([at(-0.22, -0.14), at(0.22, -0.14)], stroke);
            painter.line_segment([at(-0.22, 0.04), at(0.08, 0.04)], stroke);
        }
        Icon::Folder => {
            let pts = vec![at(-0.42, -0.28), at(-0.1, -0.28), at(0.0, -0.16), at(0.42, -0.16), at(0.42, 0.3), at(-0.42, 0.3), at(-0.42, -0.28)];
            painter.add(egui::Shape::line(pts, stroke));
        }
        Icon::Reply => {
            painter.add(egui::Shape::line(vec![at(0.38, 0.3), at(0.38, 0.0), at(-0.34, 0.0)], stroke));
            painter.add(egui::Shape::line(vec![at(-0.08, -0.26), at(-0.34, 0.0), at(-0.08, 0.26)], stroke));
        }
        Icon::Copy => {
            let front = egui::Rect::from_min_max(at(-0.36, -0.12), at(0.12, 0.38));
            painter.rect_stroke(front, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.add(egui::Shape::line(vec![at(-0.12, -0.12), at(-0.12, -0.38), at(0.38, -0.38), at(0.38, 0.12), at(0.12, 0.12)], stroke));
        }
        Icon::Trash => {
            painter.line_segment([at(-0.4, -0.26), at(0.4, -0.26)], stroke);
            painter.add(egui::Shape::line(vec![at(-0.12, -0.26), at(-0.12, -0.4), at(0.12, -0.4), at(0.12, -0.26)], stroke));
            painter.add(egui::Shape::line(vec![at(-0.28, -0.18), at(-0.22, 0.4), at(0.22, 0.4), at(0.28, -0.18)], stroke));
            painter.line_segment([at(-0.08, -0.08), at(-0.08, 0.3)], stroke);
            painter.line_segment([at(0.08, -0.08), at(0.08, 0.3)], stroke);
        }
        Icon::Close => {
            let thick = Stroke::new(2.0, color);
            painter.line_segment([at(-0.3, -0.3), at(0.3, 0.3)], thick);
            painter.line_segment([at(-0.3, 0.3), at(0.3, -0.3)], thick);
        }
    }
}

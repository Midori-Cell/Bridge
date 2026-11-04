//! The app icon, drawn in code (a blue rounded square with two opposing arrows),
//! so no image files have to ship with the program.

/// RGBA pixels, `size` x `size`.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4; // samples per axis for smooth edges
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut cover, mut white) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    if in_rounded_square(x, y) {
                        cover += 1;
                        if in_arrows(x, y) {
                            white += 1;
                        }
                    }
                }
            }
            let n = SS * SS;
            // blend blue and white by how much of the covered area is arrow
            let (br, bg, bb) = (47.0, 111.0, 235.0);
            let w = if cover == 0 { 0.0 } else { white as f32 / cover as f32 };
            let c = |b: f32| (b + (255.0 - b) * w).round() as u8;
            out.extend_from_slice(&[c(br), c(bg), c(bb), (cover * 255 / n) as u8]);
        }
    }
    out
}

fn in_rounded_square(x: f32, y: f32) -> bool {
    let (m, r) = (0.04, 0.22);
    let (cx, cy) = (x.clamp(m + r, 1.0 - m - r), y.clamp(m + r, 1.0 - m - r));
    let (dx, dy) = (x - cx, y - cy);
    x >= m && x <= 1.0 - m && y >= m && y <= 1.0 - m && dx * dx + dy * dy <= r * r
}

fn in_arrows(x: f32, y: f32) -> bool {
    // upper arrow points right, lower arrow points left
    let bar = |x: f32, y: f32, y0: f32| (0.22..=0.70).contains(&x) && (y - y0).abs() <= 0.045;
    let head = |x: f32, y: f32, y0: f32| (0.66..=0.84).contains(&x) && (y - y0).abs() <= (0.84 - x) * 1.1;
    bar(x, y, 0.36) || head(x, y, 0.36) || bar(1.0 - x, y, 0.64) || head(1.0 - x, y, 0.64)
}

#[cfg(test)]
mod tests {
    /// Writes the icon as raw RGBA to the file named by `BRIDGE_ICON_OUT` (1024 x 1024). The packaging
    /// icons (`packaging/icons`) are made from it: `BRIDGE_ICON_OUT=/tmp/icon.rgba cargo test -p bridge-gui dump_icon -- --ignored`.
    #[test]
    #[ignore]
    fn dump_icon() {
        let path = std::env::var("BRIDGE_ICON_OUT").expect("set BRIDGE_ICON_OUT");
        std::fs::write(path, super::rgba(1024)).unwrap();
    }
}

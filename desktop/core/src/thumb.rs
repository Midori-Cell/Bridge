//! Small previews of pictures. A picture that is offered to another device carries a tiny JPEG (about 48 pixels
//! on the long side, a few KB) so the receiver can show it blurred in the chat without downloading anything.
//! The picture itself is only sent when the receiving user asks for it.

use std::io::Cursor;
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};

/// Longest side of a preview, in pixels.
pub const THUMB_EDGE: u32 = 48;
/// Largest preview that is accepted from another device (base64 text length).
pub const MAX_THUMB_B64: usize = 16_000;
/// Larger pictures than this get no preview (and are not decoded at all).
const MAX_PICTURE_BYTES: u64 = 100 * 1024 * 1024;
/// A picture may need at most this much memory to decode.
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;

/// Does the file name look like a picture Bridge can show?
pub fn is_picture_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.rsplit_once('.').map(|(_, e)| e), Some("jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp"))
}

/// A decoded picture, as plain RGBA pixels (not premultiplied).
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

fn open(path: &Path) -> Option<DynamicImage> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_PICTURE_BYTES {
        return None;
    }
    let mut reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o); // photos from a phone are often stored sideways
    }
    Some(img)
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    rgb.write_with_encoder(enc).ok()?;
    Some(out)
}

/// The preview of a picture file as base64 text, or `None` if the file is not a picture Bridge can read.
pub fn make(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    if !is_picture_name(&name) {
        return None;
    }
    let small = open(path)?.thumbnail(THUMB_EDGE, THUMB_EDGE);
    for quality in [60u8, 40, 25] {
        let bytes = encode_jpeg(&small, quality)?;
        let text = STANDARD.encode(&bytes);
        if text.len() <= MAX_THUMB_B64 / 2 {
            return Some(text);
        }
    }
    None
}

/// Is this preview, received from another device, safe to keep and show? It must be a small JPEG.
pub fn valid(b64: &str) -> bool {
    if b64.is_empty() || b64.len() > MAX_THUMB_B64 {
        return false;
    }
    let Ok(bytes) = STANDARD.decode(b64) else { return false };
    if bytes.len() < 4 || bytes[..3] != [0xFF, 0xD8, 0xFF] {
        return false;
    }
    let reader = ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Jpeg);
    matches!(reader.into_dimensions(), Ok((w, h)) if (1..=160).contains(&w) && (1..=160).contains(&h))
}

/// Decode a preview made by [`make`] (already checked with [`valid`]).
pub fn decode_thumb(b64: &str) -> Option<Rgba> {
    let bytes = STANDARD.decode(b64).ok()?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).ok()?;
    let rgba = img.to_rgba8();
    Some(Rgba { width: rgba.width(), height: rgba.height(), pixels: rgba.into_raw() })
}

/// Decode a picture file for showing in the chat, scaled down so its longest side is at most `max_edge`.
pub fn decode_file(path: &Path, max_edge: u32) -> Option<Rgba> {
    let img = open(path)?;
    let img = if img.width().max(img.height()) > max_edge { img.thumbnail(max_edge, max_edge) } else { img };
    let rgba = img.to_rgba8();
    Some(Rgba { width: rgba.width(), height: rgba.height(), pixels: rgba.into_raw() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(path: &Path, w: u32, h: u32) {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, 128]));
        img.save(path).unwrap();
    }

    #[test]
    fn names() {
        assert!(is_picture_name("a.JPG") && is_picture_name("x.y.png") && is_picture_name("c.webp"));
        assert!(!is_picture_name("a.txt") && !is_picture_name("png") && !is_picture_name("a.pngx"));
    }

    #[test]
    fn a_picture_gets_a_small_valid_preview() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.png");
        sample(&p, 640, 320);
        let t = make(&p).expect("preview");
        assert!(t.len() < 8000, "{} chars", t.len());
        assert!(valid(&t));
        let d = decode_thumb(&t).unwrap();
        assert_eq!((d.width, d.height), (48, 24), "keeps the proportions");
        let full = decode_file(&p, 200).unwrap();
        assert_eq!((full.width, full.height), (200, 100));
    }

    #[test]
    fn other_files_get_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.txt");
        std::fs::write(&p, "hello").unwrap();
        assert!(make(&p).is_none());
        let fake = dir.path().join("fake.png");
        std::fs::write(&fake, "not a picture").unwrap();
        assert!(make(&fake).is_none());
    }

    #[test]
    fn previews_from_other_devices_are_checked() {
        assert!(!valid(""));
        assert!(!valid("not base64 !!"));
        assert!(!valid(&STANDARD.encode(b"plain text, not a jpeg")));
        assert!(!valid(&"A".repeat(MAX_THUMB_B64 + 1)));
        // a picture that is too big in pixels is not a preview
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("p.jpg");
        sample(&p, 400, 400);
        let bytes = std::fs::read(&p).unwrap();
        assert!(!valid(&STANDARD.encode(bytes)));
    }
}

//! Pictures in the chat. A picture file is decoded on a background thread (never while a frame is drawn), turned
//! into a texture, and kept for a while. Until the picture itself is there, or when it was never downloaded, the
//! tiny preview that came with the offer is shown instead, enlarged, which makes it look blurred.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use bridge_core::thumb::{self, Rgba};
use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions, Vec2};

/// Longest side of a decoded picture, in pixels. Enough for the biggest bubble on a sharp screen.
const MAX_EDGE: u32 = 640;
/// Textures kept at the same time; the ones not drawn for longest are dropped first.
const KEEP: usize = 40;

/// What to draw for a picture.
pub struct Pic {
    pub id: TextureId,
    /// Width and height of the picture in pixels (of the full one when it is known, else of the preview).
    pub aspect: Vec2,
    /// The preview is shown, not the picture itself.
    pub blurred: bool,
}

struct Entry {
    tex: Option<TextureHandle>,
    failed: bool,
    used: u64,
}

#[derive(Default)]
struct Cache {
    full: HashMap<String, Entry>,
    previews: HashMap<String, Entry>,
    /// Pixel sizes seen so far, so a message keeps its size when its texture was dropped.
    sizes: HashMap<String, Vec2>,
    requested: std::collections::HashSet<String>,
    finished: Arc<Mutex<Vec<(String, Option<Rgba>)>>>,
    jobs: Option<Sender<(String, PathBuf)>>,
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Cache>>);

fn lock(c: &Mutex<Cache>) -> std::sync::MutexGuard<'_, Cache> {
    c.lock().unwrap_or_else(|e| e.into_inner())
}

fn color_image(img: &Rgba) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels)
}

fn start_worker(cache: &mut Cache, ctx: &Context) {
    let (tx, rx) = channel::<(String, PathBuf)>();
    let finished = cache.finished.clone();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("bridge-pictures".into())
        .spawn(move || {
            while let Ok((key, path)) = rx.recv() {
                let img = thumb::decode_file(&path, MAX_EDGE);
                finished.lock().unwrap_or_else(|e| e.into_inner()).push((key, img));
                ctx.request_repaint();
            }
        })
        .ok();
    cache.jobs = Some(tx);
}

/// The picture to show for one file. `path` is the file on this PC (if it is there), `preview` the small preview
/// (if one came with it). `visible`: only pictures that are on screen are decoded.
pub fn show(ctx: &Context, id: &str, path: Option<&Path>, preview: Option<&str>, visible: bool) -> Option<Pic> {
    let shared: Shared = ctx.data_mut(|d| d.get_temp_mut_or_insert_with(crate::theme::id("bridge-pictures"), Shared::default).clone());
    let mut c = lock(&shared.0);
    let frame = ctx.cumulative_pass_nr();

    // pictures the worker finished since the last frame become textures
    let done: Vec<_> = std::mem::take(&mut *c.finished.lock().unwrap_or_else(|e| e.into_inner()));
    for (key, img) in done {
        c.requested.remove(&key);
        let entry = match img {
            Some(img) => {
                c.sizes.insert(key.clone(), Vec2::new(img.width as f32, img.height as f32));
                Entry { tex: Some(ctx.load_texture(&key, color_image(&img), TextureOptions::LINEAR)), failed: false, used: frame }
            }
            None => Entry { tex: None, failed: true, used: frame },
        };
        c.full.insert(key, entry);
    }

    let mut result_full = None;
    if let Some(path) = path {
        let key = format!("f:{id}");
        if let Some(e) = c.full.get_mut(&key) {
            e.used = frame;
            result_full = e.tex.as_ref().map(|t| (t.id(), key.clone()));
        } else if visible && !c.requested.contains(&key) {
            if c.jobs.is_none() {
                start_worker(&mut c, ctx);
            }
            c.requested.insert(key.clone());
            if let Some(tx) = &c.jobs {
                let _ = tx.send((key, path.to_path_buf()));
            }
        }
    }
    if let Some((tex, key)) = result_full {
        let aspect = c.sizes.get(&key).copied().unwrap_or(Vec2::new(4.0, 3.0));
        evict(&mut c, frame);
        return Some(Pic { id: tex, aspect, blurred: false });
    }

    // not decoded (yet), or not downloaded: the small preview
    if let Some(b64) = preview {
        let key = format!("t:{id}");
        if !c.previews.contains_key(&key) {
            let entry = match thumb::decode_thumb(b64) {
                Some(img) => {
                    c.sizes.insert(key.clone(), Vec2::new(img.width as f32, img.height as f32));
                    Entry { tex: Some(ctx.load_texture(&key, color_image(&img), TextureOptions::LINEAR)), failed: false, used: frame }
                }
                None => Entry { tex: None, failed: true, used: frame },
            };
            c.previews.insert(key.clone(), entry);
        }
        let aspect = c.sizes.get(&format!("f:{id}")).or_else(|| c.sizes.get(&key)).copied().unwrap_or(Vec2::new(4.0, 3.0));
        if let Some(e) = c.previews.get_mut(&key) {
            e.used = frame;
            if let Some(tex) = &e.tex {
                return Some(Pic { id: tex.id(), aspect, blurred: true });
            }
        }
    }
    None
}

/// Size of the picture shown for `id` if it is known already (for laying out the message before it is drawn).
pub fn known_aspect(ctx: &Context, id: &str) -> Option<Vec2> {
    let shared: Shared = ctx.data_mut(|d| d.get_temp_mut_or_insert_with(crate::theme::id("bridge-pictures"), Shared::default).clone());
    let c = lock(&shared.0);
    c.sizes.get(&format!("f:{id}")).or_else(|| c.sizes.get(&format!("t:{id}"))).copied()
}

fn evict(c: &mut Cache, frame: u64) {
    if c.full.len() <= KEEP {
        return;
    }
    let mut by_age: Vec<(u64, String)> = c.full.iter().filter(|(_, e)| e.used + 1 < frame).map(|(k, e)| (e.used, k.clone())).collect();
    by_age.sort();
    let excess = c.full.len() - KEEP;
    for (_, k) in by_age.into_iter().take(excess) {
        c.full.remove(&k); // its size stays known, and it is decoded again when it comes into view
    }
}

/// The picture file could not be decoded (damaged, or a kind of picture that is not supported).
pub fn failed(ctx: &Context, id: &str) -> bool {
    let shared: Shared = ctx.data_mut(|d| d.get_temp_mut_or_insert_with(crate::theme::id("bridge-pictures"), Shared::default).clone());
    let c = lock(&shared.0);
    c.full.get(&format!("f:{id}")).is_some_and(|e| e.failed)
}

//! Hats and glasses people import from their own images. Windows' built-in decoders read the file,
//! on a worker thread so a file dialog or a big picture never freezes the buddies.
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Controls::Dialogs::*;

use crate::settings;
use crate::win;

const MAX_FILE: u64 = 20 * 1024 * 1024;
const MAX_SOURCE: u32 = 12_000;
const MAX_SIDE: u32 = 256;
const IMAGE_TYPES: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "gif", "webp"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot { Hat, Glasses }

/// Premultiplied BGRA pixels, cropped to the visible part of the picture.
pub struct Wear { pub name: String, pub w: u32, pub h: u32, pub bgra: Vec<u8> }

pub struct Loaded { pub slot: Slot, pub wear: Arc<Wear>, pub select: bool }

#[derive(Default)]
pub struct Wardrobe { pub hats: Vec<Arc<Wear>>, pub glasses: Vec<Arc<Wear>> }

impl Wardrobe {
    pub fn list(&self, slot: Slot) -> &Vec<Arc<Wear>> { if slot == Slot::Hat { &self.hats } else { &self.glasses } }
    fn list_mut(&mut self, slot: Slot) -> &mut Vec<Arc<Wear>> { if slot == Slot::Hat { &mut self.hats } else { &mut self.glasses } }
    pub fn find(&self, slot: Slot, name: &str) -> Option<Arc<Wear>> { self.list(slot).iter().find(|w| w.name == name).cloned() }

    pub fn add(&mut self, slot: Slot, wear: Arc<Wear>) {
        let list = self.list_mut(slot);
        list.retain(|w| w.name != wear.name);
        list.push(wear);
    }

    pub fn remove(&mut self, slot: Slot, name: &str) {
        self.list_mut(slot).retain(|w| w.name != name);
        if is_plain_name(name) { let _ = std::fs::remove_file(folder(slot).join(name)); }
    }
}

fn folder(slot: Slot) -> PathBuf {
    settings::default_path().with_file_name("wear").join(if slot == Slot::Hat { "hats" } else { "glasses" })
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != ".."
}

fn image_ext(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    IMAGE_TYPES.contains(&ext.as_str()).then_some(ext)
}

/// A file name that sorts in import order and is safe to save.
fn stored_name(stamp: u128, stem: &str, ext: &str) -> String {
    let clean: String = stem.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect();
    format!("{stamp:014}-{}.{ext}", if clean.is_empty() { "image" } else { &clean })
}

/// Where a picture goes, in sprite cells from the sprite's top-left: (x, y, width, height).
pub fn place(slot: Slot, w: u32, h: u32, head: f32, size: f32, lift: f32) -> (f32, f32, f32, f32) {
    let aspect = h.max(1) as f32 / w.max(1) as f32;
    match slot {
        Slot::Hat => {
            let wide = 13.0 * size;
            let tall = wide * aspect;
            ((16.0 - wide) / 2.0, head - tall + 0.6 - lift, wide, tall)
        }
        Slot::Glasses => {
            let wide = 12.0 * size;
            let tall = wide * aspect;
            ((16.0 - wide) / 2.0, head + 2.0 - tall / 2.0 - lift, wide, tall)
        }
    }
}

/// The smallest box holding every visible pixel, or None for a fully transparent picture.
fn visible_box(px: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if px[((y * w + x) * 4 + 3) as usize] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

fn crop(px: &[u8], w: u32, b: (u32, u32, u32, u32)) -> Vec<u8> {
    let mut out = Vec::with_capacity(((b.2 - b.0) * (b.3 - b.1) * 4) as usize);
    for y in b.1..b.3 {
        let start = ((y * w + b.0) * 4) as usize;
        out.extend_from_slice(&px[start..start + ((b.2 - b.0) * 4) as usize]);
    }
    out
}

fn decode(path: &Path, name: String) -> Result<Wear, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE { return Err("That image is over 20 MB.".into()); }
    let bad = |e: windows::core::Error| format!("Windows couldn't read that image ({e}).");
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(bad)?;
        let decoder = wic.CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).map_err(bad)?;
        let frame = decoder.GetFrame(0).map_err(bad)?;
        let (mut iw, mut ih) = (0, 0);
        frame.GetSize(&mut iw, &mut ih).map_err(bad)?;
        if iw == 0 || ih == 0 || iw > MAX_SOURCE || ih > MAX_SOURCE { return Err("That image is too large.".into()); }
        let k = (MAX_SIDE as f32 / iw.max(ih) as f32).min(1.0);
        let (nw, nh) = (((iw as f32 * k).round() as u32).max(1), ((ih as f32 * k).round() as u32).max(1));
        let scaler = wic.CreateBitmapScaler().map_err(bad)?;
        scaler.Initialize(&frame, nw, nh, WICBitmapInterpolationModeFant).map_err(bad)?;
        let conv = wic.CreateFormatConverter().map_err(bad)?;
        conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).map_err(bad)?;
        let mut px = vec![0u8; (nw * nh * 4) as usize];
        conv.CopyPixels(std::ptr::null(), nw * 4, &mut px).map_err(bad)?;
        let b = visible_box(&px, nw, nh).ok_or("That image is completely transparent.")?;
        Ok(Wear { name, w: b.2 - b.0, h: b.3 - b.1, bgra: crop(&px, nw, b) })
    }
}

/// Reads everything imported earlier, in the background.
pub fn load_saved(tx: Sender<Loaded>) {
    std::thread::spawn(move || {
        unsafe { let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); }
        for slot in [Slot::Hat, Slot::Glasses] {
            let Ok(dir) = std::fs::read_dir(folder(slot)) else { continue };
            let mut files: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| image_ext(p).is_some()).collect();
            files.sort();
            for p in files {
                let Some(name) = p.file_name().and_then(|n| n.to_str()).map(String::from) else { continue };
                if let Ok(wear) = decode(&p, name) { let _ = tx.send(Loaded { slot, wear: Arc::new(wear), select: false }); }
            }
        }
    });
}

fn pick_file() -> Option<PathBuf> {
    let filter: Vec<u16> = "Images\0*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.webp\0\0".encode_utf16().collect();
    let mut buf = [0u16; 1024];
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: w!("Choose an image (transparent PNG works best)"),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER,
        ..Default::default()
    };
    unsafe {
        if !GetOpenFileNameW(&mut dialog).as_bool() { return None; }
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// Asks for an image, keeps a copy in the app's folder, and sends it back when it's ready.
pub fn import(slot: Slot, tx: Sender<Loaded>) {
    std::thread::spawn(move || {
        unsafe { let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); }
        let Some(path) = pick_file() else { return };
        let Some(ext) = image_ext(&path) else {
            win::alert("Please choose a PNG, JPG, BMP, GIF or WebP image.");
            return;
        };
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
        let name = stored_name(stamp, stem, &ext);
        let wear = match decode(&path, name.clone()) {
            Ok(w) => w,
            Err(e) => {
                win::alert(&e);
                return;
            }
        };
        let dir = folder(slot);
        if std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&path, dir.join(&name))).is_err() {
            win::alert("Couldn't save a copy of that image, so it will be gone after a restart.");
        }
        let _ = tx.send(Loaded { slot, wear: Arc::new(wear), select: true });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_finds_the_visible_part() {
        let mut px = vec![0u8; 4 * 4 * 4];
        px[(5 * 4) + 3] = 255;   // (1, 1)
        px[(10 * 4) + 3] = 255;  // (2, 2)
        let b = visible_box(&px, 4, 4).unwrap();
        assert_eq!(b, (1, 1, 3, 3));
        assert_eq!(crop(&px, 4, b).len(), 2 * 2 * 4);
        assert_eq!(visible_box(&[0; 16], 2, 2), None);
    }

    #[test]
    fn hats_sit_on_the_head_and_glasses_on_the_eyes() {
        let (x, y, w, h) = place(Slot::Hat, 100, 50, 2.0, 1.0, 0.0);
        assert_eq!(x + w / 2.0, 8.0);
        assert!((y + h - 2.6).abs() < 1e-5, "the brim overlaps the head slightly");
        let (_, y, _, h) = place(Slot::Glasses, 120, 40, 2.0, 1.0, 0.0);
        assert!((y + h / 2.0 - 4.0).abs() < 1e-5);
        assert!(place(Slot::Hat, 10, 10, 2.0, 1.0, 3.0).1 < place(Slot::Hat, 10, 10, 2.0, 1.0, 0.0).1, "lift moves it up");
    }

    #[test]
    fn stored_names_are_safe_and_sort_by_time() {
        let a = stored_name(5, "../my hat!", "png");
        assert!(is_plain_name(&a));
        assert!(!a.contains(".."));
        assert!(stored_name(5, "z", "png") < stored_name(6, "a", "png"));
        assert!(!is_plain_name("..\\evil.png"));
    }
}

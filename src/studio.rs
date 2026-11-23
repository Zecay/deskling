//! Deskling Studio: the real settings window. It's drawn natively, so it needs no extra runtime,
//! and it only exists while it's open, so it costs nothing the rest of the time.
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::Instant;

use windows::core::{w, Result, HSTRING, PCWSTR};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::brain::{Mode, Tune};
use crate::gpu::Color;
use crate::menu::THEMES;
use crate::settings::{self, Settings, Skin, GLASSES, HATS, PATTERNS, TRAILS};
use crate::sprites;
use crate::wear::{self, Slot, Wardrobe, Wear};

pub const PAGES: [(&str, &str, &str); 6] = [
    ("✦", "Look", "Colors, patterns and accessories for your crew."),
    ("☺", "Personality", "How they behave and what they say."),
    ("⚙", "World", "Physics, size and what they're allowed to do."),
    ("◉", "Menu", "Pick your right-click menu and make it yours."),
    ("☰", "General", "Everyday options."),
    ("?", "About", "Controls, privacy and version."),
];

const WIDTH: f32 = 920.0;
const HEIGHT: f32 = 660.0;
const SIDE: f32 = 196.0;
const ROW: f32 = 46.0;
const BG: Color = [0.063, 0.063, 0.082, 1.0];
const PANEL: Color = [0.094, 0.094, 0.122, 1.0];
const CARD: Color = [0.133, 0.133, 0.169, 1.0];
const LINE: Color = [1.0, 1.0, 1.0, 0.08];
const INK: Color = [0.95, 0.95, 0.97, 1.0];
const MUTED: Color = [0.62, 0.62, 0.69, 1.0];
const ACCENT: Color = [0.843, 0.467, 0.341, 1.0];
const SWATCHES: [u32; 16] = [0xD77757, 0xFF4D6D, 0xE86FA6, 0xB36BFF, 0x5B6BD6, 0x4D9DFF, 0x3FB8AF, 0x4DD599,
                             0x8BC34A, 0xFFD23F, 0xFF8A3D, 0xA0522D, 0xE8E4DC, 0x9AA7B8, 0x3A3A3A, 0x111114];

#[derive(Default)]
struct Input { x: f32, y: f32, press_x: f32, press_y: f32, down: bool, pressed: bool, released: bool, wheel: f32, chars: Vec<char>, close: bool, dirty: bool }

thread_local! { static INPUT: RefCell<Input> = RefCell::new(Input::default()); }

extern "system" fn studio_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let with = |f: &mut dyn FnMut(&mut Input)| INPUT.with(|i| { let mut i = i.borrow_mut(); f(&mut i); i.dirty = true; });
    let (x, y) = ((lp.0 & 0xFFFF) as u16 as i16 as f32, ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32);
    unsafe {
        match msg {
            WM_MOUSEMOVE => { with(&mut |i| { i.x = x; i.y = y; }); LRESULT(0) }
            WM_LBUTTONDOWN => {
                let _ = SetCapture(hwnd);
                let _ = SetFocus(hwnd);
                with(&mut |i| { i.x = x; i.y = y; i.press_x = x; i.press_y = y; i.down = true; i.pressed = true; });
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                with(&mut |i| { i.x = x; i.y = y; i.down = false; i.released = true; });
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let d = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
                with(&mut |i| i.wheel += d);
                LRESULT(0)
            }
            WM_CHAR => {
                if let Some(c) = char::from_u32(wp.0 as u32) { with(&mut |i| i.chars.push(c)); }
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
                with(&mut |_| {});
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_CLOSE => { with(&mut |i| i.close = true); LRESULT(0) }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn rgb(v: u32) -> Color {
    let c = sprites::rgb(v);
    [c[0], c[1], c[2], 1.0]
}

fn d2c(c: Color) -> D2D1_COLOR_F { D2D1_COLOR_F { r: c[0], g: c[1], b: c[2], a: c[3] } }

fn rc(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F { D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h } }

/// A slider position (0 to 1) to its value, and back.
fn slide_value(t: f32, lo: f64, hi: f64) -> f64 { lo + (hi - lo) * t.clamp(0.0, 1.0) as f64 }
fn slide_pos(v: f64, lo: f64, hi: f64) -> f32 { (((v - lo) / (hi - lo)) as f32).clamp(0.0, 1.0) }

fn percent(v: f64) -> String { format!("{}%", (v * 100.0).round() as i64) }
fn whole(v: f64) -> String { format!("{}", v.round() as i64) }

fn with_channel(c: u32, shift: u32, value: u32) -> u32 { (c & !(0xFF << shift)) | ((value & 0xFF) << shift) }

struct Fonts { title: IDWriteTextFormat, body: IDWriteTextFormat, small: IDWriteTextFormat, center: IDWriteTextFormat, tiny: IDWriteTextFormat, icon: IDWriteTextFormat }

struct Ui<'a> {
    rt: &'a ID2D1RenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    art: &'a HashMap<String, ID2D1Bitmap>,
    f: &'a Fonts,
    k: f32,
    input: &'a Input,
    active: &'a mut Option<u32>,
    next: u32,
    x: f32,
    y: f32,
    w: f32,
    top: f32,
    changed: bool,
}

impl Ui<'_> {
    fn fill(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&d2c(c));
            self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rc(x, y, w, h), radiusX: r, radiusY: r }, self.brush);
        }
    }

    fn edge(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Color, width: f32) {
        unsafe {
            self.brush.SetColor(&d2c(c));
            self.rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rc(x, y, w, h), radiusX: r, radiusY: r }, self.brush, width, None);
        }
    }

    fn dot(&self, x: f32, y: f32, r: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&d2c(c));
            self.rt.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: r, radiusY: r }, self.brush);
        }
    }

    fn text(&self, t: &str, font: &IDWriteTextFormat, x: f32, y: f32, w: f32, h: f32, c: Color) {
        let wide: Vec<u16> = t.encode_utf16().collect();
        unsafe {
            self.brush.SetColor(&d2c(c));
            self.rt.DrawText(&wide, font, &rc(x, y, w, h), self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
    }

    fn over(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let m = (self.input.x, self.input.y);
        m.1 >= self.top && m.0 >= x && m.0 <= x + w && m.1 >= y && m.1 <= y + h
    }

    /// Uses where the button went down, so a pointer moving before the next frame can't hit another control.
    fn clicked(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let (px, py) = (self.input.press_x, self.input.press_y);
        self.input.pressed && py >= self.top && px >= x && px <= x + w && py >= y && py <= y + h
    }

    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    fn heading(&mut self, t: &str) {
        let k = self.k;
        self.y += 8.0 * k;
        self.text(&t.to_uppercase(), &self.f.small, self.x, self.y, self.w, 18.0 * k, ACCENT);
        self.y += 26.0 * k;
    }

    fn label(&self, t: &str, h: f32) { self.text(t, &self.f.body, self.x, self.y + (h - 34.0 * self.k).max(0.0) / 2.0 + 9.0 * self.k, 190.0 * self.k, 24.0 * self.k, INK); }

    fn toggle(&mut self, title: &str, hint: &str, v: &mut bool) {
        let k = self.k;
        let (y, h) = (self.y, ROW * k);
        self.text(title, &self.f.body, self.x, y + 3.0 * k, self.w - 70.0 * k, 22.0 * k, INK);
        self.text(hint, &self.f.small, self.x, y + 24.0 * k, self.w - 70.0 * k, 18.0 * k, MUTED);
        let (tw, th) = (44.0 * k, 24.0 * k);
        let (tx, ty) = (self.x + self.w - tw, y + (h - th) / 2.0);
        if self.clicked(self.x, y, self.w, h) {
            *v = !*v;
            self.changed = true;
        }
        self.fill(tx, ty, tw, th, th / 2.0, if *v { ACCENT } else { CARD });
        self.dot(if *v { tx + tw - th / 2.0 } else { tx + th / 2.0 }, ty + th / 2.0, 8.5 * k, INK);
        self.y += h;
    }

    fn slider(&mut self, title: &str, v: &mut f64, lo: f64, hi: f64, show: fn(f64) -> String) {
        let k = self.k;
        let id = self.id();
        let (y, h) = (self.y, 38.0 * k);
        self.text(title, &self.f.body, self.x, y + 7.0 * k, 190.0 * k, 24.0 * k, INK);
        let (tx, tw) = (self.x + 200.0 * k, self.w - 270.0 * k);
        if self.clicked(tx - 8.0 * k, y, tw + 16.0 * k, h) { *self.active = Some(id); }
        if *self.active == Some(id) && self.input.down {
            let next = slide_value((self.input.x - tx) / tw, lo, hi);
            if (next - *v).abs() > 1e-6 {
                *v = next;
                self.changed = true;
            }
        }
        let t = slide_pos(*v, lo, hi);
        let mid = y + h / 2.0;
        self.fill(tx, mid - 3.0 * k, tw, 6.0 * k, 3.0 * k, CARD);
        self.fill(tx, mid - 3.0 * k, tw * t, 6.0 * k, 3.0 * k, ACCENT);
        self.dot(tx + tw * t, mid, 9.0 * k, INK);
        self.text(&show(*v), &self.f.body, tx + tw + 18.0 * k, y + 7.0 * k, 54.0 * k, 24.0 * k, MUTED);
        self.y += h;
    }

    fn choice(&mut self, title: &str, options: &[&str], v: &mut u8) {
        let k = self.k;
        let (y, h) = (self.y, 42.0 * k);
        self.label(title, h);
        let (bx, bw) = (self.x + 200.0 * k, self.w - 200.0 * k);
        let cell = bw / options.len() as f32;
        self.fill(bx, y + 3.0 * k, bw, 34.0 * k, 9.0 * k, CARD);
        for (i, o) in options.iter().enumerate() {
            let cx = bx + cell * i as f32;
            if self.clicked(cx, y, cell, h) && *v != i as u8 {
                *v = i as u8;
                self.changed = true;
            }
            let on = *v == i as u8;
            if on { self.fill(cx + 2.0 * k, y + 5.0 * k, cell - 4.0 * k, 30.0 * k, 7.0 * k, ACCENT); }
            else if self.over(cx, y, cell, h) { self.fill(cx + 2.0 * k, y + 5.0 * k, cell - 4.0 * k, 30.0 * k, 7.0 * k, LINE); }
            self.text(o, &self.f.center, cx, y + 3.0 * k, cell, 34.0 * k, if on { INK } else { MUTED });
        }
        self.y += h;
    }

    fn color(&mut self, title: &str, v: &mut u32) {
        let k = self.k;
        let y = self.y;
        self.label(title, 32.0 * k);
        // The colour you're making, big enough to judge, with its code.
        let chip = 64.0 * k;
        let (cx, cw) = (self.x + self.w - chip, self.w - 200.0 * k - chip - 12.0 * k);
        self.fill(cx, y + 3.0 * k, chip, 42.0 * k, 9.0 * k, rgb(*v));
        self.edge(cx, y + 3.0 * k, chip, 42.0 * k, 9.0 * k, [1.0, 1.0, 1.0, 0.35], 1.2 * k);
        self.text(&format!("#{:06X}", *v), &self.f.center, cx - 8.0 * k, y + 45.0 * k, chip + 16.0 * k, 18.0 * k, MUTED);
        let bx = self.x + 200.0 * k;
        let cell = cw / SWATCHES.len() as f32;
        for (i, &c) in SWATCHES.iter().enumerate() {
            let sx = bx + cell * i as f32;
            if self.clicked(sx, y, cell, 30.0 * k) && *v != c {
                *v = c;
                self.changed = true;
            }
            self.fill(sx + 2.0 * k, y + 3.0 * k, cell - 4.0 * k, 24.0 * k, 6.0 * k, rgb(c));
            if *v == c { self.edge(sx + 1.0 * k, y + 2.0 * k, cell - 2.0 * k, 26.0 * k, 7.0 * k, INK, 2.0 * k); }
        }
        self.y += 34.0 * k;
        // Any colour at all: one small bar each for red, green and blue.
        let bar = (cw - 32.0 * k) / 3.0;
        for (n, (shift, tint)) in [(16u32, 0xFF4D4Du32), (8, 0x4DD54D), (0, 0x4D7DFF)].into_iter().enumerate() {
            let id = self.id();
            let bx2 = bx + 6.0 * k + (bar + 10.0 * k) * n as f32;
            let (by, bh) = (self.y, 22.0 * k);
            if self.clicked(bx2 - 6.0 * k, by, bar + 12.0 * k, bh) { *self.active = Some(id); }
            if *self.active == Some(id) && self.input.down {
                let value = (((self.input.x - bx2) / bar).clamp(0.0, 1.0) * 255.0).round() as u32;
                let next = with_channel(*v, shift, value);
                if next != *v {
                    *v = next;
                    self.changed = true;
                }
            }
            let part = ((*v >> shift) & 255) as f32 / 255.0;
            self.fill(bx2, by + 7.0 * k, bar, 6.0 * k, 3.0 * k, CARD);
            self.fill(bx2, by + 7.0 * k, bar * part, 6.0 * k, 3.0 * k, rgb(tint));
            self.dot(bx2 + bar * part, by + 10.0 * k, 6.5 * k, INK);
        }
        self.y += 34.0 * k;
    }

    /// Your imported pictures: a + tile, then one tile each. Returns true if the choice changed.
    fn gallery(&mut self, title: &str, slot: Slot, items: &[Arc<Wear>], picked: &mut String, out: &mut Out) -> bool {
        let k = self.k;
        let (tile, gap) = (48.0 * k, 6.0 * k);
        let (bx, bw) = (self.x + 200.0 * k, self.w - 200.0 * k);
        let per = ((bw + gap) / (tile + gap)).floor().max(1.0) as usize;
        let y0 = self.y;
        self.text(title, &self.f.body, self.x, y0 + 13.0 * k, 190.0 * k, 24.0 * k, INK);
        let spot = |n: usize| (bx + (n % per) as f32 * (tile + gap), y0 + (n / per) as f32 * (tile + gap));
        let (ax, ay) = spot(0);
        let hot = self.over(ax, ay, tile, tile);
        self.fill(ax, ay, tile, tile, 9.0 * k, if hot { [0.2, 0.2, 0.25, 1.0] } else { CARD });
        self.edge(ax, ay, tile, tile, 9.0 * k, ACCENT, 1.2 * k);
        self.text("+", &self.f.icon, ax, ay - 7.0 * k, tile, tile, INK);
        self.text("Import", &self.f.tiny, ax, ay + 13.0 * k, tile, tile - 13.0 * k, MUTED);
        if self.clicked(ax, ay, tile, tile) { out.import = Some(slot); }
        let mut changed = false;
        for (i, item) in items.iter().enumerate() {
            let (x, y) = spot(i + 1);
            let on = *picked == item.name;
            self.fill(x, y, tile, tile, 9.0 * k, CARD);
            if let Some(b) = self.art.get(&art_key(slot, &item.name)) {
                let fit = (tile - 10.0 * k) / item.w.max(item.h) as f32;
                let (dw, dh) = (item.w as f32 * fit, item.h as f32 * fit);
                unsafe { self.rt.DrawBitmap(b, Some(&rc(x + (tile - dw) / 2.0, y + (tile - dh) / 2.0, dw, dh)), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None); }
            }
            if on {
                self.edge(x, y, tile, tile, 9.0 * k, INK, 2.0 * k);
                let (dx, dy, dr) = (x + tile - 8.0 * k, y + 8.0 * k, 8.0 * k);
                self.dot(dx, dy, dr, [0.85, 0.2, 0.25, 1.0]);
                self.text("×", &self.f.center, dx - dr, dy - dr - 1.0 * k, dr * 2.0, dr * 2.0, INK);
                if self.clicked(dx - dr, dy - dr, dr * 2.0, dr * 2.0) {
                    out.delete = Some((slot, item.name.clone()));
                    continue;
                }
            }
            if self.clicked(x, y, tile, tile) && !on {
                picked.clone_from(&item.name);
                changed = true;
                self.changed = true;
            }
        }
        self.y += ((items.len() + 1).div_ceil(per)) as f32 * (tile + gap) + 4.0 * k;
        changed
    }

    fn button(&mut self, x: f32, y: f32, w: f32, t: &str, primary: bool) -> bool {
        let k = self.k;
        let h = 36.0 * k;
        let hot = self.over(x, y, w, h);
        let base = if primary { ACCENT } else if hot { [0.2, 0.2, 0.25, 1.0] } else { CARD };
        self.fill(x, y, w, h, 9.0 * k, base);
        self.text(t, &self.f.center, x, y, w, h, INK);
        self.clicked(x, y, w, h)
    }

    fn buttons(&mut self, labels: &[&str]) -> Option<usize> {
        let k = self.k;
        let cell = (self.w - 8.0 * k * (labels.len() as f32 - 1.0)) / labels.len() as f32;
        let y = self.y;
        let mut hit = None;
        for (i, l) in labels.iter().enumerate() {
            if self.button(self.x + (cell + 8.0 * k) * i as f32, y, cell, l, false) { hit = Some(i); }
        }
        self.y += 46.0 * k;
        hit
    }

    fn para(&mut self, t: &str, lines: f32) {
        let h = lines * 21.0 * self.k;
        self.text(t, &self.f.body, self.x, self.y, self.w, h, MUTED);
        self.y += h + 6.0 * self.k;
    }
}

#[derive(Default)]
pub struct Out {
    pub changed: bool, pub close: bool, pub try_menu: bool, pub add: usize, pub remove_all: bool,
    pub import: Option<Slot>, pub delete: Option<(Slot, String)>,
}

fn art_key(slot: Slot, name: &str) -> String { format!("{slot:?}/{name}") }

pub struct Studio {
    hwnd: HWND,
    factory: ID2D1Factory,
    target: Option<(ID2D1HwndRenderTarget, ID2D1SolidColorBrush)>,
    art: HashMap<String, ID2D1Bitmap>,
    fonts: Fonts,
    k: f32,
    page: usize,
    scroll: [f32; PAGES.len()],
    extent: f32,
    active: Option<u32>,
    typing: bool,
    phrases: String,
    began: Instant,
    last: Instant,
    again: bool,
    count: usize,
}

impl Studio {
    pub fn new(ui: f64, page: usize, s: &Settings) -> Result<Self> {
        let k = ui as f32;
        unsafe {
            let module = GetModuleHandleW(None)?;
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(studio_proc), hInstance: module.into(), lpszClassName: w!("DesklingStudio"),
                                        hIcon: LoadIconW(module, PCWSTR(1 as *const u16)).unwrap_or_default(), hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() });
            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
            let mut box_ = RECT { left: 0, top: 0, right: (WIDTH * k) as i32, bottom: (HEIGHT * k) as i32 };
            AdjustWindowRectEx(&mut box_, style, false, WINDOW_EX_STYLE(0))?;
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("DesklingStudio"), w!("Deskling Studio"), style, CW_USEDEFAULT, CW_USEDEFAULT,
                                       box_.right - box_.left, box_.bottom - box_.top, None, None, module, None)?;
            let dark = BOOL(1);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const c_void, 4);
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let font = |family: &str, weight, size: f32| dw.CreateTextFormat(&HSTRING::from(family), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size * k, w!("en-us"));
            let fonts = Fonts {
                title: font("Segoe UI", DWRITE_FONT_WEIGHT_BOLD, 24.0)?,
                body: font("Segoe UI", DWRITE_FONT_WEIGHT_NORMAL, 14.0)?,
                small: font("Segoe UI", DWRITE_FONT_WEIGHT_SEMI_BOLD, 11.5)?,
                center: font("Segoe UI", DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.0)?,
                tiny: font("Segoe UI", DWRITE_FONT_WEIGHT_NORMAL, 10.0)?,
                icon: font("Segoe UI Symbol", DWRITE_FONT_WEIGHT_NORMAL, 16.0)?,
            };
            for f in [&fonts.center, &fonts.tiny, &fonts.icon] {
                f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            }
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            INPUT.with(|i| *i.borrow_mut() = Input { dirty: true, ..Input::default() });
            Ok(Studio { hwnd, factory, target: None, art: HashMap::new(), fonts, k, page: page.min(PAGES.len() - 1), scroll: [0.0; PAGES.len()], extent: 0.0, active: None,
                        typing: false, phrases: s.phrases.join("; "), began: Instant::now(), last: Instant::now(), again: false, count: 0 })
        }
    }

    pub fn show(&mut self, page: usize) {
        self.page = page.min(PAGES.len() - 1);
        self.again = true;
        unsafe {
            if IsIconic(self.hwnd).as_bool() { let _ = ShowWindow(self.hwnd, SW_RESTORE); }
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    fn make_target(&mut self) -> Result<()> {
        if self.target.is_some() { return Ok(()); }
        unsafe {
            let mut c = RECT::default();
            GetClientRect(self.hwnd, &mut c)?;
            let size = D2D_SIZE_U { width: (c.right - c.left).max(1) as u32, height: (c.bottom - c.top).max(1) as u32 };
            // IMMEDIATELY: waiting for vsync here would stall the buddies' main loop.
            let rt = self.factory.CreateHwndRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES::default(),
                &D2D1_HWND_RENDER_TARGET_PROPERTIES { hwnd: self.hwnd, pixelSize: size, presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY })?;
            rt.SetDpi(96.0, 96.0);
            let brush = rt.CreateSolidColorBrush(&d2c(INK), None)?;
            self.target = Some((rt, brush));
        }
        Ok(())
    }

    /// Called every loop. It only draws when something happened, so an idle Studio is nearly free.
    pub fn frame(&mut self, s: &mut Settings, closet: &Wardrobe, count: usize) -> Out {
        let mut out = Out::default();
        let input = INPUT.with(|i| {
            let mut i = i.borrow_mut();
            let snap = Input { x: i.x, y: i.y, press_x: i.press_x, press_y: i.press_y, down: i.down, pressed: i.pressed, released: i.released, wheel: i.wheel,
                               chars: std::mem::take(&mut i.chars), close: i.close, dirty: i.dirty };
            i.pressed = false;
            i.released = false;
            i.wheel = 0.0;
            i.dirty = false;
            snap
        });
        if input.close {
            out.close = true;
            return out;
        }
        if !self.typing && s.phrases.is_empty() { self.phrases.clear(); }   // after a reset
        let animate = (self.page == 0 || self.typing) && self.last.elapsed().as_millis() >= 100;
        if closet.hats.len() + closet.glasses.len() != self.art.len() { self.again = true; }
        if count != self.count {
            self.count = count;
            self.again = true;
        }
        if !input.dirty && !animate && !self.again { return out; }
        self.again = false;
        self.last = Instant::now();
        if self.make_target().is_err() { return out; }
        let k = self.k;
        let page_h = (HEIGHT - 104.0) * k;
        if input.wheel != 0.0 && input.x > SIDE * k {
            let room = (self.extent - page_h).max(0.0);
            self.scroll[self.page] = (self.scroll[self.page] - input.wheel * 60.0 * k).clamp(0.0, room);
        }
        if input.pressed && input.press_x < SIDE * k { self.typing = false; }
        let Some((rt, brush)) = self.target.clone() else { return out };
        self.art.retain(|key, _| [Slot::Hat, Slot::Glasses].iter().any(|&slot| closet.list(slot).iter().any(|w| art_key(slot, &w.name) == *key)));
        for slot in [Slot::Hat, Slot::Glasses] {
            for w in closet.list(slot) {
                let key = art_key(slot, &w.name);
                if self.art.contains_key(&key) { continue; }
                let props = D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0, dpiY: 96.0,
                };
                if let Ok(b) = unsafe { rt.CreateBitmap(D2D_SIZE_U { width: w.w, height: w.h }, Some(w.bgra.as_ptr() as *const c_void), w.w * 4, &props) } {
                    self.art.insert(key, b);
                }
            }
        }
        let scroll = self.scroll[self.page];
        let mut ui = Ui { rt: &rt, brush: &brush, art: &self.art, f: &self.fonts, k, input: &input, active: &mut self.active, next: 0,
                          x: (SIDE + 32.0) * k, y: 96.0 * k - scroll, w: (WIDTH - SIDE - 64.0) * k, top: 88.0 * k, changed: false };
        unsafe {
            rt.BeginDraw();
            rt.SetTransform(&Matrix3x2::identity());
            rt.Clear(Some(&d2c(BG)));
            rt.PushAxisAlignedClip(&rc((SIDE + 1.0) * k, 88.0 * k, (WIDTH - SIDE) * k, (HEIGHT - 88.0) * k), D2D1_ANTIALIAS_MODE_ALIASED);
        }
        let start = ui.y;
        let t = self.began.elapsed().as_secs_f64();
        match self.page {
            0 => page_look(&mut ui, s, closet, &mut out, t),
            1 => page_personality(&mut ui, s, &mut self.phrases, &mut self.typing, &input, t),
            2 => page_world(&mut ui, s, count, &mut out),
            3 => page_menu(&mut ui, s, &mut out),
            4 => page_general(&mut ui, s),
            _ => page_about(&mut ui),
        }
        self.extent = ui.y - start + 16.0 * k;
        unsafe { rt.PopAxisAlignedClip(); }
        // Header and sidebar go on top, so scrolled content slides under them.
        ui.top = 0.0;
        ui.fill(SIDE * k, 0.0, (WIDTH - SIDE) * k, 88.0 * k, 0.0, BG);
        let (_, title, about) = PAGES[self.page];
        ui.text(title, &self.fonts.title, (SIDE + 32.0) * k, 16.0 * k, 500.0 * k, 36.0 * k, INK);
        ui.text(about, &self.fonts.body, (SIDE + 32.0) * k, 52.0 * k, 620.0 * k, 24.0 * k, MUTED);
        ui.fill(0.0, 0.0, SIDE * k, HEIGHT * k, 0.0, PANEL);
        ui.dot(32.0 * k, 38.0 * k, 9.0 * k, ACCENT);
        ui.text("Deskling Studio", &self.fonts.center, 44.0 * k, 22.0 * k, 130.0 * k, 32.0 * k, INK);
        let mut next_page = self.page;
        for (i, (icon, name, _)) in PAGES.iter().enumerate() {
            let y = (76.0 + i as f32 * 46.0) * k;
            let hot = ui.over(10.0 * k, y, (SIDE - 20.0) * k, 40.0 * k);
            if i == self.page { ui.fill(10.0 * k, y, (SIDE - 20.0) * k, 40.0 * k, 10.0 * k, ACCENT); }
            else if hot { ui.fill(10.0 * k, y, (SIDE - 20.0) * k, 40.0 * k, 10.0 * k, LINE); }
            let c = if i == self.page { INK } else { MUTED };
            ui.text(icon, &self.fonts.icon, 18.0 * k, y, 30.0 * k, 40.0 * k, c);
            ui.text(name, &self.fonts.body, 56.0 * k, y + 10.0 * k, 130.0 * k, 24.0 * k, if i == self.page { INK } else { c });
            if ui.clicked(10.0 * k, y, (SIDE - 20.0) * k, 40.0 * k) { next_page = i; }
        }
        ui.text(concat!("v", env!("CARGO_PKG_VERSION")), &self.fonts.small, 24.0 * k, (HEIGHT - 32.0) * k, 150.0 * k, 18.0 * k, MUTED);
        out.changed = ui.changed;
        if input.released { *ui.active = None; }
        if next_page != self.page {
            self.page = next_page;
            self.typing = false;
            self.again = true;
        }
        if out.changed || input.pressed || input.released { self.again = true; }
        let room = (self.extent - page_h).max(0.0);
        self.scroll[self.page] = self.scroll[self.page].min(room);
        if unsafe { rt.EndDraw(None, None) }.is_err() {
            self.target = None;
            self.art.clear();   // these belong to the old graphics device
        }   // the graphics device was reset
        out
    }
}

impl Drop for Studio {
    fn drop(&mut self) {
        self.art.clear();
        self.target = None;
        unsafe { let _ = DestroyWindow(self.hwnd); }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_preview(ui: &Ui, skin: &Skin, hat_pic: Option<&Arc<Wear>>, glasses_pic: Option<&Arc<Wear>>, x: f32, y: f32, w: f32, t: f64) {
    let k = ui.k;
    let h = 230.0 * k;
    ui.fill(x, y, w, h, 14.0 * k, PANEL);
    ui.edge(x, y, w, h, 14.0 * k, LINE, 1.0);
    let u = (11.0 * k).floor();
    let frame = sprites::pose(Mode::Walk, "", t * 0.6, false);
    let (left, top) = (x + (w - 16.0 * u) / 2.0, y + (h - 10.0 * u) / 2.0 + 14.0 * k);
    let base = if skin.custom { skin.color } else { 0xD77757 };
    let second = if skin.custom { rgb(skin.color2) } else { let c = rgb(base); [c[0] * 0.7, c[1] * 0.7, c[2] * 0.7, 1.0] };
    if skin.glow {
        // Layers of the silhouette, each wider and fainter: the same soft light the real glow has.
        let c = rgb(base);
        let light = [c[0] + (1.0 - c[0]) * 0.28, c[1] + (1.0 - c[1]) * 0.28, c[2] + (1.0 - c[2]) * 0.28];
        let body = sprites::pixels(frame, false);
        for ring in (1..=6).rev() {
            let spread = ring as f32 * 0.45 * u;
            let a = 0.05 * skin.glow_power as f32;
            unsafe { ui.brush.SetColor(&d2c([light[0], light[1], light[2], a])); }
            for &(col, row, _) in &body {
                let r = rc(left + col as f32 * u - spread, top + row as f32 * u - spread, u + 2.0 * spread, u + 2.0 * spread);
                unsafe { ui.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: spread, radiusY: spread }, ui.brush); }
            }
        }
    }
    for (col, row, value) in sprites::pixels(frame, false) {
        let c = match value {
            sprites::BODY => body_color(rgb(base), second, skin.pattern, col, row),
            sprites::EYE => rgb(0x1A1A1A),
            sprites::BLUSH => rgb(0xF4A29A),
            _ => rgb(0xFF4D6D),
        };
        unsafe {
            ui.brush.SetColor(&d2c(c));
            ui.rt.FillRectangle(&rc(left + col as f32 * u, top + row as f32 * u, u + 0.5, u + 0.5), ui.brush);
        }
    }
    let wear = |grid: &[&str], ox: f32, oy: f32| {
        for (r, line) in grid.iter().enumerate() {
            for (c, ch) in line.bytes().enumerate() {
                if ch == b'.' { continue; }
                unsafe {
                    ui.brush.SetColor(&d2c(rgb(sprites::wear_color(ch))));
                    ui.rt.FillRectangle(&rc(left + (ox + c as f32) * u, top + (oy + r as f32) * u, u + 0.5, u + 0.5), ui.brush);
                }
            }
        }
    };
    let head = 2.0 + sprites::head_drop(frame) as f32;
    let picture = |p: &Arc<Wear>, slot: Slot, size: f64, lift: f64| {
        if let Some(b) = ui.art.get(&art_key(slot, &p.name)) {
            let (px, py, pw, ph) = wear::place(slot, p.w, p.h, head, size as f32, lift as f32);
            let mode = if p.w.max(p.h) <= 64 { D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR } else { D2D1_BITMAP_INTERPOLATION_MODE_LINEAR };
            unsafe { ui.rt.DrawBitmap(b, Some(&rc(left + px * u, top + py * u, pw * u, ph * u)), 1.0, mode, None); }
        }
    };
    match hat_pic {
        Some(p) => picture(p, Slot::Hat, skin.hat_size, skin.hat_lift),
        None => {
            let hat = sprites::HAT_ART[skin.hat as usize];
            if !hat.is_empty() { wear(hat, (16 - hat[0].len()) as f32 / 2.0, head - hat.len() as f32); }
        }
    }
    match glasses_pic {
        Some(p) => picture(p, Slot::Glasses, skin.glasses_size, skin.glasses_lift),
        None => {
            let glasses = sprites::GLASSES_ART[skin.glasses as usize];
            if !glasses.is_empty() { wear(glasses, 2.0, head); }
        }
    }
    ui.text("Live preview", &ui.f.small, x + 14.0 * k, y + 11.0 * k, 140.0 * k, 18.0 * k, MUTED);
}

/// The same rule as the sprite shader, so the preview matches the real thing.
fn body_color(a: Color, b: Color, pattern: u8, col: usize, row: usize) -> Color {
    match pattern {
        1 => {
            let t = ((row as f32 - 2.0) / 6.0).clamp(0.0, 1.0);
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, 1.0]
        }
        2 if row % 2 == 1 => b,
        3 if (col + 2 * row) % 5 == 0 => b,
        _ => a,
    }
}

fn page_look(ui: &mut Ui, s: &mut Settings, closet: &Wardrobe, out: &mut Out, t: f64) {
    let k = ui.k;
    let hat_pic = closet.find(Slot::Hat, &s.custom_hat);
    let glasses_pic = closet.find(Slot::Glasses, &s.custom_glasses);
    draw_preview(ui, &s.skin, hat_pic.as_ref(), glasses_pic.as_ref(), ui.x, ui.y, ui.w, t);
    ui.y += 240.0 * k;
    ui.heading("Color");
    let skin = &mut s.skin;
    ui.toggle("One look for everyone", "Off keeps each buddy's own color", &mut skin.custom);
    if skin.custom {
        ui.color("Main color", &mut skin.color);
        ui.color("Second color", &mut skin.color2);
    }
    ui.choice("Pattern", &PATTERNS, &mut skin.pattern);
    ui.heading("Hats");
    let mut shown = if hat_pic.is_some() { u8::MAX } else { skin.hat };
    ui.choice("Hat", &HATS, &mut shown);
    if shown != u8::MAX && (hat_pic.is_some() || shown != skin.hat) {
        skin.hat = shown;
        s.custom_hat.clear();
        ui.changed = true;
    }
    if ui.gallery("Your hats", Slot::Hat, closet.list(Slot::Hat), &mut s.custom_hat, out) { skin.hat = 0; }
    if hat_pic.is_some() {
        ui.slider("Hat size", &mut skin.hat_size, 0.3, 3.0, percent);
        ui.slider("Hat height", &mut skin.hat_lift, -6.0, 6.0, whole);
    }
    ui.heading("Glasses");
    let mut shown = if glasses_pic.is_some() { u8::MAX } else { skin.glasses };
    ui.choice("Glasses", &GLASSES, &mut shown);
    if shown != u8::MAX && (glasses_pic.is_some() || shown != skin.glasses) {
        skin.glasses = shown;
        s.custom_glasses.clear();
        ui.changed = true;
    }
    if ui.gallery("Your glasses", Slot::Glasses, closet.list(Slot::Glasses), &mut s.custom_glasses, out) { skin.glasses = 0; }
    if glasses_pic.is_some() {
        ui.slider("Glasses size", &mut skin.glasses_size, 0.3, 3.0, percent);
        ui.slider("Glasses height", &mut skin.glasses_lift, -6.0, 6.0, whole);
    }
    ui.heading("Effects");
    ui.choice("Trail", &TRAILS, &mut skin.trail);
    ui.toggle("Glow", "Soft light that follows their shape", &mut skin.glow);
    if skin.glow { ui.slider("Glow strength", &mut skin.glow_power, 0.2, 2.5, percent); }
}

fn page_personality(ui: &mut Ui, s: &mut Settings, phrases: &mut String, typing: &mut bool, input: &Input, t: f64) {
    let k = ui.k;
    ui.heading("Quick presets");
    if let Some(i) = ui.buttons(&["Calm", "Default", "Party", "Chaos"]) {
        settings::apply_preset(&mut s.tune, ["calm", "default", "party", "chaos"][i]);
        ui.changed = true;
    }
    ui.heading("Traits");
    let l = s.limits();
    let n: &mut Tune = &mut s.tune;
    ui.slider("Energy", &mut n.energy, 0.25, l.traits, percent);
    ui.slider("Chattiness", &mut n.chatter, 0.2, l.traits, percent);
    ui.slider("Curiosity", &mut n.curiosity, 0.0, l.traits, percent);
    ui.slider("Sleepiness", &mut n.sleepy, 0.25, l.traits, percent);
    ui.slider("Playfulness", &mut n.playful, 0.2, l.traits, percent);
    ui.slider("Chaos", &mut n.chaos, 0.0, l.chaos, percent);
    ui.heading("Their own words");
    let (x, y, w, h) = (ui.x, ui.y, ui.w, 42.0 * k);
    if input.pressed { *typing = input.press_x >= x && input.press_x <= x + w && input.press_y >= y && input.press_y <= y + h; }
    ui.fill(x, y, w, h, 9.0 * k, CARD);
    if *typing { ui.edge(x, y, w, h, 9.0 * k, ACCENT, 1.5 * k); }
    if *typing {
        for &c in &input.chars {
            match c {
                '\u{8}' => { phrases.pop(); }
                c if !c.is_control() && phrases.chars().count() < 600 => phrases.push(c),
                _ => {}
            }
        }
        if !input.chars.is_empty() {
            s.phrases = settings::parse_phrases(phrases);
            ui.changed = true;
        }
    }
    // Keep the end of a long line visible while typing.
    let shown: String = { let n = phrases.chars().count(); phrases.chars().skip(n.saturating_sub(84)).collect() };
    let caret = if *typing && (t * 2.0) as i64 % 2 == 0 { "|" } else { "" };
    if shown.is_empty() && !*typing {
        ui.text("Click and type, like: hello there; nice code; snack time?", &ui.f.body, x + 12.0 * k, y + 10.0 * k, w - 24.0 * k, 24.0 * k, MUTED);
    } else {
        ui.text(&format!("{shown}{caret}"), &ui.f.body, x + 12.0 * k, y + 10.0 * k, w - 24.0 * k, 24.0 * k, INK);
    }
    ui.y += h + 6.0 * k;
    ui.text("Separate phrases with semicolons.", &ui.f.small, x, ui.y, w, 18.0 * k, MUTED);
    ui.y += 24.0 * k;
}

fn page_world(ui: &mut Ui, s: &mut Settings, count: usize, out: &mut Out) {
    let l = s.limits();
    let k = ui.k;
    ui.heading(&format!("Crew: {count} {}", if count == 1 { "buddy" } else { "buddies" }));
    let added: &[&str] = if s.unleashed { &["+1", "+10", "+100", "+1,000", "Remove all"] } else { &["+1", "+10", "+50", "Remove all"] };
    if let Some(i) = ui.buttons(added) {
        if i == added.len() - 1 { out.remove_all = true; }
        else { out.add = if s.unleashed { [1, 10, 100, 1000][i] } else { [1, 10, 50][i] }; }
    }
    ui.heading("Physics");
    ui.slider("Buddy size", &mut s.size, 1.0, l.size, whole);
    s.size = s.size.round();
    let n = &mut s.tune;
    ui.slider("Gravity", &mut n.gravity, l.gravity.0, l.gravity.1, percent);
    ui.slider("Walk speed", &mut n.speed, l.speed.0, l.speed.1, percent);
    ui.slider("Bounce", &mut n.bounce, 0.0, l.bounce, percent);
    ui.heading("Behaviors");
    ui.toggle("Climb onto windows", "Hop up onto the top of your apps", &mut n.climb);
    ui.toggle("Hang out together", "Chats, dance-offs, high fives and tag", &mut n.social);
    ui.toggle("Take naps", "Fall asleep when you step away", &mut n.naps);
    ui.y += 8.0 * k;
    if ui.buttons(&["Reset physics and behaviors"]).is_some() {
        let keep = s.tune;
        s.tune = Tune { energy: keep.energy, chatter: keep.chatter, curiosity: keep.curiosity, sleepy: keep.sleepy, playful: keep.playful, chaos: keep.chaos, ..Tune::default() };
        s.size = 4.0;
        ui.changed = true;
    }
}

fn page_menu(ui: &mut Ui, s: &mut Settings, out: &mut Out) {
    let m = &mut s.menu;
    ui.heading("Style");
    let mut kind = u8::from(m.list);
    ui.choice("Menu type", &["Radial wheel", "Dropdown panel"], &mut kind);
    m.list = kind == 1;
    let names: Vec<&str> = THEMES.iter().map(|t| t.0).collect();
    ui.choice("Theme", &names, &mut m.theme);
    ui.toggle("Rainbow sections", "Off uses your accent color everywhere", &mut m.rainbow);
    if !m.rainbow { ui.color("Accent color", &mut m.accent); }
    ui.heading("Feel");
    ui.slider("Size", &mut m.size, 0.85, 1.35, percent);
    ui.slider("Opacity", &mut m.opacity, 0.6, 1.0, percent);
    ui.slider("Animation speed", &mut m.speed, 0.5, 2.0, percent);
    let k = ui.k;
    ui.y += 8.0 * k;
    match ui.buttons(&["Try it now", "Reset menu"]) {
        Some(0) => out.try_menu = true,
        Some(_) => {
            *m = Default::default();
            ui.changed = true;
        }
        None => {}
    }
}

fn page_general(ui: &mut Ui, s: &mut Settings) {
    ui.heading("Unleashed mode");
    let was = s.unleashed;
    ui.toggle("Remove all limits", "Any number of buddies, giant sizes, extreme physics. Your PC may struggle", &mut s.unleashed);
    if was && !s.unleashed { s.fit(); }
    if s.unleashed { ui.para("If it gets too heavy, press Ctrl+Alt+Shift+R to remove every buddy.", 1.0); }
    ui.heading("Options");
    ui.toggle("Chatty", "Speech bubbles on their own", &mut s.chatty);
    ui.toggle("Type along when I type", "Only detects that a key was pressed, never which", &mut s.type_along);
    ui.toggle("Pause buddies while menu is open", "Off keeps them moving behind the menu", &mut s.pause_menu);
    ui.toggle("Menu sounds", "Small clicks when using the menu", &mut s.sounds);
    let k = ui.k;
    ui.heading("Reset");
    if ui.buttons(&["Reset everything to defaults"]).is_some() {
        *s = Settings::default();
        ui.changed = true;
    }
    ui.y += 4.0 * k;
}

fn page_about(ui: &mut Ui) {
    ui.heading("Controls");
    ui.para("Drag to carry, flick to throw, click to poke, double-click for a backflip, scroll to resize, right-click for the menu.", 2.0);
    ui.para("In the menu: P or middle-click pins a favourite, Shift+arrows reorders pins, Esc goes back.", 2.0);
    ui.heading("Hotkeys (Ctrl+Alt+Shift)");
    ui.para("B menu   ·   H hide or show   ·   S summon everyone   ·   C clear   ·   G select a group   ·   R remove all", 1.0);
    ui.heading("Privacy");
    ui.para("No telemetry, no accounts and no network use beyond a local port on your own PC. Typing detection never sees which keys you press.", 2.0);
    ui.heading("About");
    ui.para("Deskling is basically Clawd living on your desktop. It's an unofficial, fan-made project, not made by or affiliated with Anthropic.", 2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_maps_both_ways_and_stays_in_range() {
        assert_eq!(slide_value(0.5, 0.0, 2.0), 1.0);
        assert_eq!(slide_value(-3.0, 1.0, 2.0), 1.0);
        assert_eq!(slide_value(9.0, 1.0, 2.0), 2.0);
        assert!((slide_pos(slide_value(0.25, 0.2, 2.5), 0.2, 2.5) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn colour_channels_change_alone() {
        assert_eq!(with_channel(0x112233, 8, 0xFF), 0x11FF33);
        assert_eq!(with_channel(0x112233, 16, 0), 0x002233);
    }

    #[test]
    fn preview_patterns_match_the_shader_rule() {
        let (a, b) = ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(body_color(a, b, 0, 3, 3), a);
        assert_eq!(body_color(a, b, 2, 3, 3), b);
        assert_eq!(body_color(a, b, 2, 3, 4), a);
        assert_eq!(body_color(a, b, 3, 1, 2), b);
        assert_eq!(body_color(a, b, 1, 0, 8), b);
    }
}

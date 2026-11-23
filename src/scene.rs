//! What's on screen: every buddy, bubble, particle, drawing and prop. It simulates effects at a fixed step
//! and draws at the display's rate, blending between the last two steps so motion stays smooth.
use std::collections::{HashMap, HashSet};
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, ID2D1PathGeometry};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

use crate::brain::{Brain, Event, InkStyle, Mode, Rng};
use crate::gpu::{pt, Color, Gpu, Instance};
use crate::menu::{self, Marquee, Wheel};
use crate::props::{Prop, PropEvent, Props};
use crate::settings::Skin;
use crate::shapes;
use crate::sprites;
use crate::wear::{self, Slot, Wear};

const MAX_INK: usize = 400;
const MAX_PARTICLES: usize = 900;
const MAX_BUBBLES: usize = 14;       // in a big crowd, only the newest speak at once
const INK: Color = [0.11, 0.11, 0.11, 1.0];
const SQ_LOW: f64 = 0.6;
const SQ_HIGH: f64 = 1.4;
const SQ_STEP: f64 = 0.04;
const CONFETTI: [u32; 7] = [0xFF4D6D, 0xFFD23F, 0x4DD599, 0x4D9DFF, 0xB36BFF, 0xFF8A3D, 0xD77757];
const FLAME: [u32; 4] = [0xFFE45C, 0xFFB627, 0xFF7A1A, 0xE8441A];
const CLOUD_PUFFS: [(f32, f32, f32); 5] = [(0.0, 0.08, 0.36), (-0.34, 0.04, 0.26), (-0.12, -0.1, 0.3), (0.14, -0.06, 0.28), (0.36, 0.05, 0.22)];

fn hex(v: u32) -> Color {
    let c = sprites::rgb(v);
    [c[0], c[1], c[2], 1.0]
}

/// A colour by #hex or by common name; anything unknown is red.
pub fn parse_color(name: &str) -> Color {
    let n = name.trim().to_lowercase();
    if let Some(h) = n.strip_prefix('#') {
        if h.len() == 6 { if let Ok(v) = u32::from_str_radix(h, 16) { return hex(v); } }
        if h.len() == 3 {
            if let Ok(v) = u32::from_str_radix(h, 16) {
                let (r, g, b) = ((v >> 8) & 15, (v >> 4) & 15, v & 15);
                return hex((r * 17) << 16 | (g * 17) << 8 | (b * 17));
            }
        }
        return hex(0xFF0000);
    }
    hex(match n.as_str() {
        "black" => 0x000000, "white" => 0xFFFFFF, "gray" | "grey" => 0x808080, "silver" => 0xC0C0C0, "red" => 0xFF0000,
        "green" => 0x00A000, "lime" => 0x00FF00, "blue" => 0x0000FF, "navy" => 0x000080, "sky" | "skyblue" => 0x87CEEB,
        "cyan" | "aqua" => 0x00FFFF, "teal" => 0x008080, "yellow" => 0xFFFF00, "gold" => 0xFFD700, "orange" => 0xFFA500,
        "pink" => 0xFFC0CB, "hotpink" => 0xFF69B4, "magenta" | "fuchsia" => 0xFF00FF, "purple" => 0x800080, "violet" => 0xEE82EE,
        "brown" => 0xA52A2A, "maroon" => 0x800000, "olive" => 0x808000, "coral" => 0xFF7F50, "salmon" => 0xFA8072,
        "tomato" => 0xFF6347, "turquoise" => 0x40E0D0, "indigo" => 0x4B0082, "beige" => 0xF5F5DC, _ => 0xFF0000,
    })
}

#[derive(Clone)]
enum Look { Box(Color), Oval(Color, Option<Color>), Heart, Star, Text(String, u8, Color) }

#[derive(Clone)]
struct Particle {
    look: Look, x: f32, y: f32, vx: f32, vy: f32, g: f32, drag: f32, life: f32, age: f32, w: f32, h: f32,
    flutter: f32, shrink: bool, sway: f32, phase: f32, burst: Option<Color>, grow: f32,
}

impl Particle {
    fn new(look: Look, x: f32, y: f32, w: f32, h: f32, life: f32) -> Self {
        Particle { look, x, y, vx: 0.0, vy: 0.0, g: 0.0, drag: 0.0, life, age: 0.0, w, h, flutter: 0.0, shrink: false, sway: 0.0, phase: 0.0, burst: None, grow: 0.0 }
    }
}

struct Ring { x: f32, y: f32, r: f32, speed: f32, life: f32, age: f32, width: f32, color: Color }

struct Stroke { pts: Vec<windows::Win32::Graphics::Direct2D::Common::D2D_POINT_2F>, geo: Option<ID2D1PathGeometry>, color: Color, width: f32, ttl: f64, done_at: Option<f64> }

struct TextStroke { text: Vec<char>, x: f32, y: f32, color: Color, font: IDWriteTextFormat, duration: f64, ttl: f64, start: f64, done_at: Option<f64> }

struct Bubble { full: Vec<char>, start: f64, w: f32, h: f32 }

struct Bit { x: f32, y: f32, vx: f32, vy: f32, fx: f32, fy: f32, col: f32, row: f32, color: Color }

struct Blast { bits: Vec<Bit>, end: f64, floor: f32 }

struct View {
    sq: f64, sq_v: f64, prev_mode: Mode, blast: Option<Blast>, bubble: Option<Bubble>, bubble_id: u64, next_zzz: f64,
    last: (f64, f64), cur: (f64, f64),
}

type Picture = Option<(Arc<Wear>, ID2D1Bitmap1)>;

pub struct Scene {
    pub skin: Skin,
    hat_img: Picture,
    glasses_img: Picture,
    pub vx: f64,
    pub vy: f64,
    vw: f32,
    vh: f32,
    ui: f32,
    rng: Rng,
    views: HashMap<u32, View>,
    particles: Vec<Particle>,
    rings: Vec<Ring>,
    ink: HashMap<u64, Stroke>,
    ink_order: Vec<u64>,
    texts: Vec<TextStroke>,
    scheduled: Vec<(f64, Job)>,
    snow_until: f64,
    next_drop: HashMap<u32, f64>,
    instances: Vec<Instance>,
    shown: Vec<Instance>,
    changed: bool,
}

#[derive(Clone, Copy)]
enum Job { Rocket(f32), Star(f32, f32) }

fn lerp(a: f64, b: f64, t: f64) -> f64 { a + (b - a) * t }

impl Scene {
    pub fn new(bounds: (i32, i32, i32, i32), ui: f32) -> Self {
        Scene {
            skin: Skin::default(), hat_img: None, glasses_img: None, vx: bounds.0 as f64, vy: bounds.1 as f64, vw: (bounds.2 - bounds.0) as f32, vh: (bounds.3 - bounds.1) as f32, ui,
            rng: Rng::new(0x5EED), views: HashMap::new(), particles: Vec::new(), rings: Vec::new(), ink: HashMap::new(), ink_order: Vec::new(),
            texts: Vec::new(), scheduled: Vec::new(), snow_until: 0.0, next_drop: HashMap::new(), instances: Vec::new(), shown: Vec::new(), changed: true,
        }
    }

    pub fn set_bounds(&mut self, bounds: (i32, i32, i32, i32)) {
        self.vx = bounds.0 as f64;
        self.vy = bounds.1 as f64;
        self.vw = (bounds.2 - bounds.0) as f32;
        self.vh = (bounds.3 - bounds.1) as f32;
        self.changed = true;
    }

    fn r(&mut self, a: f32, b: f32) -> f32 { self.rng.range(a as f64, b as f64) as f32 }
    fn room(&self) -> bool { self.particles.len() < MAX_PARTICLES }
    fn add(&mut self, mut p: Particle) {
        if self.room() {
            p.phase = self.r(0.0, TAU);
            self.particles.push(p);
        }
    }

    // ---- one fixed simulation step ----------------------------------------------------

    pub fn step(&mut self, brains: &[Brain], props: &Props, events: Vec<(u32, Vec<Event>)>, prop_events: Vec<PropEvent>, now: f64, dt: f64, gpu: Option<&Gpu>) {
        let alive: HashSet<u32> = brains.iter().map(|b| b.id).collect();
        self.views.retain(|id, _| alive.contains(id));
        for b in brains {
            self.views.entry(b.id).or_insert_with(|| View { sq: 1.0, sq_v: 0.0, prev_mode: b.mode, blast: None, bubble: None, bubble_id: 0, next_zzz: 0.0, last: (b.x, b.y), cur: (b.x, b.y) });
        }
        let slots: HashMap<u32, usize> = if events.is_empty() { HashMap::new() } else { brains.iter().enumerate().map(|(i, b)| (b.id, i)).collect() };
        for (id, evs) in events {
            let Some(b) = slots.get(&id).map(|&i| &brains[i]) else { continue };
            for ev in evs { self.event(ev, b, now, gpu); }
        }
        for ev in prop_events { self.prop_event(ev); }
        self.update_props(props, now, dt as f32);
        let due: Vec<Job> = self.scheduled.iter().filter(|j| j.0 <= now).map(|j| j.1).collect();
        self.scheduled.retain(|j| j.0 > now);
        for job in due {
            match job {
                Job::Rocket(x) => self.rocket(x),
                Job::Star(x, y) => {
                    let life = self.r(0.4, 0.9);
                    let mut p = Particle::new(Look::Star, x, y, 0.0, 0.0, life);
                    p.vy = -30.0 * self.ui;
                    self.add(p);
                }
            }
        }
        if now < self.snow_until { self.snow(dt as f32); }
        self.update_particles(dt as f32);
        self.update_ink(now);
        let mut zzz = Vec::new();
        let mut trail = Vec::new();
        for b in brains {
            let Some(v) = self.views.get_mut(&b.id) else { continue };
            v.last = v.cur;
            v.cur = (b.x, b.y);
            if self.skin.trail > 0 && (v.cur.0 - v.last.0).abs() + (v.cur.1 - v.last.1).abs() > 0.4 * b.unit * dt * 60.0 {
                trail.push(((b.x - self.vx) as f32, (b.y - self.vy) as f32 - 2.0 * b.unit as f32, b.unit as f32));
            }
            spring(v, b, dt);
            if b.mode == Mode::Sleep && now >= v.next_zzz {
                v.next_zzz = now + 1.8;
                zzz.push(((b.x - self.vx) as f32 + 6.0 * b.unit as f32, (b.y - self.vy - sprites::H as f64 * b.unit) as f32, b.unit as f32));
            }
            update_blast(v, b, now, dt as f32, self.vx as f32, self.vy as f32);
            update_bubble(v, b, now, gpu);
            v.prev_mode = b.mode;
        }
        // A shared budget, so 200 buddies can't flood the particle pool.
        let chance = (dt as f32 * 9.0).min(1.0) * (40.0 / trail.len().max(40) as f32);
        for (x, y, u) in trail {
            if (self.rng.f() as f32) < chance {
                let (dx, life) = (self.r(-5.0, 5.0) * u, self.r(0.5, 0.9));
                let mut p = Particle::new(if self.skin.trail == 1 { Look::Star } else { Look::Heart }, x + dx, y, 0.0, 0.0, life);
                p.vy = -10.0 * u;
                self.add(p);
            }
        }
        for (x, y, u) in zzz {
            let text = if self.rng.f() < 0.5 { "z" } else { "Z" };
            let mut p = Particle::new(Look::Text(text.into(), 2, hex(0x5B6BD6)), x, y, 0.0, 0.0, 2.0);
            p.vx = 6.0 * u;
            p.vy = -14.0 * u;
            self.add(p);
        }
    }

    fn event(&mut self, ev: Event, b: &Brain, now: f64, gpu: Option<&Gpu>) {
        let u = b.unit as f32;
        let cx = (b.x - self.vx) as f32;
        let feet = (b.y - self.vy) as f32;
        let head = feet - sprites::H as f32 * u;
        match ev {
            Event::Heart => {
                let dx = self.r(-6.0, 6.0) * u;
                let mut p = Particle::new(Look::Heart, cx + dx, head, 0.0, 0.0, 1.2);
                p.vy = -22.0 * u;
                self.add(p);
            }
            Event::Glyph(text) => {
                let color = if text == "!" { hex(0xE8443A) } else { hex(0x5B6BD6) };
                let (dx, vx) = (self.r(-5.0, 5.0) * u, self.r(-6.0, 6.0) * u);
                let mut p = Particle::new(Look::Text(text.into(), 1, color), cx + dx, head, 0.0, 0.0, 1.1);
                p.vx = vx;
                p.vy = -26.0 * u;
                self.add(p);
            }
            Event::Impact { power, x, y, wall } => {
                if !wall { if let Some(v) = self.views.get_mut(&b.id) { v.sq_v -= 2.0 + 9.0 * power; } }
                self.dust((x - self.vx) as f32, (y - self.vy) as f32, 3 + (10.0 * power) as usize, u);
            }
            Event::Dust { x, y } => self.dust((x - self.vx) as f32, (y - self.vy) as f32, 2, u),
            Event::Smoke => self.puff(cx, head, 6.0 * u, hex(0x5E5E5E), 1, 1.0),
            Event::Stars => {
                for _ in 0..5 {
                    let a = self.r(PI * 1.1, PI * 1.9);
                    let s = self.r(30.0, 60.0) * u;
                    let mut p = Particle::new(Look::Star, cx, head, 0.0, 0.0, 0.8);
                    p.vx = a.cos() * s;
                    p.vy = a.sin() * s;
                    p.g = 60.0 * u;
                    self.add(p);
                }
            }
            Event::Explode { seconds } => self.explode(b, cx, feet, now, seconds),
            Event::Fx { kind, x, y } => self.fx(kind, (x - self.vx) as f32, (y - self.vy) as f32, now),
            Event::Ink { id, pts, style, done } => self.ink_event(id, pts, style, done, now),
            Event::Text { x, y, text, color, size, ttl, duration } => {
                if let Some(font) = gpu.and_then(|g| g.handwriting(size as f32).ok()) {
                    self.texts.push(TextStroke { text: text.chars().collect(), x: (x - self.vx) as f32, y: (y - self.vy) as f32, color: parse_color(&color), font, duration: duration.max(0.05), ttl, start: now, done_at: None });
                }
            }
            Event::Clear => {
                self.ink.clear();
                self.ink_order.clear();
                self.texts.clear();
                self.changed = true;
            }
            Event::Prop(_) => {}
        }
    }

    fn ink_event(&mut self, id: u64, pts: Vec<shapes::P>, style: Option<InkStyle>, done: bool, now: f64) {
        if let Some(s) = style {
            self.ink.insert(id, Stroke { pts: Vec::new(), geo: None, color: parse_color(&s.color), width: s.width.max(1.0) as f32, ttl: s.ttl, done_at: None });
            self.ink_order.push(id);
            if self.ink_order.len() > MAX_INK {
                let old = self.ink_order.remove(0);
                self.ink.remove(&old);
            }
        }
        let (vx, vy) = (self.vx, self.vy);
        if let Some(stroke) = self.ink.get_mut(&id) {
            if !pts.is_empty() {
                stroke.pts.extend(pts.iter().map(|p| pt((p.0 - vx) as f32, (p.1 - vy) as f32)));
                stroke.geo = None;
            }
            if done { stroke.done_at = Some(now); }
        }
        self.changed = true;
    }

    fn update_ink(&mut self, now: f64) {
        let before = self.ink.len();
        self.ink.retain(|_, s| s.done_at.is_none() || s.ttl <= 0.0 || now - s.done_at.unwrap() < s.ttl);
        if self.ink.len() != before {
            let ink = &self.ink;
            self.ink_order.retain(|id| ink.contains_key(id));
            self.changed = true;
        }
        let count = self.texts.len();
        self.texts.retain_mut(|t| {
            if now - t.start >= t.duration && t.done_at.is_none() { t.done_at = Some(now); }
            match t.done_at { Some(d) if t.ttl > 0.0 => now - d < t.ttl, _ => true }
        });
        if self.texts.len() != count || self.texts.iter().any(|t| now - t.start < t.duration + 0.1) { self.changed = true; }
    }

    fn dust(&mut self, x: f32, y: f32, count: usize, u: f32) {
        for _ in 0..count {
            let side = if self.rng.f() < 0.5 { -1.0 } else { 1.0 };
            let size = self.r(1.0, 2.2) * u;
            let dx = side * self.r(2.0, 6.0) * u;
            let mut p = Particle::new(Look::Box(hex(0xC9C1B8)), x + dx, y - size, size, size, 0.5);
            p.vx = side * self.r(20.0, 60.0) * u;
            p.vy = -self.r(5.0, 25.0) * u;
            p.g = 40.0 * u;
            p.drag = 3.0;
            p.shrink = true;
            self.add(p);
        }
    }

    fn puff(&mut self, x: f32, y: f32, size: f32, color: Color, count: usize, life: f32) {
        for _ in 0..count {
            let r = self.r(0.15, 0.3) * size;
            let (dx, dy) = (self.r(-0.4, 0.4) * size, self.r(0.0, 0.3) * size);
            let l = self.r(life * 0.6, life);
            let mut p = Particle::new(Look::Oval(color, None), x + dx, y - dy, r, r, l);
            p.vx = self.r(-20.0, 20.0) * self.ui;
            p.vy = -self.r(30.0, 70.0) * self.ui;
            p.drag = 0.8;
            p.grow = 0.8;
            p.sway = 20.0 * self.ui;
            self.add(p);
        }
    }

    fn update_particles(&mut self, dt: f32) {
        let mut bursts = Vec::new();
        self.particles.retain_mut(|p| {
            p.age += dt;
            if p.age >= p.life {
                if let Some(c) = p.burst { bursts.push((p.x, p.y, c)); }
                return false;
            }
            p.vy += p.g * dt;
            if p.drag != 0.0 {
                let k = (1.0 - p.drag * dt).max(0.0);
                p.vx *= k;
                p.vy *= k;
            }
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            if p.sway != 0.0 { p.x += (p.age * 3.0 + p.phase).sin() * p.sway * dt; }
            true
        });
        for (x, y, c) in bursts { self.burst(x, y, c); }
        self.rings.retain_mut(|r| {
            r.age += dt;
            r.r += r.speed * dt;
            r.age < r.life
        });
    }

    fn pick_hex(&mut self, list: &[u32]) -> Color { hex(self.rng.pick(list)) }

    fn fx(&mut self, kind: &str, x: f32, y: f32, now: f64) {
        let ui = self.ui;
        match kind {
            "confetti" => for _ in 0..120 {
                let a = self.r(-PI * 0.8, -PI * 0.2);
                let s = self.r(500.0, 1300.0) * ui;
                let w = self.r(5.0, 9.0) * ui;
                let c = self.pick_hex(&CONFETTI);
                let life = self.r(2.5, 4.0);
                let mut p = Particle::new(Look::Box(c), x, y, w, w * 0.6, life);
                p.vx = a.cos() * s;
                p.vy = a.sin() * s;
                p.g = 700.0 * ui;
                p.drag = 2.2;
                p.flutter = self.r(6.0, 12.0);
                p.sway = 40.0 * ui;
                self.add(p);
            },
            "fireworks" => for i in 0..5 {
                let rx = (x + self.r(-450.0, 450.0) * ui).max(40.0).min(self.vw - 40.0);
                self.scheduled.push((now + i as f64 * 0.35, Job::Rocket(rx)));
            },
            "hearts" => for _ in 0..14 {
                let a = self.r(-PI, 0.0);
                let s = self.r(60.0, 220.0) * ui;
                let life = self.r(1.4, 2.4);
                let mut p = Particle::new(Look::Heart, x, y, 0.0, 0.0, life);
                p.vx = a.cos() * s;
                p.vy = a.sin() * s - 40.0 * ui;
                p.g = -30.0 * ui;
                p.drag = 1.5;
                self.add(p);
            },
            "sparkle" => for i in 0..18 {
                let (px, py) = (x + self.r(-70.0, 70.0) * ui, y + self.r(-60.0, 40.0) * ui);
                self.scheduled.push((now + i as f64 * 0.04, Job::Star(px, py)));
            },
            "snow" => self.snow_until = now + 8.0,
            _ => {}
        }
    }

    fn rocket(&mut self, x: f32) {
        let color = self.pick_hex(&CONFETTI);
        let rise = self.r(0.9, 1.3);
        let speed = self.r(self.vh * 0.9, self.vh * 1.15) / rise;
        let mut p = Particle::new(Look::Box(hex(0xFFE9A8)), x, self.vh, 4.0 * self.ui, 10.0 * self.ui, rise);
        p.vy = -speed;
        p.g = speed / rise * 0.9;
        p.burst = Some(color);
        self.add(p);
    }

    fn burst(&mut self, x: f32, y: f32, color: Color) {
        let ui = self.ui;
        for i in 0..36 {
            let a = i as f32 / 36.0 * TAU + self.r(-0.05, 0.05);
            let s = self.r(180.0, 340.0) * ui;
            let c = if self.rng.f() < 0.33 { hex(0xFFFFFF) } else { color };
            let life = self.r(0.9, 1.4);
            let mut p = Particle::new(Look::Oval(c, None), x, y, 5.0 * ui, 5.0 * ui, life);
            p.vx = a.cos() * s;
            p.vy = a.sin() * s;
            p.g = 220.0 * ui;
            p.drag = 1.6;
            p.shrink = true;
            self.add(p);
        }
    }

    fn snow(&mut self, dt: f32) {
        let ui = self.ui;
        let rate = dt * 30.0;
        let n = rate as usize + usize::from(self.rng.f() < (rate % 1.0) as f64);
        for _ in 0..n {
            let r = self.r(3.0, 6.0) * ui;
            let x = self.r(0.0, self.vw);
            let mut p = Particle::new(Look::Oval(hex(0xFFFFFF), Some(hex(0xB9D7F2))), x, -10.0, r, r, self.vh / (70.0 * ui));
            p.vy = self.r(60.0, 110.0) * ui;
            p.sway = 50.0 * ui;
            self.add(p);
        }
    }

    fn prop_event(&mut self, ev: PropEvent) {
        let ui = self.ui;
        match ev {
            PropEvent::Steam { x, y, size } => {
                let (x, y, size) = ((x - self.vx) as f32, (y - self.vy) as f32, size as f32);
                self.puff(x, y - size * 0.3, size, hex(0xD9DEE5), 10, 1.6);
            }
            PropEvent::Strike { x, y } => {
                let (x, y) = ((x - self.vx) as f32, (y - self.vy) as f32);
                for _ in 0..16 {
                    let a = self.r(PI, TAU);
                    let s = self.r(120.0, 320.0) * ui;
                    let c = self.pick_hex(&[0xFFF6A8, 0xFFD23F, 0xFFFFFF]);
                    let mut p = Particle::new(Look::Box(c), x, y, 4.0 * ui, 4.0 * ui, 0.5);
                    p.vx = a.cos() * s;
                    p.vy = a.sin() * s;
                    p.g = 600.0 * ui;
                    p.shrink = true;
                    self.add(p);
                }
                self.rings.push(Ring { x, y, r: 0.0, speed: 300.0 * ui, life: 0.35, age: 0.0, width: (3.0 * ui).round().max(2.0), color: hex(0xFFF6A8) });
            }
            PropEvent::Boom { x, y, size } => {
                let (x, y, size) = ((x - self.vx) as f32, (y - self.vy) as f32, size as f32);
                for _ in 0..70 {
                    let a = self.r(0.0, TAU);
                    let s = self.r(150.0, 650.0) * ui * size / 50.0;
                    let w = self.r(5.0, 12.0) * ui;
                    let c = self.pick_hex(&[FLAME[0], FLAME[1], FLAME[2], FLAME[3], 0x4A4A4A, 0x2B2B2B]);
                    let life = self.r(0.5, 1.1);
                    let mut p = Particle::new(Look::Box(c), x, y, w, w, life);
                    p.vx = a.cos() * s;
                    p.vy = a.sin() * s - 120.0 * ui;
                    p.g = 500.0 * ui;
                    p.drag = 2.0;
                    p.shrink = true;
                    self.add(p);
                }
                self.puff(x, y, size * 2.2, hex(0x6E6E6E), 12, 2.0);
                self.rings.push(Ring { x, y, r: 0.0, speed: 900.0 * ui * size / 50.0, life: 0.4, age: 0.0, width: (5.0 * ui).round().max(3.0), color: hex(0xFFB627) });
                self.rings.push(Ring { x, y, r: 0.0, speed: 600.0 * ui * size / 50.0, life: 0.3, age: 0.0, width: (3.0 * ui).round().max(2.0), color: hex(0xFFFFFF) });
                let mut p = Particle::new(Look::Text("BOOM!".into(), 3, hex(0xFF4D1A)), x, y - size, 0.0, 0.0, 0.9);
                p.vy = -60.0 * ui;
                self.add(p);
            }
        }
    }

    fn update_props(&mut self, props: &Props, now: f64, dt: f32) {
        let ui = self.ui;
        let live: Vec<Prop> = props.items.clone();
        self.next_drop.retain(|id, _| live.iter().any(|p| p.id == *id));
        for p in &live {
            let (x, y, size) = ((p.x - self.vx) as f32, (p.y - self.vy) as f32, p.size as f32);
            match p.kind {
                "fire" => {
                    let rate = dt * 70.0 * size / 70.0;
                    let n = rate as usize + usize::from(self.rng.f() < (rate % 1.0) as f64);
                    for _ in 0..n {
                        let off = (self.r(-1.0, 1.0) + self.r(-1.0, 1.0)) * 0.17 * 0.7;
                        let w = size * self.r(0.12, 0.24) * (1.0 - (off.abs() * 1.5).min(0.8));
                        let idx = ((off.abs() * 9.0 + self.r(0.0, 1.5)) as usize).min(3);
                        let dy = self.r(0.0, 0.12) * size;
                        let life = self.r(0.35, 0.75);
                        let mut f = Particle::new(Look::Box(hex(FLAME[idx])), x + off * size, y - dy, w, w, life);
                        f.vx = -off * size * 0.6;
                        f.vy = -size * self.r(1.4, 2.4);
                        f.shrink = true;
                        f.sway = size * 0.35;
                        self.add(f);
                    }
                    if self.rng.f() < (dt * 4.0) as f64 {
                        let dx = self.r(-0.3, 0.3) * size;
                        let mut e = Particle::new(Look::Box(hex(0xFFB347)), x + dx, y - size * 0.5, 3.0 * ui, 3.0 * ui, 1.3);
                        e.vy = -size * 1.1;
                        e.sway = size * 0.8;
                        e.shrink = true;
                        self.add(e);
                    }
                }
                "cloud" => {
                    let mut next = self.next_drop.get(&p.id).copied().unwrap_or(0.0) - dt as f64;
                    while next <= 0.0 {
                        next += 1.0 / 40.0;
                        let speed = 700.0 * ui;
                        let dx = self.r(-0.5, 0.5) * size;
                        let life = ((self.vh - y) / speed).clamp(0.2, 2.5);
                        let mut d = Particle::new(Look::Box(hex(0x5DA9F5)), x + dx, y + size * 0.25, 2.0 * ui, 11.0 * ui, life);
                        d.vy = speed;
                        self.add(d);
                    }
                    self.next_drop.insert(p.id, next);
                }
                "bomb" if self.rng.f() < (dt * 25.0) as f64 => {
                    let r = size * 0.4;
                    let left = if p.landed < 0.0 { 1.0 } else { (1.0 - ((now - p.landed) / 3.0) as f32).max(0.05) };
                    let (tx, ty) = (x + 0.3 * size * left, y - r - 0.22 * size * left);
                    let c = self.pick_hex(&[0xFFD23F, 0xFF7A1A]);
                    let mut s = Particle::new(Look::Box(c), tx, ty, 2.0 * ui, 2.0 * ui, 0.3);
                    s.vx = self.r(-80.0, 80.0) * ui;
                    s.vy = self.r(-90.0, 10.0) * ui;
                    s.g = 300.0 * ui;
                    self.add(s);
                }
                _ => {}
            }
        }
    }

    fn explode(&mut self, b: &Brain, cx: f32, feet: f32, now: f64, seconds: f64) {
        let u = b.unit as f32;
        let frame = sprites::pose(b.mode, b.sub, b.clock, false);
        let pal = palette(b, &self.skin);
        let (left, top) = (cx - sprites::W as f32 * u / 2.0, feet - sprites::H as f32 * u);
        let mut bits = Vec::new();
        for (col, row, value) in sprites::pixels(frame, b.facing < 0) {
            let (x, y) = (left + col as f32 * u, top + row as f32 * u);
            let (dx, dy) = (x - cx, y - (feet - sprites::H as f32 * u / 2.0));
            let d = dx.hypot(dy).max(1.0);
            let sp = self.r(60.0, 160.0) * u;
            let up = self.r(40.0, 110.0) * u;
            bits.push(Bit { x, y, vx: dx / d * sp, vy: dy / d * sp - up, fx: x, fy: y, col: col as f32, row: row as f32, color: pal[value as usize] });
        }
        if let Some(v) = self.views.get_mut(&b.id) { v.blast = Some(Blast { bits, end: now + seconds - 0.2, floor: feet }); }
        self.fx("sparkle", cx, feet - sprites::H as f32 * u / 2.0, now);
    }

    // ---- drawing ----------------------------------------------------------------------

    /// True when this frame would look the same as the last one, so nothing has to be drawn.
    fn is_still(&self) -> bool {
        !self.changed && self.particles.is_empty() && self.rings.is_empty() && self.scheduled.is_empty()
            && self.views.values().all(|v| v.bubble.is_none() && v.blast.is_none()) && self.instances == self.shown
    }

    pub fn set_wear(&mut self, gpu: &Gpu, hat: Option<Arc<Wear>>, glasses: Option<Arc<Wear>>) {
        let update = |slot: &mut Picture, next: Option<Arc<Wear>>| {
            let same = match (&*slot, &next) {
                (Some(a), Some(b)) => Arc::ptr_eq(&a.0, b),
                (None, None) => true,
                _ => false,
            };
            if !same { *slot = next.and_then(|w| gpu.image(&w).ok().map(|b| (w, b))); }
        };
        update(&mut self.hat_img, hat);
        update(&mut self.glasses_img, glasses);
    }

    /// Something outside the scene changed, so the next frame must be drawn.
    pub fn touch(&mut self) { self.changed = true; }

    pub fn draw(&mut self, gpu: &mut Gpu, brains: &[Brain], props: &Props, wheel: Option<&Wheel>, marquee: Option<&Marquee>, now: f64, alpha: f64) -> windows::core::Result<bool> {
        self.instances.clear();
        let (ox, oy) = (self.vx, self.vy);
        for b in brains {
            let Some(v) = self.views.get(&b.id) else { continue };
            if v.blast.is_some() { continue; }
            let jump = (v.cur.0 - v.last.0).abs() + (v.cur.1 - v.last.1).abs();
            let (x, y) = if b.mode == Mode::Held || jump > 200.0 * b.unit { (b.x, b.y) } else { (lerp(v.last.0, v.cur.0, alpha), lerp(v.last.1, v.cur.1, alpha)) };
            let u = b.unit;
            let angle = (b.angle / 15.0).round() * 15.0 % 360.0;
            let sy = if angle != 0.0 { 1.0 } else { ((v.sq.clamp(SQ_LOW, SQ_HIGH) / SQ_STEP).round() * SQ_STEP * 100.0).round() / 100.0 };
            let mut shake = 0.0;
            if b.mode == Mode::Act && (b.sub == "angry" || b.sub == "scared") {
                shake = (b.clock * 70.0).sin() * u * if b.sub == "angry" { 0.6 } else { 0.35 };
            }
            let lift = if b.mode == Mode::Act && matches!(b.sub, "dance" | "happy" | "celebrate") { (b.clock * 10.0).sin().abs() * 4.0 * u } else { 0.0 };
            let frame = sprites::pose(b.mode, b.sub, b.clock, b.looking);
            let pal = palette(b, &self.skin);
            let plain = b.charred();
            let t2 = if self.skin.custom { sprites::rgb(self.skin.color2) } else { [pal[1][0] * 0.7, pal[1][1] * 0.7, pal[1][2] * 0.7] };
            let on_ground = b.mode.standing() || b.mode == Mode::Slide;
            self.instances.push(Instance {
                x: (x - ox + shake).round() as f32, y: (y - oy - lift).round() as f32, unit: u as f32, sy: sy as f32,
                angle: (angle as f32).to_radians(), frame: frame as f32, flip: if b.facing < 0 { 1.0 } else { 0.0 },
                eye: pal[sprites::EYE as usize][0], tint: [pal[1][0], pal[1][1], pal[1][2]], shadow: if on_ground && lift == 0.0 { 1.0 } else { 0.0 },
                tint2: t2, pattern: if plain { 0.0 } else { self.skin.pattern as f32 },
            });
        }
        // A held buddy draws above the crowd.
        if let Some(i) = brains.iter().position(|b| b.mode == Mode::Held) {
            if i < self.instances.len() {
                let last = self.instances.len() - 1;
                self.instances.swap(i, last);
            }
        }
        if wheel.is_none() && marquee.is_none() && self.is_still() && props.items.is_empty() && brains.iter().all(|b| b.mode != Mode::Pen) { return Ok(false); }

        gpu.clear();
        gpu.begin();
        for id in &self.ink_order {
            let Some(s) = self.ink.get_mut(id) else { continue };
            if s.pts.len() < 2 { continue; }
            if s.geo.is_none() { s.geo = gpu.path(&s.pts, false).ok(); }
            if let Some(g) = &s.geo { gpu.stroke(g, s.width, s.color); }
        }
        for t in &self.texts {
            let frac = ((now - t.start) / t.duration).min(1.0);
            let n = ((t.text.len() as f64 * frac).round() as usize).clamp(1, t.text.len());
            let text: String = t.text[..n].iter().collect();
            if let Ok(l) = gpu.layout(&text, &t.font, 2000.0) { gpu.draw_layout(&l, t.x, t.y, t.color); }
        }
        for p in &props.items { self.draw_prop(gpu, p, now)?; }
        for p in &self.particles { draw_particle(gpu, p, self.ui); }
        for r in &self.rings { gpu.ring(r.x, r.y, r.r, r.r * 0.6, r.width, r.color); }
        for v in self.views.values() {
            if let Some(blast) = &v.blast {
                for (b, bit) in brains.iter().filter(|b| self.views.get(&b.id).is_some_and(|o| std::ptr::eq(o, v))).flat_map(|b| blast.bits.iter().map(move |bit| (b, bit))) {
                    let u = b.unit as f32;
                    gpu.rect(bit.x, bit.y, u, u, bit.color);
                }
            }
        }
        gpu.end()?;
        gpu.draw_sprites(&self.instances, if self.skin.glow { self.skin.glow_power as f32 } else { 0.0 })?;
        gpu.begin();
        if self.skin.hat > 0 || self.skin.glasses > 0 || self.hat_img.is_some() || self.glasses_img.is_some() {
            for i in &self.instances { draw_wear(gpu, i, &self.skin, &self.hat_img, &self.glasses_img); }
        }
        for b in brains { self.draw_tools(gpu, b); }
        let mut speaking: Vec<(&Brain, &View)> = brains.iter().filter_map(|b| self.views.get(&b.id).filter(|v| v.bubble.is_some()).map(|v| (b, v))).collect();
        speaking.sort_by(|a, b| b.1.bubble.as_ref().unwrap().start.total_cmp(&a.1.bubble.as_ref().unwrap().start));
        for (b, v) in speaking.into_iter().take(MAX_BUBBLES).rev() { draw_bubble(gpu, v, b, ox as f32, oy as f32, self.vw, self.ui, now, alpha)?; }
        if let Some(w) = wheel {
            for b in brains.iter().filter(|b| w.group.binary_search(&b.id).is_ok()) {
                let u = b.unit as f32;
                gpu.ring((b.x - ox) as f32, (b.y - oy) as f32, 8.0 * u, 2.2 * u, 2.0 * self.ui, hex(0xFFD23F));
            }
            let buddy = brains.iter().find(|b| b.id == w.buddy).map(|b| (b.x as f32, b.y as f32, b.facing as f32, (b.y - sprites::H as f64 * b.unit * 0.5) as f32));
            menu::draw(gpu, w, ox as f32, oy as f32, buddy)?;
        }
        if let Some(m) = marquee {
            for b in brains {
                let u = b.unit as f32;
                let (x, feet) = (b.x as f32, b.y as f32);
                if m.touches(x, feet - sprites::H as f32 * u, feet, sprites::W as f32 * u / 2.0) {
                    gpu.ring(x - ox as f32, feet - oy as f32, 8.0 * u, 2.2 * u, 2.0 * self.ui, hex(0xFFD23F));
                }
            }
            menu::draw_marquee(gpu, m, ox as f32, oy as f32, self.ui)?;
        }
        gpu.end()?;
        gpu.present()?;
        std::mem::swap(&mut self.shown, &mut self.instances);
        self.changed = false;
        Ok(true)
    }

    fn draw_tools(&self, gpu: &Gpu, b: &Brain) {
        let u = b.unit as f32;
        let (cx, feet) = ((b.x - self.vx) as f32, (b.y - self.vy) as f32);
        if b.mode == Mode::Act && b.sub == "dizzy" {
            let head = feet - sprites::H as f32 * u;
            for i in 0..3 {
                let a = b.clock as f32 * 5.0 + i as f32 * TAU / 3.0;
                draw_grid(gpu, &sprites::STAR, 0xFFD23F, cx + a.cos() * 7.0 * u, head + a.sin() * 2.0 * u, (2.0 * self.ui).round());
            }
        }
        if let (Mode::Pen, Some(tip)) = (b.mode, b.pen_tip) {
            let (tx, ty) = ((tip.0 - self.vx) as f32, (tip.1 - self.vy) as f32);
            let side = if b.pen_off >= 0.0 { 1.0 } else { -1.0 };
            let (hx, hy) = (cx + side * 7.0 * u, feet - 6.0 * u);
            let (mx, my) = (hx + (tx - hx) * 0.78, hy + (ty - hy) * 0.78);
            let width = (u * 1.1).round().max(3.0);
            gpu.line((hx, hy), (mx, my), width, hex(0xFFD23F));
            gpu.line((mx, my), (tx, ty), (width - 1.0).max(2.0), INK);
        }
    }

    fn draw_prop(&self, gpu: &Gpu, p: &Prop, now: f64) -> windows::core::Result<()> {
        let ui = self.ui;
        let (x, y, size) = ((p.x - self.vx) as f32, (p.y - self.vy) as f32, p.size as f32);
        let age = (now - p.born) as f32;
        match p.kind {
            "fire" => {
                let flick = 1.0 + 0.12 * (age * 23.0).sin() + 0.08 * (age * 37.0).sin();
                let (gw, gh) = (size * 0.42 * flick, size * 0.16);
                gpu.oval(x, y - gh * 0.2, gw, gh * 0.8, hex(0xFF7A1A));
                gpu.oval(x, y - gh * 0.2, gw * 0.55, gh * 0.5, hex(0xFFE45C));
            }
            "cloud" => {
                let bob = (age * 1.5).sin() * 3.0 * ui;
                for pass in 0..2 {
                    for (dx, dy, r) in CLOUD_PUFFS {
                        let (rx, ry) = (r * size, r * size * 0.7);
                        let (cx, cy) = (x + dx * size, y + dy * size + bob);
                        if pass == 0 { gpu.ring(cx, cy, rx, ry, (2.0 * ui).round().max(1.0), hex(0x9AA7B8)); } else { gpu.oval(cx, cy, rx, ry, hex(0xEEF1F6)); }
                    }
                }
            }
            "bolt" => {
                if age < 0.3 || self.flicker(age) {
                    let pts: Vec<_> = shapes::BOLT.iter().map(|q| pt(x + q.0 as f32 * size, y + q.1 as f32 * size)).collect();
                    let geo = gpu.path(&pts[..pts.len() - 1], true)?;
                    gpu.fill(&geo, if (age * 20.0) as i32 % 2 == 1 { hex(0xFFF6A8) } else { hex(0xFFD23F) });
                    gpu.stroke(&geo, (3.0 * ui).round().max(2.0), hex(0xFFFFFF));
                }
            }
            "bomb" => {
                let r = size * 0.4;
                let blink = p.landed >= 0.0 && now - p.landed > 2.0 && (now * 8.0) as i64 % 2 == 1;
                let left = if p.landed < 0.0 { 1.0 } else { (1.0 - ((now - p.landed) / 3.0) as f32).max(0.05) };
                let (fx1, fy1, fx2, fy2) = (x + 0.12 * size * left, y - r - 0.18 * size * left, x + 0.3 * size * left, y - r - 0.22 * size * left);
                let fuse = gpu.path(&[pt(x, y - r), pt(fx1, fy1), pt(fx2, fy2)], false)?;
                gpu.stroke(&fuse, (3.0 * ui).round().max(2.0), hex(0x8B5A2B));
                gpu.oval(x, y, r, r, if blink { hex(0xC0392B) } else { hex(0x222222) });
                gpu.oval(x - r * 0.375, y - r * 0.425, r * 0.175, r * 0.175, hex(0x6A6A6A));
                draw_grid(gpu, &sprites::STAR, 0xFFD23F, fx2, fy2, (2.0 * ui).round());
            }
            _ => {
                let (x2, y2) = ((p.x2 - self.vx) as f32, (p.y2 - self.vy) as f32);
                let w = (7.0 * ui).round().max(5.0);
                gpu.line((x, y), (x2, y2), w, hex(0x7A5230));
                gpu.line((x, y - w / 3.0), (x2, y2 - w / 3.0), (w / 3.0).floor().max(2.0), hex(0xB07A47));
            }
        }
        Ok(())
    }

    fn flicker(&self, age: f32) -> bool { (age * 47.0).sin() + (age * 91.0).sin() > -0.8 }
}

/// Hats and glasses follow the sprite's squash and rotation.
fn draw_wear(gpu: &Gpu, i: &Instance, skin: &Skin, hat_img: &Picture, glasses_img: &Picture) {
    let (u, uy) = (i.unit, i.unit * i.sy);
    let (left, top) = (i.x - 8.0 * u, i.y - 10.0 * uy);
    let head = 2.0 + sprites::head_drop(i.frame as usize) as f32;
    if i.angle != 0.0 { gpu.turn(i.angle, i.x, i.y - 5.0 * uy); }
    let put = |grid: &[&str], ox: f32, oy: f32| {
        for (r, row) in grid.iter().enumerate() {
            let bytes = row.as_bytes();
            let mut n = 0;
            while n < bytes.len() {
                if bytes[n] == b'.' { n += 1; continue; }
                let (start, ch) = (n, bytes[n]);
                while n < bytes.len() && bytes[n] == ch { n += 1; }
                gpu.rect(left + (ox + start as f32) * u, top + (oy + r as f32) * uy, (n - start) as f32 * u, uy, hex(sprites::wear_color(ch)));
            }
        }
    };
    let picture = |p: &(Arc<Wear>, ID2D1Bitmap1), slot: Slot, size: f64, lift: f64| {
        let (x, y, w, h) = wear::place(slot, p.0.w, p.0.h, head, size as f32, lift as f32);
        gpu.picture(&p.1, left + x * u, top + y * uy, w * u, h * uy, p.0.w.max(p.0.h) <= 64);
    };
    match hat_img {
        Some(p) => picture(p, Slot::Hat, skin.hat_size, skin.hat_lift),
        None => {
            let hat = sprites::HAT_ART[skin.hat as usize];
            if !hat.is_empty() { put(hat, (16 - hat[0].len()) as f32 / 2.0, head - hat.len() as f32); }
        }
    }
    match glasses_img {
        Some(p) => picture(p, Slot::Glasses, skin.glasses_size, skin.glasses_lift),
        None => {
            let glasses = sprites::GLASSES_ART[skin.glasses as usize];
            if !glasses.is_empty() { put(glasses, 2.0, head); }
        }
    }
    if i.angle != 0.0 { gpu.unturn(); }
}

fn palette(b: &Brain, skin: &Skin) -> [Color; 5] {
    let heart = hex(0xFF4D6D);
    if b.charred() { return [[0.0; 4], hex(0x3B3330), hex(0xFFFFFF), hex(0x5A4A44), heart]; }
    let t = if skin.custom { sprites::rgb(skin.color) } else { sprites::tint_rgb(&b.tint) };
    let dark = t[0] * 0.3 + t[1] * 0.59 + t[2] * 0.11 < 0.3;
    let eye = if dark { hex(0xFFFFFF) } else { hex(0x1A1A1A) };
    [[0.0; 4], [t[0], t[1], t[2], 1.0], eye, hex(0xF4A29A), heart]
}

fn spring(v: &mut View, b: &Brain, dt: f64) {
    let u = b.unit;
    let mut target = 1.0;
    match b.mode {
        Mode::Fall => target = 1.0 + (b.vy.abs() / (900.0 * u)).min(0.25),
        Mode::Held => target = 1.1,
        Mode::Sleep => target = 1.0 + 0.04 * (b.clock * 2.0).sin(),
        Mode::Idle | Mode::Pen => target = 1.0 + 0.02 * (b.clock * 3.0).sin(),
        Mode::Act if b.sub == "squat" => target = 0.85,
        _ => {}
    }
    if matches!(b.mode, Mode::Hop | Mode::Fall) && !matches!(v.prev_mode, Mode::Hop | Mode::Fall | Mode::Held) && b.vy <= 0.0 { v.sq_v += 4.0; }
    v.sq_v += ((target - v.sq) * 340.0 - v.sq_v * 13.0) * dt;
    v.sq = (v.sq + v.sq_v * dt).clamp(0.5, 1.6);
}

fn update_blast(v: &mut View, b: &Brain, now: f64, dt: f32, ox: f32, oy: f32) {
    let Some(blast) = v.blast.as_mut() else { return };
    let u = b.unit as f32;
    if now >= blast.end || b.mode != Mode::Act || b.sub != "explode" {
        v.blast = None;
        v.sq_v -= 6.0;
        return;
    }
    let regroup = blast.end - 0.7;
    let left = b.x as f32 - ox - sprites::W as f32 * u / 2.0;
    let top = b.y as f32 - oy - sprites::H as f32 * u;
    for bit in &mut blast.bits {
        if now < regroup {
            bit.vy += 300.0 * u * dt;
            bit.x += bit.vx * dt;
            bit.y += bit.vy * dt;
            if bit.y > blast.floor - u && bit.vy > 0.0 {
                bit.y = blast.floor - u;
                bit.vy *= -0.45;
                bit.vx *= 0.7;
            }
            bit.fx = bit.x;
            bit.fy = bit.y;
        } else {
            let t = (((now - regroup) / 0.6).min(1.0)) as f32;
            let t = 1.0 - (1.0 - t).powi(3);
            bit.x = bit.fx + (left + bit.col * u - bit.fx) * t;
            bit.y = bit.fy + (top + bit.row * u - bit.fy) * t;
        }
    }
}

fn update_bubble(v: &mut View, b: &Brain, now: f64, gpu: Option<&Gpu>) {
    if b.bubble_id != v.bubble_id {
        v.bubble_id = b.bubble_id;
        v.bubble = None;
        if !b.bubble_text.is_empty() {
            let (mut w, mut h) = (80.0, 20.0);
            if let Some(g) = gpu {
                if let Ok(l) = g.layout(&b.bubble_text, &g.bubble_font, 240.0 * ui_of(g)) { (w, h) = g.layout_size(&l); }
            }
            v.bubble = Some(Bubble { full: b.bubble_text.chars().collect(), start: now, w, h });
        }
    }
    if v.bubble.is_some() && now >= b.bubble_until { v.bubble = None; }
}

fn ui_of(g: &Gpu) -> f32 { unsafe { g.bubble_font.GetFontSize() / 13.5 } }

fn draw_grid(gpu: &Gpu, grid: &[&str], color: u32, cx: f32, cy: f32, cell: f32) {
    let (w, h) = (grid[0].len() as f32 * cell, grid.len() as f32 * cell);
    let c = hex(color);
    for (r, row) in grid.iter().enumerate() {
        let bytes = row.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'.' { i += 1; continue; }
            let start = i;
            while i < bytes.len() && bytes[i] != b'.' { i += 1; }
            gpu.rect(cx - w / 2.0 + start as f32 * cell, cy - h / 2.0 + r as f32 * cell, (i - start) as f32 * cell, cell, c);
        }
    }
}

fn draw_particle(gpu: &Gpu, p: &Particle, ui: f32) {
    let t = p.age / p.life;
    let scale = if p.shrink { 1.0 - t } else { 1.0 + p.grow * t };
    let mut hw = p.w / 2.0 * scale;
    if p.flutter != 0.0 { hw *= (p.age * p.flutter + p.phase).cos().abs() * 0.8 + 0.2; }
    let hh = p.h / 2.0 * scale;
    match &p.look {
        Look::Box(c) => gpu.rect(p.x - hw, p.y - hh, hw * 2.0, hh * 2.0, *c),
        Look::Oval(c, outline) => {
            gpu.oval(p.x, p.y, hw, hh, *c);
            if let Some(o) = outline { gpu.ring(p.x, p.y, hw, hh, 1.0, *o); }
        }
        Look::Heart => draw_grid(gpu, &sprites::HEART, 0xFF4D6D, p.x, p.y, (2.0 * ui).round().max(1.0)),
        Look::Star => draw_grid(gpu, &sprites::STAR, 0xFFD23F, p.x, p.y, (2.0 * ui).round().max(1.0)),
        Look::Text(text, font, c) => {
            let f = match font { 1 => &gpu.glyph_font, 2 => &gpu.zzz_font, _ => &gpu.boom_font };
            gpu.text(text, f, p.x, p.y, *c);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_bubble(gpu: &Gpu, v: &View, b: &Brain, ox: f32, oy: f32, vw: f32, ui: f32, now: f64, alpha: f64) -> windows::core::Result<()> {
    let bubble = v.bubble.as_ref().unwrap();
    let u = b.unit as f32;
    let cx = lerp(v.last.0, v.cur.0, alpha) as f32 - ox;
    let bottom = lerp(v.last.1, v.cur.1, alpha) as f32 - oy - sprites::H as f32 * u - 2.0 * u;
    let (pad, r, tail) = (9.0 * ui, 11.0 * ui, 8.0 * ui);
    let (w, h) = (bubble.w + 2.0 * pad, bubble.h + 2.0 * pad);
    let x1 = (cx - w / 2.0).max(4.0).min(vw - w - 4.0);
    let y2 = bottom - tail;
    let y1 = y2 - h;
    let t = (((now - bubble.start) / 0.22).min(1.0)) as f32;
    let k = 1.70158f32;
    let scale = if t < 1.0 { (1.0 + (k + 1.0) * (t - 1.0).powi(3) + k * (t - 1.0).powi(2)).max(0.05) } else { 1.0 };
    let at = |px: f32, py: f32| (cx + (px - cx) * scale, bottom + (py - bottom) * scale);
    let (sx, sy) = at(x1, y1);
    let (sw, sh, sr) = (w * scale, h * scale, r * scale);
    let off = 3.0 * ui;
    let tail_x = cx.max(x1 + r + 6.0 * ui).min(x1 + w - r - 6.0 * ui);
    let (a, c) = (at(tail_x - 6.0 * ui, y2 - 1.0), at(tail_x + 6.0 * ui, y2 - 1.0));
    let shape = gpu.path(&[pt(a.0, a.1), pt(cx, bottom), pt(c.0, c.1)], true)?;
    let shadow = gpu.path(&[pt(a.0 + off, a.1 + off), pt(cx + off, bottom + off), pt(c.0 + off, c.1 + off)], true)?;
    let shade = [0.0, 0.0, 0.0, 0.3];
    gpu.round_rect(sx + off, sy + off, sw, sh, sr, shade, None);
    gpu.fill(&shadow, shade);
    gpu.round_rect(sx, sy, sw, sh, sr, [1.0; 4], Some((INK, (1.6 * ui).max(1.5))));
    gpu.stroke(&shape, (1.6 * ui).max(1.5), INK);
    gpu.fill(&shape, [1.0; 4]);
    let n = if t < 0.7 { 0 } else { (((now - bubble.start - 0.154) * 45.0) as usize + 1).min(bubble.full.len()) };
    if n > 0 {
        let text: String = bubble.full[..n].iter().collect();
        let (lx, ly) = at(x1 + pad, y1 + pad);
        let l = gpu.layout(&text, &gpu.bubble_font, bubble.w + 1.0)?;
        gpu.draw_layout(&l, lx, ly, INK);
    }
    Ok(())
}

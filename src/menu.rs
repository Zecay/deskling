//! The radial quick menu. It lives in the overlay and never blocks the main loop, so buddies keep moving
//! while it's open. The model has no Windows calls, so its rules are unit-tested.
use std::f32::consts::{FRAC_PI_2, TAU};

use crate::gpu::{pt, Color, Gpu};
use crate::sprites;

pub const HUB: f32 = 46.0;
const INNER_1: f32 = 54.0;
const INNER_2: f32 = 120.0;
const OUTER_1: f32 = 126.0;
const OUTER_2: f32 = 196.0;
/// Half the size of the input window around the wheel, before UI scaling.
pub const REACH: f32 = 206.0;
const ITEM_ARC: f32 = 38.0 * TAU / 360.0;
const DWELL: f32 = 0.11;              // how long the pointer rests on a section before its ring opens

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Act(&'static str), Doodle, Heart, Star, Ledge, ClearInk, Fire, Rain, Bomb, Fireworks, Snow,
    Add, AddTen, Remove, RemoveAll, Bigger, Smaller, Summon, Studio(u8), Select, HideAll, Quit,
}

impl Action {
    /// Toggles and size steps are often repeated, so the wheel stays open for them.
    pub fn keeps_open(self) -> bool {
        matches!(self, Action::Bigger | Action::Smaller | Action::Add)
    }

    /// With a group selected, these happen once per buddy. Everything else is a single global action.
    pub fn per_buddy(self) -> bool {
        matches!(self, Action::Act(_) | Action::Doodle | Action::Heart | Action::Star | Action::Ledge | Action::Fire | Action::Rain
            | Action::Bomb | Action::Remove | Action::Bigger | Action::Smaller)
    }
}

pub struct Item { pub icon: &'static str, pub label: &'static str, pub hint: &'static str, pub action: Action }
pub struct Section { pub icon: &'static str, pub label: &'static str, pub color: u32, pub items: &'static [Item] }

const fn it(icon: &'static str, label: &'static str, hint: &'static str, action: Action) -> Item { Item { icon, label, hint, action } }

pub const SECTIONS: [Section; 5] = [
    Section { icon: "♪", label: "Play", color: 0xD77757, items: &[
        it("♪", "Dance", "Dance", Action::Act("dance")),
        it("↻", "Backflip", "Do a backflip", Action::Act("backflip")),
        it("★", "Party", "Dance with confetti", Action::Act("celebrate")),
        it("➤", "Chase", "Chase my cursor", Action::Act("chase")),
        it("♥", "Love", "Show some love", Action::Act("love")),
        it("☾", "Nap", "Take a nap", Action::Act("sleep")),
    ]},
    Section { icon: "✎", label: "Create", color: 0x4D9DFF, items: &[
        it("✎", "Doodle", "Doodle something", Action::Doodle),
        it("♥", "Heart", "Draw a heart", Action::Heart),
        it("★", "Star", "Draw a star", Action::Star),
        it("▬", "Ledge", "Draw a ledge to stand on", Action::Ledge),
        it("✕", "Clear", "Clear all drawings", Action::ClearInk),
        it("⬚", "Select", "Drag a box around buddies", Action::Select),
    ]},
    Section { icon: "✸", label: "Chaos", color: 0xFF4D6D, items: &[
        it("✸", "Explode", "Burst into pixels", Action::Act("explode")),
        it("♨", "Fire", "Start a fire", Action::Fire),
        it("☂", "Rain", "Make it rain", Action::Rain),
        it("●", "Bomb", "Drop a bomb", Action::Bomb),
        it("✦", "Fireworks", "Fireworks", Action::Fireworks),
        it("❄", "Snow", "Let it snow", Action::Snow),
    ]},
    Section { icon: "☻", label: "Crew", color: 0x4DD599, items: &[
        it("+", "Add", "Add a buddy", Action::Add),
        it("⁺¹⁰", "Add 10", "Add 10 buddies", Action::AddTen),
        it("−", "Remove", "Remove this buddy", Action::Remove),
        it("✖", "Remove all", "Remove every buddy (the tray icon brings one back)", Action::RemoveAll),
        it("▲", "Bigger", "Make this buddy bigger", Action::Bigger),
        it("▼", "Smaller", "Make this buddy smaller", Action::Smaller),
        it("◎", "Summon", "Call everyone to my cursor", Action::Summon),
    ]},
    Section { icon: "⚙", label: "Studio", color: 0xB36BFF, items: &[
        it("✦", "Look", "Open Studio: colors and accessories", Action::Studio(0)),
        it("☺", "Personality", "Open Studio: how they behave", Action::Studio(1)),
        it("⚙", "World", "Open Studio: physics and behaviors", Action::Studio(2)),
        it("◉", "Menu", "Open Studio: change this menu", Action::Studio(3)),
        it("◌", "Hide all", "Hide everyone (tray icon brings them back)", Action::HideAll),
        it("✖", "Quit", "Quit Deskling", Action::Quit),
    ]},
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flags { pub can_add: bool, pub can_remove: bool }

impl Flags {
    fn enabled(self, a: Action) -> bool {
        match a {
            Action::Add | Action::AddTen => self.can_add,
            Action::Remove => self.can_remove,
            _ => true,
        }
    }
}

/// (name, panel colour, text colour, quiet text colour)
pub const THEMES: [(&str, [f32; 3], [f32; 3], [f32; 3]); 4] = [
    ("Midnight", [0.075, 0.075, 0.1], [1.0, 1.0, 1.0], [0.72, 0.72, 0.78]),
    ("Ocean", [0.04, 0.12, 0.18], [0.93, 0.98, 1.0], [0.6, 0.76, 0.84]),
    ("Candy", [0.2, 0.07, 0.17], [1.0, 0.95, 0.98], [0.88, 0.67, 0.8]),
    ("Paper", [0.96, 0.94, 0.9], [0.13, 0.12, 0.11], [0.42, 0.4, 0.37]),
];

/// How the right-click menu looks. Set in Deskling Studio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuLook { pub list: bool, pub theme: u8, pub rainbow: bool, pub accent: u32, pub size: f64, pub opacity: f64, pub speed: f64 }

impl Default for MenuLook {
    fn default() -> Self { MenuLook { list: false, theme: 0, rainbow: true, accent: 0xD77757, size: 1.0, opacity: 0.94, speed: 1.0 } }
}

const LIST_W: f32 = 376.0;
const LIST_H: f32 = 332.0;
const LIST_HEAD: f32 = 44.0;
const LIST_ROW: f32 = 40.0;
const LIST_LEFT: f32 = 166.0;

/// Half the width and height of the input area, in screen pixels.
pub fn half_size(look: &MenuLook, ui: f32) -> (f32, f32) {
    if look.list { (LIST_W / 2.0 * ui, LIST_H / 2.0 * ui) } else { (REACH * ui, REACH * ui) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot { Hub, Section(usize), Item(usize), Nothing, Outside }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome { Nothing, Run(Action), Close }

pub struct Wheel {
    pub buddy: u32,
    pub x: f32,
    pub y: f32,
    pub ui: f32,
    pub flags: Flags,
    pub look: MenuLook,
    pub age: f32,
    pub open: Option<usize>,
    pub hover: Spot,
    pub had_focus: bool,
    pub armed: bool,
    pub pins: Vec<&'static Item>,
    pub group: Vec<u32>,        // sorted ids, empty for a single buddy
    last_hover: Spot,
    keyboard: bool,
    dwell: f32,
    ring_age: f32,
    glow_section: [f32; 5],
    glow_item: [f32; 7],
}

fn section_angle(i: usize) -> f32 { -FRAC_PI_2 + i as f32 * TAU / SECTIONS.len() as f32 }

fn arc_angle(center: f32, n: usize, j: usize) -> f32 { center + (j as f32 - (n as f32 - 1.0) / 2.0) * ITEM_ARC }

#[cfg(test)]
fn item_angle(section: usize, j: usize) -> f32 { arc_angle(section_angle(section), SECTIONS[section].items.len(), j) }

pub const MAX_PINS: usize = 6;

fn find_item(label: &str) -> Option<&'static Item> { SECTIONS.iter().flat_map(|s| s.items.iter()).find(|it| it.label == label) }

/// Where an action will land, shared by the live preview and the action itself so they always agree.
pub fn aim(a: Action, x: f64, y: f64, facing: f64, ui: f64) -> Option<[(f64, f64); 2]> {
    let tip = x + facing * 60.0 * ui;
    match a {
        Action::Heart | Action::Star => Some([(x - 60.0 * ui, y - 300.0 * ui), (x + 60.0 * ui, y - 190.0 * ui)]),
        Action::Ledge => Some([(x - 100.0 * ui, y - 110.0 * ui), (x + 100.0 * ui, y - 110.0 * ui)]),
        Action::Fire => Some([(tip, y); 2]),
        Action::Rain => Some([(x, y - 260.0 * ui); 2]),
        Action::Bomb => Some([(tip, y - 200.0 * ui); 2]),
        _ => None,
    }
}

/// The drag box for selecting a group. Coordinates are screen pixels.
pub struct Marquee { pub from: Option<(f32, f32)>, pub to: (f32, f32), pub age: f32 }

impl Marquee {
    pub fn rect(&self) -> Option<(f32, f32, f32, f32)> {
        let a = self.from?;
        Some((a.0.min(self.to.0), a.1.min(self.to.1), a.0.max(self.to.0), a.1.max(self.to.1)))
    }

    /// True when the box touches a buddy's body.
    pub fn touches(&self, x: f32, top: f32, bottom: f32, half_w: f32) -> bool {
        self.rect().is_some_and(|r| x + half_w >= r.0 && x - half_w <= r.2 && bottom >= r.1 && top <= r.3)
    }
}

pub fn draw_marquee(gpu: &Gpu, m: &Marquee, ox: f32, oy: f32, ui: f32) -> windows::core::Result<()> {
    if let Some((l, t, r, b)) = m.rect() {
        let accent = hex(0xD77757);
        gpu.round_rect(l - ox, t - oy, r - l, b - t, 4.0 * ui, faded(accent, 0.16), Some((accent, 1.6 * ui)));
    }
    let l = gpu.layout("Drag a box around buddies. Esc cancels.", &gpu.bubble_font, 460.0 * ui)?;
    let (tw, th) = gpu.layout_size(&l);
    let (bw, bh) = (tw + 22.0 * ui, th + 10.0 * ui);
    let (bx, by) = (m.to.0 - ox + 16.0 * ui, m.to.1 - oy + 18.0 * ui);
    gpu.round_rect(bx, by, bw, bh, bh / 2.0, [0.075, 0.075, 0.1, 0.96], Some(([1.0, 1.0, 1.0, 0.13], 1.0)));
    gpu.draw_layout(&l, bx + 11.0 * ui, by + 5.0 * ui, [1.0; 4]);
    Ok(())
}

/// The shortest signed turn from one angle to another.
fn turn(from: f32, to: f32) -> f32 { (to - from + TAU / 2.0).rem_euclid(TAU) - TAU / 2.0 }

impl Wheel {
    pub fn new(buddy: u32, x: f32, y: f32, ui: f32, flags: Flags) -> Self {
        Wheel { buddy, x, y, ui, flags, look: MenuLook::default(), age: 0.0, open: None, hover: Spot::Hub, had_focus: false, armed: false, pins: Vec::new(), group: Vec::new(), last_hover: Spot::Hub, keyboard: false,
                dwell: 0.0, ring_age: 0.0, glow_section: [0.0; 5], glow_item: [0.0; 7] }
    }

    pub fn section_color(&self, i: usize) -> u32 { if self.look.rainbow { SECTIONS[i].color } else { self.look.accent } }

    fn hit_list(&self, px: f32, py: f32) -> Spot {
        let (dx, dy) = ((px - self.x) / self.ui + LIST_W / 2.0, (py - self.y) / self.ui + LIST_H / 2.0);
        if !(0.0..=LIST_W).contains(&dx) || !(0.0..=LIST_H).contains(&dy) { return Spot::Outside; }
        if dy < LIST_HEAD { return Spot::Hub; }
        let row = ((dy - LIST_HEAD) / LIST_ROW) as usize;
        if dx < LIST_LEFT {
            if row < SECTIONS.len() { Spot::Section(row) } else { Spot::Nothing }
        } else if row < self.ring_len() {
            Spot::Item(row)
        } else {
            Spot::Nothing
        }
    }

    pub fn hit(&self, px: f32, py: f32) -> Spot {
        if self.look.list { return self.hit_list(px, py); }
        let (dx, dy) = ((px - self.x) / self.ui, (py - self.y) / self.ui);
        let r = dx.hypot(dy);
        let angle = dy.atan2(dx);
        if r < INNER_1 { return Spot::Hub; }
        if r <= INNER_2 + (OUTER_1 - INNER_2) / 2.0 {
            let n = SECTIONS.len();
            let i = (0..n).min_by(|&a, &b| turn(section_angle(a), angle).abs().total_cmp(&turn(section_angle(b), angle).abs())).unwrap_or(0);
            return Spot::Section(i);
        }
        if r <= OUTER_2 {
            for j in 0..self.ring_len() {
                if turn(self.ring_angle(j), angle).abs() <= ITEM_ARC / 2.0 { return Spot::Item(j); }
            }
        }
        Spot::Outside
    }

    /// The outer ring shows the open section, or the pinned favourites when none is open.
    fn ring_len(&self) -> usize { self.open.map_or(self.pins.len(), |s| SECTIONS[s].items.len()) }

    pub fn ring_item(&self, j: usize) -> Option<&'static Item> {
        match self.open {
            Some(s) => SECTIONS[s].items.get(j),
            None => self.pins.get(j).copied(),
        }
    }

    fn ring_angle(&self, j: usize) -> f32 { arc_angle(self.open.map_or(-FRAC_PI_2, section_angle), self.ring_len(), j) }

    pub fn set_pins(&mut self, labels: &[String]) {
        self.pins = labels.iter().filter_map(|l| find_item(l)).take(MAX_PINS).collect();
    }

    pub fn pin_labels(&self) -> Vec<String> { self.pins.iter().map(|it| it.label.to_string()).collect() }

    /// Pin or unpin the hovered item. True if the pins changed.
    pub fn toggle_pin(&mut self) -> bool {
        let Spot::Item(j) = self.hover else { return false };
        let Some(item) = self.ring_item(j) else { return false };
        if let Some(p) = self.pins.iter().position(|x| x.label == item.label) {
            self.pins.remove(p);
            if self.open.is_none() {
                self.hover = if self.pins.is_empty() { Spot::Hub } else { Spot::Item(j.min(self.pins.len() - 1)) };
            }
            true
        } else if self.pins.len() < MAX_PINS {
            self.pins.push(item);
            true
        } else {
            false
        }
    }

    /// Shift+arrow: move a pinned item along the favourites ring. True if the order changed.
    pub fn shift_pin(&mut self, delta: i32) -> bool {
        let (None, Spot::Item(j)) = (self.open, self.hover) else { return false };
        let n = self.pins.len() as i32;
        if n < 2 || j as i32 >= n { return false; }
        let to = (j as i32 + delta).rem_euclid(n) as usize;
        self.pins.swap(j, to);
        self.hover = Spot::Item(to);
        self.keyboard = true;
        true
    }

    /// Reports each change of highlight once: `Some(item)` for a ring item, `Some(None)` for anything else.
    pub fn hover_change(&mut self) -> Option<Option<&'static Item>> {
        if self.hover == self.last_hover { return None; }
        self.last_hover = self.hover;
        Some(match self.hover { Spot::Item(j) => self.ring_item(j), _ => None })
    }

    pub fn pointer(&mut self, px: f32, py: f32) {
        let spot = self.hit(px, py);
        if spot != self.hover { self.dwell = 0.0; }
        self.hover = spot;
        self.keyboard = false;
    }

    fn open_section(&mut self, i: usize) {
        if self.open != Some(i) {
            self.open = Some(i);
            self.ring_age = 0.0;
            self.glow_item = [0.0; 7];
        }
    }

    pub fn step(&mut self, dt: f32) {
        let dt_look = dt * self.look.speed as f32;
        self.age += dt_look;
        self.ring_age += dt_look;
        if let (Spot::Section(i), false) = (self.hover, self.keyboard) {
            if self.open != Some(i) {
                self.dwell += dt;
                if self.dwell >= DWELL { self.open_section(i); }
            }
        }
        let k = (dt * 18.0).min(1.0);
        for (i, g) in self.glow_section.iter_mut().enumerate() {
            let want = if self.hover == Spot::Section(i) { 1.0 } else if self.open == Some(i) { 0.55 } else { 0.0 };
            *g += (want - *g) * k;
        }
        for (j, g) in self.glow_item.iter_mut().enumerate() {
            let want = if self.hover == Spot::Item(j) { 1.0 } else { 0.0 };
            *g += (want - *g) * k;
        }
    }

    /// Arrow keys and the scroll wheel: move through the ring that's showing.
    pub fn cycle(&mut self, delta: i32) {
        self.keyboard = true;
        match self.open {
            Some(s) => {
                let n = SECTIONS[s].items.len() as i32;
                let next = match self.hover {
                    Spot::Item(j) => (j as i32 + delta).rem_euclid(n),
                    _ if delta > 0 => 0,
                    _ => n - 1,
                };
                self.hover = Spot::Item(next as usize);
            }
            None => {
                // Sections first, then any pinned favourites.
                let (sections, n) = (SECTIONS.len() as i32, (SECTIONS.len() + self.pins.len()) as i32);
                let next = match self.hover {
                    Spot::Section(i) => (i as i32 + delta).rem_euclid(n),
                    Spot::Item(j) => (sections + j as i32 + delta).rem_euclid(n),
                    _ if delta > 0 => 0,
                    _ => sections - 1,
                };
                self.hover = if next < sections { Spot::Section(next as usize) } else { Spot::Item((next - sections) as usize) };
            }
        }
    }

    fn run(&self, j: usize) -> Outcome {
        let Some(item) = self.ring_item(j) else { return Outcome::Nothing };
        if self.flags.enabled(item.action) { Outcome::Run(item.action) } else { Outcome::Nothing }
    }

    /// Enter or Space.
    pub fn confirm(&mut self) -> Outcome {
        match self.hover {
            Spot::Section(i) => {
                self.open_section(i);
                self.keyboard = true;
                self.hover = Spot::Item(0);
                Outcome::Nothing
            }
            Spot::Item(j) => self.run(j),
            _ => Outcome::Nothing,
        }
    }

    /// Escape, right-click or the hub: one level up, then closed.
    pub fn back(&mut self) -> Outcome {
        match self.open.take() {
            Some(s) => {
                self.keyboard = true;
                self.hover = Spot::Section(s);
                self.ring_age = 0.0;
                self.glow_item = [0.0; 7];
                Outcome::Nothing
            }
            None => Outcome::Close,
        }
    }

    pub fn click(&mut self, px: f32, py: f32) -> Outcome {
        self.pointer(px, py);
        match self.hover {
            Spot::Hub => self.back(),
            Spot::Section(i) => {
                self.open_section(i);
                Outcome::Nothing
            }
            Spot::Item(j) => self.run(j),
            Spot::Nothing => Outcome::Nothing,
            Spot::Outside => Outcome::Close,
        }
    }

    /// Hold right-click, drag to an item and let go.
    pub fn right_release(&mut self, px: f32, py: f32) -> Outcome {
        if self.age < 0.3 { return Outcome::Nothing; }
        self.pointer(px, py);
        if let Spot::Item(j) = self.hover { self.run(j) } else { Outcome::Nothing }
    }

    fn hint(&self) -> Option<&'static str> {
        match self.hover {
            Spot::Item(j) => self.ring_item(j).map(|it| it.hint),
            _ => None,
        }
    }
}

fn hex(v: u32) -> Color {
    let c = sprites::rgb(v);
    [c[0], c[1], c[2], 1.0]
}

fn mix(a: Color, b: Color, k: f32) -> Color {
    [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k, a[3] + (b[3] - a[3]) * k]
}

fn faded(c: Color, a: f32) -> Color { [c[0], c[1], c[2], c[3] * a] }

/// Overshoots a little, then settles at 1.
fn spring(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0) - 1.0;
    let k = 1.9f32;
    1.0 + (k + 1.0) * t * t * t + k * t * t
}

#[allow(clippy::too_many_arguments)]
fn slice(gpu: &Gpu, cx: f32, cy: f32, r1: f32, r2: f32, mid: f32, half: f32, fill: Color, edge: Color, width: f32) -> windows::core::Result<()> {
    const STEPS: usize = 14;
    let gap_outer = 1.6 / r2.max(1.0);
    let gap_inner = 1.6 / r1.max(1.0);
    let mut pts = Vec::with_capacity((STEPS + 1) * 2);
    for n in 0..=STEPS {
        let a = mid - half + gap_outer + (2.0 * (half - gap_outer)) * n as f32 / STEPS as f32;
        pts.push(pt(cx + a.cos() * r2, cy + a.sin() * r2));
    }
    for n in (0..=STEPS).rev() {
        let a = mid - half + gap_inner + (2.0 * (half - gap_inner)) * n as f32 / STEPS as f32;
        pts.push(pt(cx + a.cos() * r1, cy + a.sin() * r1));
    }
    let geo = gpu.path(&pts, true)?;
    gpu.fill(&geo, fill);
    gpu.stroke(&geo, width, edge);
    Ok(())
}

/// Draws the wheel. `ox`, `oy` is the overlay's origin, and `head` is where the chosen buddy is now.
pub fn draw(gpu: &Gpu, w: &Wheel, ox: f32, oy: f32, buddy: Option<(f32, f32, f32, f32)>) -> windows::core::Result<()> {
    let u = w.ui;
    let (cx, cy) = (w.x - ox, w.y - oy);
    let bloom = spring(w.age / 0.24);
    let fade = (w.age / 0.12).min(1.0);
    let theme = THEMES[(w.look.theme as usize).min(THEMES.len() - 1)];
    let glass = [theme.1[0], theme.1[1], theme.1[2], w.look.opacity as f32];
    let ink = [theme.2[0], theme.2[1], theme.2[2], 1.0];
    let muted = [theme.3[0], theme.3[1], theme.3[2], 1.0];
    let rim = [ink[0], ink[1], ink[2], 0.13];

    if let Some((bx, feet, facing, hy)) = buddy {
        gpu.line((cx, cy), (bx - ox, hy - oy), 2.0 * u, faded(hex(0xD77757), 0.65 * fade));
        gpu.oval(bx - ox, hy - oy, 4.0 * u, 4.0 * u, faded(hex(0xD77757), fade));
        // Live preview: a pulsing ghost where the hovered item will land.
        let item = match w.hover { Spot::Item(j) => w.ring_item(j), _ => None };
        if let Some(pts) = item.and_then(|it| aim(it.action, bx as f64, feet as f64, facing as f64, u as f64)) {
            let glow = faded(hex(0xFFD23F), 0.55 + 0.35 * (w.age * 7.0).sin());
            let (a, b) = ((pts[0].0 as f32 - ox, pts[0].1 as f32 - oy), (pts[1].0 as f32 - ox, pts[1].1 as f32 - oy));
            if a == b {
                gpu.ring(a.0, a.1, 26.0 * u, 26.0 * u, 2.0 * u, glow);
                gpu.oval(a.0, a.1, 3.0 * u, 3.0 * u, glow);
            } else if a.1 == b.1 {
                gpu.line(a, b, 5.0 * u, glow);
            } else {
                gpu.round_rect(a.0, a.1, b.0 - a.0, b.1 - a.1, 8.0 * u, faded(glow, 0.12), Some((glow, 2.0 * u)));
            }
        }
    }
    if w.look.list {
        draw_list(gpu, w, cx, cy, glass, rim, muted, ink)?;
        return draw_hint(gpu, w, cx, cy - (LIST_H / 2.0 + 10.0) * u, glass, rim, ink);
    }
    gpu.oval(cx, cy, (INNER_2 + 8.0) * u * bloom, (INNER_2 + 8.0) * u * bloom, [0.0, 0.0, 0.0, 0.3 * fade]);

    let half = TAU / SECTIONS.len() as f32 / 2.0;
    for (i, sec) in SECTIONS.iter().enumerate() {
        let t = spring((w.age - i as f32 * 0.025) / 0.22);
        let a = ((w.age - i as f32 * 0.025) / 0.1).clamp(0.0, 1.0);
        let g = w.glow_section[i];
        let accent = hex(w.section_color(i));
        let mid = section_angle(i);
        slice(gpu, cx, cy, INNER_1 * u * t, INNER_2 * u * t, mid, half, faded(mix(glass, accent, g * 0.92), a), faded(mix(rim, accent, g), a), 1.2 * u)?;
        let r = (INNER_1 + INNER_2) / 2.0 * u * t;
        let (tx, ty) = (cx + mid.cos() * r, cy + mid.sin() * r);
        gpu.text(sec.icon, &gpu.glyph_font, tx, ty - 8.0 * u, faded(mix(accent, ink, g), a));
        gpu.text(sec.label, &gpu.menu_font, tx, ty + 13.0 * u, faded(mix(muted, ink, g), a));
    }

    {
        let accent = hex(w.open.map_or(0xFFD23F, |s| w.section_color(s)));
        for j in 0..w.ring_len() {
            let Some(item) = w.ring_item(j) else { continue };
            let at = w.ring_age - j as f32 * 0.018;
            let t = spring(at / 0.2);
            let a = (at / 0.09).clamp(0.0, 1.0);
            let g = w.glow_item[j];
            let live = w.flags.enabled(item.action);
            let mid = w.ring_angle(j);
            let (r1, r2) = (INNER_2 + (OUTER_1 - INNER_2) * t, INNER_2 + (OUTER_2 - INNER_2) * t);
            slice(gpu, cx, cy, r1 * u, r2 * u, mid, ITEM_ARC / 2.0, faded(mix(glass, accent, g * 0.95), a), faded(mix(rim, accent, g), a), 1.2 * u)?;
            let r = (r1 + r2) / 2.0 * u;
            let (tx, ty) = (cx + mid.cos() * r, cy + mid.sin() * r);
            let ink = if live { mix(muted, ink, g) } else { [0.4, 0.4, 0.44, 1.0] };
            gpu.text(item.icon, &gpu.glyph_font, tx, ty - 9.0 * u, faded(ink, a));
            gpu.text(item.label, &gpu.menu_font, tx, ty + 13.0 * u, faded(ink, a));
            if w.open.is_some() && w.pins.iter().any(|p| p.label == item.label) { gpu.oval(tx - 15.0 * u, ty - 18.0 * u, 3.5 * u, 3.5 * u, faded(hex(0xFFD23F), a)); }
        }
    }

    let hub_r = HUB * u * bloom;
    let over_hub = w.hover == Spot::Hub;
    gpu.oval(cx, cy, hub_r, hub_r, faded(if over_hub { mix(glass, ink, 0.1) } else { glass }, fade));
    gpu.ring(cx, cy, hub_r, hub_r, 1.6 * u, faded(hex(w.open.map_or(w.look.accent, |s| w.section_color(s))), fade));
    let many = format!("{} buddies", w.group.len());
    let (icon, title) = match w.open {
        Some(s) => ("‹", SECTIONS[s].label),
        None if w.group.len() > 1 => ("✕", many.as_str()),
        None => ("✕", "Close"),
    };
    gpu.text(icon, &gpu.glyph_font, cx, cy - 9.0 * u, faded(ink, fade));
    gpu.text(if w.open.is_some() && over_hub { "Back" } else { title }, &gpu.menu_font, cx, cy + 13.0 * u, faded(muted, fade));

    let top = cy - (OUTER_2 + 12.0) * u;
    draw_hint(gpu, w, cx, top, glass, rim, ink)
}

fn draw_hint(gpu: &Gpu, w: &Wheel, cx: f32, bottom: f32, glass: Color, rim: Color, ink: Color) -> windows::core::Result<()> {
    let u = w.ui;
    if let Some(hint) = w.hint() {
        let pinned = matches!(w.hover, Spot::Item(j) if w.ring_item(j).is_some_and(|it| w.pins.iter().any(|p| p.label == it.label)));
        let text = format!("{hint}   ·   P to {}", if pinned { "unpin" } else { "pin" });
        let l = gpu.layout(&text, &gpu.bubble_font, 460.0 * u)?;
        let (tw, th) = gpu.layout_size(&l);
        let (bw, bh) = (tw + 22.0 * u, th + 10.0 * u);
        let (bx, by) = (cx - bw / 2.0, bottom - bh);
        gpu.round_rect(bx, by, bw, bh, bh / 2.0, [glass[0], glass[1], glass[2], 0.97], Some((rim, 1.0)));
        gpu.draw_layout(&l, bx + 11.0 * u, by + 5.0 * u, ink);
    }
    Ok(())
}

/// The dropdown style: sections on the left, their items flying out on the right.
fn draw_list(gpu: &Gpu, w: &Wheel, cx: f32, cy: f32, glass: Color, rim: Color, muted: Color, ink: Color) -> windows::core::Result<()> {
    let u = w.ui;
    let t = spring(w.age / 0.22);
    let fade = (w.age / 0.1).min(1.0);
    let (hw, hh) = (LIST_W / 2.0 * u, LIST_H / 2.0 * u);
    let (l, top) = (cx - hw, cy - hh - (1.0 - t) * 18.0 * u);
    let main = hex(w.open.map_or(w.look.accent, |s| w.section_color(s)));
    gpu.round_rect(l + 4.0 * u, top + 6.0 * u, hw * 2.0, hh * 2.0, 16.0 * u, [0.0, 0.0, 0.0, 0.28 * fade], None);
    gpu.round_rect(l, top, hw * 2.0, hh * 2.0, 16.0 * u, faded(glass, fade), Some((faded(rim, fade), 1.2 * u)));
    gpu.round_rect(l + 14.0 * u, top + (LIST_HEAD - 3.0) * u, (LIST_W - 28.0) * u * t, 2.0 * u, 1.0 * u, faded(main, fade), None);
    let title = if w.group.len() > 1 { format!("{} buddies", w.group.len()) } else { "Deskling".to_string() };
    gpu.text(&title, &gpu.menu_font, l + LIST_LEFT / 2.0 * u + 8.0 * u, top + LIST_HEAD / 2.0 * u - 1.0 * u, faded(ink, fade));
    let over_head = w.hover == Spot::Hub;
    let back = if w.open.is_some() { "‹ Back" } else { "✕ Close" };
    gpu.text(back, &gpu.menu_font, l + (LIST_W - 46.0) * u, top + LIST_HEAD / 2.0 * u - 1.0 * u, faded(if over_head { ink } else { muted }, fade));
    let row = |n: usize| top + (LIST_HEAD + LIST_ROW * n as f32) * u;
    for (i, sec) in SECTIONS.iter().enumerate() {
        let a = ((w.age - i as f32 * 0.02) / 0.1).clamp(0.0, 1.0);
        let g = w.glow_section[i];
        let accent = hex(w.section_color(i));
        let y = row(i);
        let slide = (1.0 - spring((w.age - i as f32 * 0.02) / 0.2)) * -12.0 * u;
        if g > 0.01 { gpu.round_rect(l + 8.0 * u + slide, y + 3.0 * u, (LIST_LEFT - 12.0) * u, (LIST_ROW - 6.0) * u, 9.0 * u, faded(accent, a * g * 0.92), None); }
        gpu.text(sec.icon, &gpu.glyph_font, l + 30.0 * u + slide, y + LIST_ROW / 2.0 * u, faded(mix(accent, ink, g), a));
        gpu.text(sec.label, &gpu.menu_font, l + 96.0 * u + slide, y + LIST_ROW / 2.0 * u, faded(mix(muted, ink, g), a));
        if w.open == Some(i) { gpu.text("›", &gpu.menu_font, l + (LIST_LEFT - 16.0) * u, y + LIST_ROW / 2.0 * u, faded(ink, a)); }
    }
    let x2 = l + LIST_LEFT * u;
    gpu.rect(x2, top + LIST_HEAD * u, 1.0, (LIST_H - LIST_HEAD - 8.0) * u, faded(rim, fade));
    if w.ring_len() == 0 {
        gpu.text("Hover a section", &gpu.menu_font, x2 + (LIST_W - LIST_LEFT) / 2.0 * u, cy, faded(muted, fade));
    }
    let accent = hex(w.open.map_or(0xFFD23F, |s| w.section_color(s)));
    for j in 0..w.ring_len() {
        let Some(item) = w.ring_item(j) else { continue };
        let at = w.ring_age - j as f32 * 0.016;
        let a = (at / 0.09).clamp(0.0, 1.0);
        let shift = (1.0 - spring(at / 0.18)) * 16.0 * u;
        let g = w.glow_item[j];
        let live = w.flags.enabled(item.action);
        let y = row(j);
        if g > 0.01 { gpu.round_rect(x2 + 6.0 * u + shift, y + 3.0 * u, (LIST_W - LIST_LEFT - 14.0) * u, (LIST_ROW - 6.0) * u, 9.0 * u, faded(accent, a * g * 0.95), None); }
        let c = if live { mix(muted, ink, g) } else { [0.4, 0.4, 0.44, 1.0] };
        gpu.text(item.icon, &gpu.glyph_font, x2 + 28.0 * u + shift, y + LIST_ROW / 2.0 * u, faded(c, a));
        gpu.text(item.label, &gpu.menu_font, x2 + 104.0 * u + shift, y + LIST_ROW / 2.0 * u, faded(c, a));
        if w.pins.iter().any(|p| p.label == item.label) && w.open.is_some() {
            gpu.oval(x2 + (LIST_W - LIST_LEFT - 22.0) * u + shift, y + LIST_ROW / 2.0 * u, 3.5 * u, 3.5 * u, faded(hex(0xFFD23F), a));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel() -> Wheel {
        Wheel::new(7, 500.0, 500.0, 1.0, Flags { can_add: true, can_remove: true })
    }

    fn polar(w: &Wheel, angle: f32, r: f32) -> (f32, f32) { (w.x + angle.cos() * r * w.ui, w.y + angle.sin() * r * w.ui) }

    #[test]
    fn every_section_and_item_can_be_hit() {
        let mut w = wheel();
        for s in 0..SECTIONS.len() {
            let (x, y) = polar(&w, section_angle(s), 85.0);
            assert_eq!(w.hit(x, y), Spot::Section(s));
            w.open = Some(s);
            for j in 0..SECTIONS[s].items.len() {
                let (x, y) = polar(&w, item_angle(s, j), 160.0);
                assert_eq!(w.hit(x, y), Spot::Item(j), "section {s} item {j}");
            }
        }
    }

    #[test]
    fn hit_areas_scale_with_the_display() {
        let mut w = wheel();
        w.ui = 2.0;
        let (x, y) = polar(&w, section_angle(2), 85.0);
        assert_eq!(w.hit(x, y), Spot::Section(2));
        assert_eq!(w.hit(w.x + 10.0, w.y), Spot::Hub);
    }

    #[test]
    fn resting_on_a_section_opens_its_ring() {
        let mut w = wheel();
        let (x, y) = polar(&w, section_angle(1), 85.0);
        w.pointer(x, y);
        w.step(0.05);
        assert_eq!(w.open, None);
        w.step(0.1);
        assert_eq!(w.open, Some(1));
    }

    #[test]
    fn clicking_an_item_runs_it() {
        let mut w = wheel();
        let (x, y) = polar(&w, section_angle(2), 85.0);
        assert_eq!(w.click(x, y), Outcome::Nothing);
        let (x, y) = polar(&w, item_angle(2, 3), 160.0);
        assert_eq!(w.click(x, y), Outcome::Run(Action::Bomb));
    }

    #[test]
    fn disabled_items_do_nothing() {
        let mut w = wheel();
        w.flags.can_remove = false;
        w.open = Some(3);
        let (x, y) = polar(&w, item_angle(3, 2), 160.0);
        assert_eq!(w.click(x, y), Outcome::Nothing);
    }

    #[test]
    fn keyboard_walks_in_and_back_out() {
        let mut w = wheel();
        w.cycle(1);
        assert_eq!(w.hover, Spot::Section(0));
        w.cycle(-1);
        assert_eq!(w.hover, Spot::Section(4));
        assert_eq!(w.confirm(), Outcome::Nothing);
        assert_eq!(w.open, Some(4));
        w.cycle(2);
        assert_eq!(w.confirm(), Outcome::Run(Action::Studio(2)));
        assert_eq!(w.back(), Outcome::Nothing);
        assert_eq!(w.open, None);
        assert_eq!(w.back(), Outcome::Close);
    }

    #[test]
    fn keyboard_choice_is_not_replaced_by_dwell() {
        let mut w = wheel();
        w.cycle(1);
        w.step(1.0);
        assert_eq!(w.open, None);
    }

    #[test]
    fn quick_right_click_release_does_not_pick_an_item() {
        let mut w = wheel();
        w.open = Some(0);
        let (x, y) = polar(&w, item_angle(0, 0), 160.0);
        assert_eq!(w.right_release(x, y), Outcome::Nothing);
        w.step(0.4);
        assert_eq!(w.right_release(x, y), Outcome::Run(Action::Act("dance")));
    }

    #[test]
    fn pins_show_on_the_first_ring_and_run() {
        let mut w = wheel();
        w.set_pins(&["Bomb".to_string(), "not a real item".to_string(), "Dance".to_string()]);
        assert_eq!(w.pin_labels(), ["Bomb", "Dance"]);
        let (x, y) = polar(&w, w.ring_angle(1), 160.0);
        assert_eq!(w.click(x, y), Outcome::Run(Action::Act("dance")));
    }

    #[test]
    fn pinning_toggles_and_stops_at_the_limit() {
        let mut w = wheel();
        w.open = Some(0);
        for j in 0..6 {
            w.hover = Spot::Item(j);
            assert!(w.toggle_pin());
        }
        w.open = Some(2);
        w.hover = Spot::Item(3);
        assert!(!w.toggle_pin(), "a seventh pin is refused");
        w.open = Some(0);
        w.hover = Spot::Item(0);
        assert!(w.toggle_pin());
        assert_eq!(w.pins.len(), 5);
    }

    #[test]
    fn pins_can_be_rearranged() {
        let mut w = wheel();
        w.set_pins(&["Bomb".to_string(), "Dance".to_string(), "Snow".to_string()]);
        w.hover = Spot::Item(0);
        assert!(w.shift_pin(1));
        assert_eq!(w.pin_labels(), ["Dance", "Bomb", "Snow"]);
        assert_eq!(w.hover, Spot::Item(1));
        assert!(w.shift_pin(-1));
        assert!(w.shift_pin(-1), "moving past the start wraps around");
        assert_eq!(w.pin_labels()[2], "Bomb");
    }

    #[test]
    fn keyboard_reaches_pins_after_the_sections() {
        let mut w = wheel();
        w.set_pins(&["Bomb".to_string()]);
        w.hover = Spot::Section(4);
        w.cycle(1);
        assert_eq!(w.hover, Spot::Item(0));
        assert_eq!(w.confirm(), Outcome::Run(Action::Bomb));
        w.cycle(1);
        assert_eq!(w.hover, Spot::Section(0));
    }

    #[test]
    fn hover_changes_are_reported_once() {
        let mut w = wheel();
        w.open = Some(2);
        w.hover = Spot::Item(3);
        assert_eq!(w.hover_change().flatten().map(|it| it.label), Some("Bomb"));
        assert!(w.hover_change().is_none());
    }

    #[test]
    fn labels_are_unique_because_pins_are_saved_by_label() {
        let mut all: Vec<&str> = SECTIONS.iter().flat_map(|s| s.items.iter().map(|it| it.label)).collect();
        let n = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n);
    }

    #[test]
    fn previews_cover_the_placed_actions() {
        for a in [Action::Heart, Action::Star, Action::Ledge, Action::Fire, Action::Rain, Action::Bomb] { assert!(aim(a, 100.0, 500.0, 1.0, 1.0).is_some()); }
        assert_eq!(aim(Action::Snow, 100.0, 500.0, 1.0, 1.0), None);
        assert_eq!(aim(Action::Fire, 100.0, 500.0, -1.0, 2.0).unwrap()[0], (-20.0, 500.0));
    }

    #[test]
    fn marquee_works_in_any_drag_direction() {
        let m = Marquee { from: Some((300.0, 300.0)), to: (100.0, 100.0), age: 0.0 };
        assert!(m.touches(200.0, 180.0, 220.0, 10.0));
        assert!(m.touches(305.0, 280.0, 320.0, 10.0), "touching the edge counts");
        assert!(!m.touches(400.0, 180.0, 220.0, 10.0));
        assert!(!Marquee { from: None, to: (0.0, 0.0), age: 0.0 }.touches(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn group_commands_split_per_buddy_and_global() {
        assert!(Action::Act("dance").per_buddy());
        assert!(Action::Remove.per_buddy());
        assert!(!Action::Fireworks.per_buddy());
        assert!(!Action::Quit.per_buddy());
        assert!(!Action::AddTen.per_buddy());
    }

    #[test]
    fn dropdown_rows_can_all_be_hit() {
        let mut w = wheel();
        w.look.list = true;
        let (left, top) = (w.x - LIST_W / 2.0, w.y - LIST_H / 2.0);
        assert_eq!(w.hit(left + 10.0, top + 10.0), Spot::Hub);
        assert_eq!(w.hit(left - 5.0, top + 100.0), Spot::Outside);
        for s in 0..SECTIONS.len() {
            let y = top + LIST_HEAD + LIST_ROW * (s as f32 + 0.5);
            assert_eq!(w.hit(left + 40.0, y), Spot::Section(s));
            w.open = Some(s);
            for j in 0..SECTIONS[s].items.len() {
                assert_eq!(w.hit(left + LIST_LEFT + 40.0, top + LIST_HEAD + LIST_ROW * (j as f32 + 0.5)), Spot::Item(j));
            }
        }
    }

    #[test]
    fn dropdown_runs_items_and_ignores_empty_rows() {
        let mut w = wheel();
        w.look.list = true;
        w.open = Some(1);
        let (left, top) = (w.x - LIST_W / 2.0, w.y - LIST_H / 2.0);
        assert_eq!(w.click(left + LIST_LEFT + 40.0, top + LIST_HEAD + LIST_ROW * 2.5), Outcome::Run(Action::Star));
        w.open = None;
        assert_eq!(w.click(left + LIST_LEFT + 40.0, top + LIST_HEAD + LIST_ROW * 2.5), Outcome::Nothing, "an empty row keeps the menu open");
    }

    #[test]
    fn studio_items_open_their_page() {
        let studio = &SECTIONS[4];
        assert!(studio.items.iter().any(|i| i.action == Action::Studio(3)));
    }

    #[test]
    fn rings_fit_the_item_counts() {
        for s in &SECTIONS { assert!(s.items.len() <= 7, "{} has too many items", s.label); }
    }
}

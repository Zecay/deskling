//! One buddy's behaviour as a plain state machine: no Windows calls, so it can be tested anywhere.
//!
//! Coordinates are screen pixels on the virtual desktop, y growing downwards. `x` is his centre and
//! `y` the line his feet stand on. Speeds are written in sprite pixels and scaled by `unit`.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use crate::logo;
use crate::shapes::{self, P};

const GRAVITY_PX: f64 = 260.0;
pub const GRAVITY: f64 = GRAVITY_PX;
const WALK_SPEED: f64 = 16.0;
const CHASE_SPEED: f64 = 38.0;
const PEN_SPEED: f64 = 170.0;
const MAX_PEN_TIME: f64 = 18.0;
const LEDGE_KEEP: f64 = 0.6;
const HOP_TIME: f64 = 0.6;
const TYPING_HOLD: f64 = 1.2;
const SLEEP_AFTER: f64 = 180.0;
const MAX_TEXT: usize = 200;
const MAX_QUEUE: usize = 20;
pub const HALF_W: f64 = 8.0;
pub const HEIGHT: f64 = 10.0;
pub const BODY_SPACE: f64 = HALF_W + 1.0;
const BODY_H: f64 = 8.0;
const TAG_GRACE: f64 = 0.8;
const MAX_THROW: f64 = 900.0;
const BOUNCE: f64 = 0.55;
const WALL_BOUNCE: f64 = 0.7;
const BOUNCE_MIN: f64 = 70.0;
const HARD_LANDING: f64 = 320.0;
const SLIDE_MIN: f64 = 20.0;
const SLIDE_FRICTION: f64 = 5.0;
const DIZZY_SPEED: f64 = 450.0;
const FUN_SPEED: f64 = 200.0;
const MAX_SPIN: f64 = 900.0;

pub const ACTIONS: [&str; 18] = ["idle", "sit", "sleep", "wave", "dance", "happy", "jump", "type", "backflip", "spin",
    "explode", "dizzy", "angry", "love", "think", "scared", "celebrate", "chase"];
const PROPS: [&str; 5] = ["fire", "cloud", "bolt", "bomb", "platform"];
const KINDS: [&str; 14] = ["line", "curve", "rect", "ellipse", "arrow", "heart", "star", "text", "fire", "cloud", "bolt",
    "bomb", "platform", "clear"];
const FX: [&str; 5] = ["confetti", "fireworks", "hearts", "sparkle", "snow"];

fn act_seconds(a: &str) -> f64 {
    match a {
        "sit" => 45.0, "type" => 6.0, "wave" => 3.0, "dance" => 4.0, "happy" => 3.0, "spin" => 1.2, "explode" => 2.6,
        "dizzy" => 3.0, "angry" => 3.0, "love" => 3.0, "think" => 4.0, "scared" => 3.0, "celebrate" => 4.0,
        "chase" => 12.0, "squat" => 0.2, _ => 3.0,
    }
}

fn prop_style(kind: &str) -> (&'static str, f64, f64) {
    match kind {
        "fire" => ("#FF7A1A", 70.0, 25.0),
        "cloud" => ("#9AA7B8", 160.0, 12.0),
        "bolt" => ("#FFD23F", 120.0, 1.6),
        "bomb" => ("#333333", 50.0, 0.0),
        _ => ("#7A5230", 0.0, 30.0),
    }
}

pub fn quips(key: &str) -> &'static [&'static str] {
    match key {
        "grab" => &["whoa!", "hey!", "up we go", "eep!", "where are we going?"],
        "hold_long" => &["put me down!!", "the view is nice up here", "ok i live here now"],
        "throw" => &["WHEEEE!", "aaaaaa!", "yeeeet", "to infinity!"],
        "drop" => &["oop", "boop", "careful!"],
        "bonk" => &["oof!", "ow!", "bonk!", "my pixels!"],
        "dizzy" => &["@_@", "the room is spinning", "i see stars", "which way is up"],
        "again" => &["again! again!", "that was fun!", "10/10 would fly again"],
        "poke" => &["hey!", "hehe", "that tickles", "boop!", "yes?"],
        "poked_lots" => &["STOP POKING ME", "okay that's enough", "i will bite (i have no mouth)"],
        "pet" => &["hehe~", "<3", "more pats pls", "happy crab noises"],
        "wake" => &["huh? i'm up!", "*yawn*", "five more minutes..."],
        "typing" => &["go go go!", "you're on fire", "clack clack clack", "ship it!"],
        "caught" => &["gotcha!", "caught it!", "mine now"],
        "burn" => &["HOT HOT HOT", "AAAH FIRE", "who drew that?!", "ouch ouch ouch"],
        "zap" => &["ZAP!", "bzzzt", "i can taste colours", "that was shocking"],
        "boom" => &["KABOOM", "my ears!", "worth it", "who lit that?!"],
        "tag" => &["tag! you're it!", "got you!", "you're it!"],
        "tagged" => &["nooo!", "no fair!", "i'll get you!"],
        "night" => &["it's late... bed?", "night owl mode", "*yawns in pixels*"],
        _ => &["*scuttles*", "nice screen you got", "need a hand? i have two", "what are we building?", "beep boop",
               "is it snack time", "i'm guarding your taskbar"],
    }
}

static INK_IDS: AtomicU64 = AtomicU64::new(1);

/// A small fast random generator, seeded per buddy.
#[derive(Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self { Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1) }
    pub fn f(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, a: f64, b: f64) -> f64 { a + (b - a) * self.f() }
    pub fn pick<T: Copy>(&mut self, v: &[T]) -> T { v[((self.f() * v.len() as f64) as usize).min(v.len() - 1)] }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode { Idle, Walk, Sit, Sleep, Type, Act, Chase, Fall, Hop, Held, Slide, Pen }

impl Mode {
    /// On a surface and moving along it, not flying.
    pub fn standing(self) -> bool {
        matches!(self, Mode::Idle | Mode::Walk | Mode::Sit | Mode::Sleep | Mode::Type | Mode::Act | Mode::Chase)
    }
    fn air(self) -> bool { matches!(self, Mode::Fall | Mode::Hop | Mode::Held | Mode::Slide | Mode::Pen) }
    fn name(self) -> &'static str {
        match self {
            Mode::Idle => "idle", Mode::Walk => "walk", Mode::Sit => "sit", Mode::Sleep => "sleep", Mode::Type => "type",
            Mode::Act => "act", Mode::Chase => "chase", Mode::Fall => "fall", Mode::Hop => "hop", Mode::Held => "held",
            Mode::Slide => "slide", Mode::Pen => "drawing",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind { Floor, Taskbar, Window, Buddy, Drawn }

impl Kind {
    fn name(self) -> &'static str {
        match self { Kind::Floor => "floor", Kind::Taskbar => "taskbar", Kind::Window => "window", Kind::Buddy => "buddy", Kind::Drawn => "drawn" }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Platform { pub key: i64, pub x1: f64, pub x2: f64, pub y: f64, pub kind: Kind }

/// What other buddies need to know about one buddy, kept live as each one moves.
#[derive(Clone, Copy)]
pub struct Snap { pub id: u32, pub x: f64, pub y: f64, pub unit: f64, pub mode: Mode, pub plat: Option<(i64, f64)> }

/// The crew's physics and personality, set in Deskling Studio. 1.0 is the original behaviour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tune {
    pub gravity: f64, pub speed: f64, pub bounce: f64, pub energy: f64, pub chatter: f64, pub curiosity: f64,
    pub sleepy: f64, pub playful: f64, pub chaos: f64, pub climb: bool, pub social: bool, pub naps: bool,
    pub wild: bool,             // Unleashed mode
}

impl Default for Tune {
    fn default() -> Self {
        Tune { gravity: 1.0, speed: 1.0, bounce: 1.0, energy: 1.0, chatter: 1.0, curiosity: 1.0, sleepy: 1.0, playful: 1.0, chaos: 0.0,
               climb: true, social: true, naps: true, wild: false }
    }
}

pub struct World {
    pub tune: Tune,
    pub phrases: Vec<String>,
    pub platforms: Vec<Platform>,
    pub ground_len: usize,
    pub cursor: P,
    pub bounds: (f64, f64, f64, f64),
    pub key_presses: u32,
    pub idle_s: f64,
    pub hour: u32,
    pub snaps: Vec<Snap>,
    pub order: Vec<(f64, usize)>,
    pub xs: Vec<f64>,
    pub pad: f64,
    pub riders: HashMap<i64, u32>,
    pub by_key: HashMap<i64, Platform>,
}

impl World {
    pub fn new(ground: Vec<Platform>) -> Self {
        let by_key = ground.iter().map(|p| (p.key, *p)).collect();
        World { tune: Tune::default(), phrases: Vec::new(), ground_len: ground.len(), platforms: ground, cursor: (0.0, 0.0), bounds: (0.0, 0.0, 1920.0, 1080.0), key_presses: 0,
                idle_s: 0.0, hour: 12, snaps: Vec::new(), order: Vec::new(), xs: Vec::new(), pad: 0.0, riders: HashMap::new(), by_key }
    }
    pub fn platform(&self, key: i64) -> Option<Platform> { self.by_key.get(&key).copied() }
    /// Slots of the buddies whose x was within reach of this range when the frame began.
    pub fn between(&self, x1: f64, x2: f64) -> &[(f64, usize)] {
        let lo = self.xs.partition_point(|&x| x < x1 - self.pad);
        let hi = self.xs.partition_point(|&x| x <= x2 + self.pad);
        &self.order[lo..hi]
    }
    pub fn slot(&self, id: u32) -> Option<usize> { self.snaps.binary_search_by_key(&id, |s| s.id).ok() }
    fn surfaces_at(&self, x: f64) -> Vec<Platform> {
        let mut out = self.platforms[..self.ground_len].to_vec();
        for &(_, slot) in self.between(x, x) {
            if let Some(p) = self.by_key.get(&-(self.snaps[slot].id as i64 + 1)) { out.push(*p); }
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct InkStyle { pub color: String, pub width: f64, pub ttl: f64 }

#[derive(Clone, Debug)]
pub struct PropSpec { pub kind: &'static str, pub x: f64, pub y: f64, pub x2: f64, pub y2: f64, pub size: f64, pub ttl: f64 }

#[derive(Clone, Debug)]
pub enum Event {
    Heart,
    Glyph(&'static str),
    Impact { power: f64, x: f64, y: f64, wall: bool },
    Dust { x: f64, y: f64 },
    Smoke,
    Stars,
    Explode { seconds: f64 },
    Fx { kind: &'static str, x: f64, y: f64 },
    Ink { id: u64, pts: Vec<P>, style: Option<InkStyle>, done: bool },
    Text { x: f64, y: f64, text: String, color: String, size: f64, ttl: f64, duration: f64 },
    Prop(PropSpec),
    Clear,
}

/// Something one buddy does to another; the crew delivers it.
pub enum Note {
    SayLater { to: u32, text: &'static str, delay: f64 },
    TagBack { to: u32, from: u32, rounds: u32 },
}

#[derive(Clone)]
struct Seg { pen: bool, pts: Vec<P>, color: String, width: f64, ttl: f64, speed: Option<f64>, event: Option<Event> }

struct Job { segs: Vec<Seg>, done: Option<Event> }

struct Pen { segs: Vec<Seg>, i: usize, k: usize, pos: P, began: bool, sid: Option<u64>, done: Option<Event>, speed: f64 }

#[derive(Clone, Copy)]
pub struct Hop { x0: f64, y0: f64, pub x1: f64, y1: f64, flip: f64, height: f64, dur: f64, t: f64, pub target: Platform }

pub struct Brain {
    pub tune: Tune,
    pub id: u32,
    pub tint: String,
    pub ui: f64,
    pub unit: f64,
    pub rng: Rng,
    pub now: f64,
    pub started: bool,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub angle: f64,
    pub spin: f64,
    pub facing: i32,
    pub mode: Mode,
    pub sub: &'static str,
    pub timer: f64,
    pub clock: f64,
    pub plat: Option<Platform>,
    pub goal: Option<f64>,
    pub hop: Option<Hop>,
    pub bouncy: bool,
    pub can_crash: bool,        // thrown by the mouse: only these knock others over, so a crowd can't chain-react
    air_drag: f64,
    flight_peak: f64,
    grip: P,
    samples: Vec<(f64, f64, f64)>,
    held_since: f64,
    held_quipped: bool,
    pokes: Vec<f64>,
    next_pet: f64,
    pub chase_until: f64,
    pub chase_who: Option<u32>,
    tag_rounds: u32,
    tag_ready: f64,
    next_leap: f64,
    pub looking: bool,
    pen: Option<Pen>,
    pen_queue: Vec<Job>,
    pub pen_tip: Option<P>,
    pub pen_off: f64,
    pub charred_until: f64,
    burn_cool: f64,
    pub typing_until: f64,
    key_heat: f64,
    next_type_quip: f64,
    pub next_chat: f64,
    pub chatty: bool,
    later: Vec<(f64, String)>,
    pub last_input: f64,
    pub last_cmd: f64,
    pub bubble_text: String,
    pub bubble_until: f64,
    pub bubble_id: u64,
    bubble_mine: bool,
    pending: Option<&'static str>,
    pub events: Vec<Event>,
    pub notes: Vec<Note>,
    pub jam: f64,
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 { v.max(lo).min(hi) }

fn number(v: Option<&Value>, name: &str) -> Result<f64, String> {
    v.and_then(Value::as_f64).ok_or_else(|| format!("{name} must be a number"))
}

fn points(v: Option<&Value>, low: usize) -> Result<Vec<P>, String> {
    let bad = || format!("pts must be a list of {low} to {} [x, y] pairs", shapes::MAX_POINTS);
    let list = v.and_then(Value::as_array).ok_or_else(bad)?;
    if list.len() < low || list.len() > shapes::MAX_POINTS { return Err(bad()); }
    list.iter().map(|p| {
        let pair = p.as_array().filter(|a| a.len() == 2).ok_or("each point must be [x, y]")?;
        Ok((clamp(number(pair.first(), "x")?, -20000.0, 20000.0), clamp(number(pair.get(1), "y")?, -20000.0, 20000.0)))
    }).collect()
}

fn action_name(v: Option<&Value>) -> Option<&'static str> {
    let s = v?.as_str()?;
    ACTIONS.iter().find(|a| **a == s).copied()
}

impl Brain {
    pub fn new(unit: f64, seed: u64, id: u32, tint: &str, ui: f64) -> Self {
        Brain {
            tune: Tune::default(), id, tint: tint.into(), ui, unit, rng: Rng::new(seed), now: 0.0, started: false, x: 0.0, y: 0.0, vx: 0.0, vy: 0.0,
            angle: 0.0, spin: 0.0, facing: 1, mode: Mode::Fall, sub: "", timer: 0.0, clock: 0.0, plat: None, goal: None,
            hop: None, bouncy: false, can_crash: false, air_drag: 3.0, flight_peak: 0.0, grip: (0.0, 0.0), samples: Vec::new(), held_since: 0.0,
            held_quipped: false, pokes: Vec::new(), next_pet: 0.0, chase_until: 0.0, chase_who: None, tag_rounds: 0,
            tag_ready: 0.0, next_leap: 0.0, looking: false, pen: None, pen_queue: Vec::new(), pen_tip: None, pen_off: 10.0,
            charred_until: 0.0, burn_cool: 0.0, typing_until: 0.0, key_heat: 0.0, next_type_quip: 0.0, next_chat: 0.0,
            chatty: true, later: Vec::new(), last_input: 0.0, last_cmd: 0.0, bubble_text: String::new(), bubble_until: 0.0,
            bubble_id: 0, bubble_mine: false, pending: None, events: Vec::new(), notes: Vec::new(), jam: 0.0,
        }
    }

    pub fn charred(&self) -> bool { self.now < self.charred_until }

    pub fn head_y(&self) -> f64 {
        let drop = if matches!(self.mode, Mode::Sit | Mode::Type | Mode::Sleep) { 2.0 } else { 0.0 };
        self.y - (8.0 - drop) * self.unit
    }

    pub fn snap(&self) -> Snap {
        Snap { id: self.id, x: self.x, y: self.y, unit: self.unit, mode: self.mode, plat: self.plat.map(|p| (p.key, p.y)) }
    }

    fn shares_ground(&self, o: &Snap) -> bool {
        if o.id == self.id || !self.mode.standing() || !o.mode.standing() { return false; }
        match (self.plat, o.plat) {
            (Some(a), Some((key, y))) => a.key == key || (a.y - y).abs() < 1.0,
            _ => false,
        }
    }

    pub fn keep_out(&self, other_unit: f64) -> f64 { BODY_SPACE * (self.unit + other_unit) }

    pub fn collides(&self, o: &Brain) -> bool {
        if o.id == self.id || (o.x - self.x).abs() >= self.keep_out(o.unit) { return false; }
        if self.plat.is_some_and(|p| p.key == -(o.id as i64 + 1)) || o.plat.is_some_and(|p| p.key == -(self.id as i64 + 1)) {
            return false;
        }
        self.y > o.y - BODY_H * o.unit && o.y > self.y - BODY_H * self.unit
    }

    fn span(&self, w: &World) -> (f64, f64) {
        let p = self.plat.unwrap();
        let lo = p.x1.max(w.bounds.0 + HALF_W * self.unit);
        let hi = p.x2.min(w.bounds.2 - HALF_W * self.unit);
        if lo <= hi { (lo, hi) } else { (p.x1, p.x2) }
    }

    fn shift(&mut self, dx: f64, w: &World) -> f64 {
        let (lo, hi) = self.span(w);
        let new = if dx > 0.0 { (self.x + dx).min(hi.max(self.x)) } else { (self.x + dx).max(lo.min(self.x)) };
        let moved = (new - self.x).abs();
        self.x = new;
        moved
    }

    /// Ease him sideways out of another buddy. Held and drawing buddies stay put.
    pub fn nudge(&mut self, dx: f64, w: &World) -> f64 {
        match self.mode {
            Mode::Held | Mode::Pen => 0.0,
            Mode::Hop => {
                let Some(hop) = self.hop.as_mut() else { return 0.0 };
                let landing = clamp(hop.x1 + dx, hop.target.x1, hop.target.x2);
                let step = landing - hop.x1;
                if step * dx <= 0.0 { return 0.0; }
                hop.x0 += step;
                hop.x1 += step;
                self.x += step;
                step.abs()
            }
            _ if self.plat.is_some() => self.shift(dx, w),
            _ => {
                let new = clamp(self.x + dx, w.bounds.0 + HALF_W * self.unit, w.bounds.2 - HALF_W * self.unit);
                let moved = (new - self.x).abs();
                self.x = new;
                moved
            }
        }
    }

    pub fn may_land(&self, p: &Platform, w: &World) -> bool {
        if p.kind != Kind::Buddy { return true; }
        let me = -(self.id as i64 + 1);
        if p.key == me { return false; }
        if let Some(&holder) = w.riders.get(&p.key) { if holder != self.id { return false; } }
        match w.riders.get(&me) { Some(&mine) => p.key != -(mine as i64 + 1), None => true }
    }

    fn next_ahead(&self, dir: i32, within: f64, w: &World) -> (Option<Snap>, f64) {
        let d = dir as f64;
        let far = self.x + d * within;
        let (a, b) = if dir > 0 { (self.x, far) } else { (far, self.x) };
        let (mut nearest, mut room) = (None, f64::INFINITY);
        for &(_, slot) in w.between(a, b) {
            let o = w.snaps[slot];
            if self.shares_ground(&o) {
                let ahead = (o.x - self.x) * d;
                if ahead > 0.0 && ahead - self.keep_out(o.unit) < room {
                    nearest = Some(o);
                    room = ahead - self.keep_out(o.unit);
                }
            }
        }
        (nearest, room)
    }

    // ---- per frame ----------------------------------------------------------------

    pub fn update(&mut self, now: f64, dt: f64, w: &World) {
        self.now = now;
        self.tune = w.tune;
        self.clock += dt;
        if !self.started {
            self.started = true;
            self.last_input = now;
            self.last_cmd = now;
            self.next_chat = now + self.rng.range(60.0, 150.0) / self.tune.chatter.max(0.1);
            if self.x == 0.0 && self.y == 0.0 {
                self.x = self.rng.range(w.bounds.0 + HALF_W * self.unit, w.bounds.2 - HALF_W * self.unit);
                self.y = w.bounds.1 + HEIGHT * self.unit;
                self.start_fall(0.0);
            }
        }
        if w.key_presses > 0 { self.typing_until = now + TYPING_HOLD; }
        self.key_heat = self.key_heat * (-dt / 8.0).exp() + w.key_presses as f64;
        if w.key_presses > 0 || w.idle_s < 1.0 { self.last_input = now; }
        if self.later.first().is_some_and(|l| l.0 <= now) {
            let (_, text) = self.later.remove(0);
            self.speak(&text, 2.6);
        }
        if self.charred() && self.rng.f() < dt * 4.0 { self.events.push(Event::Smoke); }
        if self.mode.standing() { self.sync_platform(w); }
        let typing = now < self.typing_until;
        if self.mode == Mode::Sleep && (typing || w.idle_s < 1.0) {
            self.set_idle(Some(0.5));
            self.quip("wake", 0.3);
        }
        if typing && (matches!(self.mode, Mode::Idle | Mode::Walk | Mode::Sit) || self.sub == "squat") {
            self.mode = Mode::Type;
            self.sub = "";
        }
        if self.chatty && self.key_heat > 50.0 && now >= self.next_type_quip {
            self.next_type_quip = now + 120.0;
            self.quip("typing", 0.5);
        }
        if self.chase_until > 0.0 && now >= self.tag_ready && !matches!(self.mode, Mode::Held | Mode::Pen) && self.near_target(10.0, w) {
            self.caught();
        }
        match self.mode {
            Mode::Fall => self.fall(dt, w),
            Mode::Hop => self.hop_step(dt, w),
            Mode::Held => self.held(dt),
            Mode::Slide => self.slide(dt, w),
            Mode::Pen => self.pen_step(dt, w),
            Mode::Walk => self.walk(dt, w),
            Mode::Chase => self.chase(dt, w),
            _ => self.stand(dt, w),
        }
    }

    pub fn drop_off(&mut self) {
        if self.mode.standing() { self.start_fall(0.0); }
    }

    pub fn resize(&mut self, unit: f64) {
        self.unit = clamp(unit, 1.0, 2000.0);
        if self.mode.standing() { self.events.push(Event::Impact { power: 0.3, x: self.x, y: self.y, wall: false }); }
    }

    // ---- the mouse ----------------------------------------------------------------

    pub fn grab(&mut self, cx: f64, cy: f64) {
        if self.mode == Mode::Act && self.sub == "explode" { return; }
        self.cancel_pen();
        self.mode = Mode::Held;
        self.sub = "";
        self.plat = None;
        self.hop = None;
        self.goal = None;
        self.grip = (self.x - cx, self.y - cy);
        self.samples = vec![(self.now, self.x, self.y)];
        self.vx = 0.0;
        self.vy = 0.0;
        self.spin = 0.0;
        self.held_since = self.now;
        self.held_quipped = false;
        self.chase_until = 0.0;
        self.chase_who = None;
        self.quip("grab", 0.7);
    }

    pub fn hold(&mut self, cx: f64, cy: f64, w: &World) {
        if self.mode != Mode::Held { return; }
        let (l, t, r, b) = w.bounds;
        self.x = clamp(cx + self.grip.0, l, r);
        self.y = clamp(cy + self.grip.1, t + HEIGHT * self.unit, b);
        self.samples.push((self.now, self.x, self.y));
        let now = self.now;
        self.samples.retain(|s| now - s.0 <= 0.1);
        let (first, last) = (self.samples[0], self.samples[self.samples.len() - 1]);
        if self.samples.len() >= 2 && last.0 > first.0 {
            self.vx = (last.1 - first.1) / (last.0 - first.0);
            self.vy = (last.2 - first.2) / (last.0 - first.0);
        } else {
            self.vx = 0.0;
            self.vy = 0.0;
        }
    }

    pub fn release(&mut self) {
        if self.mode != Mode::Held { return; }
        let top = MAX_THROW * self.unit;
        let mut speed = self.vx.hypot(self.vy);
        if speed > top {
            self.vx *= top / speed;
            self.vy *= top / speed;
            speed = top;
        }
        self.bouncy = true;
        self.can_crash = true;
        self.air_drag = 0.15;
        self.flight_peak = speed / self.unit;
        self.spin = clamp(self.vx / self.unit * 2.5, -MAX_SPIN, MAX_SPIN);
        self.mode = Mode::Fall;
        self.sub = "";
        self.plat = None;
        if self.flight_peak > 250.0 { self.quip("throw", 0.8); } else if self.flight_peak < 60.0 { self.quip("drop", 0.4); }
    }

    pub fn poke(&mut self) {
        self.last_input = self.now;
        if self.mode == Mode::Sleep {
            self.enter(Mode::Act, "scared", 1.2);
            self.quip("wake", 1.0);
            self.events.push(Event::Glyph("!"));
            return;
        }
        let Some(plat) = self.plat else { return };
        if self.mode.air() { return; }
        let now = self.now;
        self.pokes.retain(|t| now - t < 4.0);
        self.pokes.push(now);
        if self.pokes.len() >= 5 {
            self.pokes.clear();
            self.enter(Mode::Act, "angry", act_seconds("angry"));
            self.quip("poked_lots", 1.0);
            return;
        }
        self.events.push(Event::Glyph("!"));
        self.quip("poke", 0.6);
        self.start_hop(self.x, plat.y, 14.0 * self.unit, 0.35, plat, 0.0);
    }

    pub fn pet(&mut self) {
        if self.mode.air() || self.now < self.next_pet { return; }
        self.next_pet = self.now + 1.0;
        self.last_input = self.now;
        if self.mode == Mode::Sleep {
            self.events.push(Event::Heart);
            return;
        }
        self.events.push(Event::Heart);
        self.events.push(Event::Heart);
        self.enter(Mode::Act, "love", 2.0);
        self.quip("pet", 0.4);
    }

    pub fn trick(&mut self) {
        self.last_input = self.now;
        if !self.mode.air() { let _ = self.act("backflip"); }
    }

    // ---- things that happen to him -------------------------------------------------

    pub fn burn(&mut self, away: f64) {
        if self.mode == Mode::Held || self.now < self.burn_cool { return; }
        self.burn_cool = self.now + 2.5;
        self.charred_until = self.now + 4.0;
        self.cancel_pen();
        let line = self.rng.pick(quips("burn"));
        self.speak(line, 2.6);
        self.launch(away * 90.0 * self.unit, -170.0 * self.unit, away * 500.0, true);
        self.pending = Some("scared");
    }

    pub fn zap(&mut self) {
        if self.mode == Mode::Held || self.now < self.burn_cool { return; }
        self.burn_cool = self.now + 2.0;
        self.charred_until = self.now + 5.0;
        self.cancel_pen();
        let line = self.rng.pick(quips("zap"));
        self.speak(line, 2.6);
        self.events.push(Event::Stars);
        let vx = self.rng.range(-30.0, 30.0) * self.unit;
        self.launch(vx, -140.0 * self.unit, 720.0, true);
        self.pending = Some("dizzy");
    }

    pub fn knock(&mut self, vx: f64, vy: f64, quip: &str) {
        if self.mode == Mode::Held { return; }
        self.cancel_pen();
        self.can_crash = false;
        self.launch(vx, vy, clamp(vx / self.unit * 2.5, -MAX_SPIN, MAX_SPIN), true);
        self.quip(quip, 0.7);
    }

    pub fn say_later(&mut self, text: &str, delay: f64) {
        self.later.push((self.now + delay, text.to_string()));
        self.later.sort_by(|a, b| a.0.total_cmp(&b.0));
    }

    pub fn start_tag(&mut self, other: u32, rounds: u32) {
        self.chase_who = Some(other);
        self.tag_rounds = rounds;
        self.chase_until = self.now + 10.0;
        self.tag_ready = self.now + TAG_GRACE;
        self.goal = None;
        if matches!(self.mode, Mode::Idle | Mode::Walk | Mode::Sit | Mode::Act | Mode::Type) {
            self.mode = Mode::Chase;
            self.sub = "";
        }
    }

    pub fn forget(&mut self, gone: u32) {
        if self.chase_who == Some(gone) {
            self.chase_who = None;
            self.chase_until = 0.0;
        }
    }

    fn target(&self, w: &World) -> P {
        if let Some(s) = self.chase_who.and_then(|id| w.slot(id)).map(|slot| w.snaps[slot]) {
            return (s.x, s.y - HEIGHT / 2.0 * s.unit);
        }
        w.cursor
    }

    fn near_target(&self, mut reach: f64, w: &World) -> bool {
        if let Some(s) = self.chase_who.and_then(|id| w.slot(id)).map(|slot| w.snaps[slot]) {
            reach = reach.max(self.keep_out(s.unit) / self.unit + 1.0);
        }
        let (tx, ty) = self.target(w);
        (tx - self.x).hypot(ty - (self.y - HEIGHT / 2.0 * self.unit)) < reach * self.unit
    }

    fn caught(&mut self) {
        self.chase_until = 0.0;
        let victim = self.chase_who.take();
        if let Some(v) = victim {
            let line = self.rng.pick(quips("tag"));
            self.speak(line, 2.6);
            let text = self.rng.pick(quips("tagged"));
            self.notes.push(Note::SayLater { to: v, text, delay: 0.6 });
            if self.tag_rounds > 1 { self.notes.push(Note::TagBack { to: v, from: self.id, rounds: self.tag_rounds - 1 }); }
            self.tag_rounds = 0;
        } else {
            self.quip("caught", 1.0);
            for _ in 0..3 { self.events.push(Event::Heart); }
        }
        if self.mode.standing() {
            self.enter(Mode::Act, "happy", 2.0);
        } else if self.pending.is_none() {
            self.pending = Some("happy");
        }
    }

    // ---- movement -----------------------------------------------------------------

    fn sync_platform(&mut self, w: &World) {
        let Some(old) = self.plat else { return };
        let found = w.platform(old.key).filter(|p| self.may_land(p, w));
        if let Some(m) = found {
            if ((m.x2 - m.x1) - (old.x2 - old.x1)).abs() < 1.0 { self.x += m.x1 - old.x1; }
            if m.x1 - 1.0 <= self.x && self.x <= m.x2 + 1.0 {
                self.plat = Some(m);
                self.y = m.y;
                return;
            }
        }
        self.start_fall(0.0);
    }

    fn start_fall(&mut self, vx: f64) {
        self.mode = Mode::Fall;
        self.sub = "";
        self.plat = None;
        self.vx = vx;
        self.vy = 0.0;
        self.air_drag = 3.0;
    }

    fn launch(&mut self, vx: f64, vy: f64, spin: f64, bouncy: bool) {
        self.mode = Mode::Fall;
        self.sub = "";
        self.plat = None;
        self.hop = None;
        self.vx = vx;
        self.vy = vy;
        self.spin = spin;
        self.bouncy = bouncy;
        self.air_drag = if bouncy { 0.15 } else { 0.0 };
        self.flight_peak = if bouncy { self.flight_peak.max(vx.hypot(vy) / self.unit) } else { 0.0 };
        if vx != 0.0 { self.facing = if vx > 0.0 { 1 } else { -1 }; }
    }

    /// Launched from a spawn point, popping out of another buddy.
    pub fn pop_out(&mut self, x: f64, y: f64) {
        self.x = x;
        self.y = y;
        let side = if self.rng.f() < 0.5 { -1.0 } else { 1.0 };
        let vx = side * self.rng.range(60.0, 140.0) * self.unit;
        let vy = -self.rng.range(150.0, 230.0) * self.unit;
        let spin = self.rng.range(-400.0, 400.0);
        self.launch(vx, vy, spin, true);
    }

    fn fall(&mut self, dt: f64, w: &World) {
        let u = self.unit;
        let prev = self.y;
        self.vy += GRAVITY * self.tune.gravity * u * dt;
        self.y += self.vy * dt;
        self.x += self.vx * dt;
        self.vx *= (1.0 - self.air_drag * dt).max(0.0);
        self.angle = (self.angle + self.spin * dt).rem_euclid(360.0);
        if self.bouncy { self.flight_peak = self.flight_peak.max(self.vx.hypot(self.vy) / u); }
        self.walls(w);
        let mut best: Option<Platform> = None;
        for p in w.surfaces_at(self.x) {
            if p.x1 <= self.x && self.x <= p.x2 && prev <= p.y && p.y <= self.y && self.may_land(&p, w)
                && best.map_or(true, |b| p.y < b.y) { best = Some(p); }
        }
        let Some(p) = best else { return };
        self.y = p.y;
        let impact = self.vy / u;
        let limit = if self.bouncy { BOUNCE_MIN } else { HARD_LANDING };
        self.events.push(Event::Impact { power: clamp(impact / MAX_THROW, 0.0, 1.0), x: self.x, y: self.y, wall: false });
        if impact > limit {
            self.vy = -self.vy * (BOUNCE * self.tune.bounce).min(if self.tune.wild { 1.0 } else { 0.92 });
            self.vx *= 0.8;
            self.spin = clamp(self.vx / u * 2.5, -MAX_SPIN, MAX_SPIN);
            self.bouncy = true;
            self.air_drag = self.air_drag.max(0.15);
            self.flight_peak = self.flight_peak.max(impact);
            if impact > 500.0 {
                self.events.push(Event::Stars);
                self.quip("bonk", 0.5);
            }
            return;
        }
        self.plat = Some(p);
        self.vy = 0.0;
        self.angle = 0.0;
        self.spin = 0.0;
        if self.bouncy && self.vx.abs() > SLIDE_MIN * u {
            self.mode = Mode::Slide;
            self.sub = "";
            return;
        }
        self.settle();
    }

    fn walls(&mut self, w: &World) {
        let u = self.unit;
        let (l, t, r, _) = w.bounds;
        let (lo, hi) = (l + HALF_W * u, r - HALF_W * u);
        let mut hit: f64 = 0.0;
        if self.x < lo || self.x > hi {
            hit = self.vx.abs() / u;
            self.x = clamp(self.x, lo, hi);
            self.vx = if self.x <= lo { 1.0 } else { -1.0 } * self.vx.abs() * WALL_BOUNCE;
            self.spin = -self.spin * 0.7;
        }
        if self.y - HEIGHT * u < t && self.vy < 0.0 {
            hit = hit.max(-self.vy / u);
            self.y = t + HEIGHT * u;
            self.vy = -self.vy * WALL_BOUNCE;
        }
        if hit > 150.0 {
            self.events.push(Event::Impact { power: clamp(hit / MAX_THROW, 0.0, 1.0), x: self.x, y: self.y, wall: true });
            if hit > 500.0 {
                self.events.push(Event::Stars);
                self.quip("bonk", 0.4);
            }
        }
    }

    fn slide(&mut self, dt: f64, w: &World) {
        let u = self.unit;
        self.x += self.vx * dt;
        self.vx *= (-SLIDE_FRICTION * dt).exp();
        let (l, _, r, _) = w.bounds;
        if !(l + HALF_W * u <= self.x && self.x <= r - HALF_W * u) {
            self.x = clamp(self.x, l + HALF_W * u, r - HALF_W * u);
            self.vx = -self.vx * WALL_BOUNCE;
        }
        if self.rng.f() < dt * 12.0 { self.events.push(Event::Dust { x: self.x, y: self.y }); }
        // Same as standing: follow the surface, and fly on with his speed if it ends.
        let keep = self.vx;
        let on = self.plat.and_then(|old| {
            let m = w.platform(old.key).filter(|p| self.may_land(p, w))?;
            if ((m.x2 - m.x1) - (old.x2 - old.x1)).abs() < 1.0 { self.x += m.x1 - old.x1; }
            (m.x1 - 1.0 <= self.x && self.x <= m.x2 + 1.0).then_some(m)
        });
        match on {
            Some(m) => {
                self.plat = Some(m);
                self.y = m.y;
            }
            None => {
                self.start_fall(keep);
                self.bouncy = true;
                self.air_drag = 0.15;
                return;
            }
        }
        if self.vx.abs() < SLIDE_MIN * u {
            self.vx = 0.0;
            self.settle();
        }
    }

    fn settle(&mut self) {
        let peak = std::mem::take(&mut self.flight_peak);
        let thrown = std::mem::take(&mut self.bouncy);
        self.can_crash = false;
        self.vx = 0.0;
        self.vy = 0.0;
        self.angle = 0.0;
        self.spin = 0.0;
        self.set_idle(Some(0.4));
        self.enter(Mode::Act, "squat", act_seconds("squat"));
        if self.pending.is_some() {
            self.resume();
        } else if thrown && peak > DIZZY_SPEED {
            self.enter(Mode::Act, "dizzy", act_seconds("dizzy"));
            self.quip("dizzy", 0.8);
        } else if thrown && peak > FUN_SPEED && self.rng.f() < 0.5 {
            self.enter(Mode::Act, "happy", 2.0);
            self.quip("again", 1.0);
        } else if self.now < self.chase_until {
            self.mode = Mode::Chase;
            self.sub = "";
        }
    }

    pub fn start_hop(&mut self, x1: f64, y1: f64, height: f64, dur: f64, target: Platform, flip: f64) {
        self.hop = Some(Hop { x0: self.x, y0: self.y, x1, y1, flip, height, dur, t: 0.0, target });
        if x1 != self.x { self.facing = if x1 > self.x { 1 } else { -1 }; }
        self.mode = Mode::Hop;
        self.sub = "";
        self.plat = None;
    }

    fn hop_step(&mut self, dt: f64, w: &World) {
        let Some(mut hop) = self.hop else { self.start_fall(0.0); return };
        hop.t += dt;
        let t = (hop.t / hop.dur).min(1.0);
        self.x = hop.x0 + (hop.x1 - hop.x0) * t;
        self.y = hop.y0 + (hop.y1 - hop.y0) * t - hop.height * 4.0 * t * (1.0 - t);
        self.angle = (hop.flip * t).rem_euclid(360.0);
        if t < 1.0 {
            self.hop = Some(hop);
            return;
        }
        self.hop = None;
        self.angle = 0.0;
        // A buddy he's aiming for may have shuffled sideways during the jump, so land on wherever that head is now.
        if hop.target.kind == Kind::Buddy {
            if let Some(p) = w.platform(hop.target.key) { hop.x1 = clamp(hop.x1, p.x1, p.x2); }
        }
        let target = w.platform(hop.target.key).filter(|p| self.may_land(p, w) && p.x1 - 1.0 <= hop.x1 && hop.x1 <= p.x2 + 1.0);
        let Some(target) = target else {
            self.x = hop.x1;
            self.y = hop.y1;
            self.start_fall(0.0);
            return;
        };
        self.plat = Some(target);
        self.x = hop.x1;
        self.y = target.y;
        self.events.push(Event::Impact { power: 0.15, x: self.x, y: self.y, wall: false });
        self.set_idle(Some(0.3));
        self.resume();
        if self.mode == Mode::Idle && self.now < self.chase_until { self.mode = Mode::Chase; }
    }

    fn held(&mut self, dt: f64) {
        let target = clamp(self.vx / self.unit * 0.12, -60.0, 60.0);
        let angle = if self.angle <= 180.0 { self.angle } else { self.angle - 360.0 };
        self.spin += ((target - angle) * 60.0 - self.spin * 7.0) * dt;
        self.angle = (angle + self.spin * dt).rem_euclid(360.0);
        if self.vx.abs() > 30.0 * self.unit { self.facing = if self.vx > 0.0 { 1 } else { -1 }; }
        if !self.held_quipped && self.now - self.held_since > 5.0 {
            self.held_quipped = true;
            self.quip("hold_long", 0.7);
        }
    }

    fn resume(&mut self) {
        if let Some(action) = self.pending.take() { let _ = self.act(action); }
    }

    fn walk(&mut self, dt: f64, w: &World) {
        let Some(goal) = self.goal else { self.set_idle(None); return };
        let speed = WALK_SPEED * self.tune.speed * self.unit;
        let step = self.facing as f64 * speed * dt;
        let (_, room) = self.next_ahead(self.facing, step.abs(), w);
        if step.abs() >= room {
            self.x += self.facing as f64 * room.max(0.0);
            self.goal = None;
            self.set_idle(None);
            return;
        }
        if (goal - self.x).abs() <= step.abs() {
            self.x = goal;
            self.goal = None;
            self.set_idle(None);
            return;
        }
        self.x += step;
        if let Some(p) = self.plat {
            if !(p.x1 <= self.x && self.x <= p.x2) { self.start_fall(self.facing as f64 * speed * LEDGE_KEEP); }
        }
    }

    fn walk_to(&mut self, goal: f64, w: &World) {
        let dir = if goal > self.x { 1 } else { -1 };
        let (_, room) = self.next_ahead(dir, (goal - self.x).abs(), w);
        let goal = if room < (goal - self.x).abs() { self.x + dir as f64 * room.max(0.0) } else { goal };
        if (goal - self.x).abs() < 1.0 {
            self.goal = None;
            self.set_idle(None);
            return;
        }
        self.goal = Some(goal);
        self.facing = if goal > self.x { 1 } else { -1 };
        self.mode = Mode::Walk;
        self.sub = "";
    }

    fn chase(&mut self, dt: f64, w: &World) {
        if self.now >= self.chase_until {
            self.chase_until = 0.0;
            self.chase_who = None;
            self.set_idle(None);
            return;
        }
        let u = self.unit;
        let (cx, cy) = self.target(w);
        let gap = cx - self.x;
        if gap.abs() > 2.0 * u {
            self.facing = if gap > 0.0 { 1 } else { -1 };
            let f = self.facing as f64;
            let step = gap.abs().min(CHASE_SPEED * self.tune.speed * u * dt);
            let (blocker, room) = self.next_ahead(self.facing, step, w);
            if let Some(b) = blocker {
                if room < step && Some(b.id) != self.chase_who && (cx - b.x) * f > 0.0 && self.now >= self.next_leap {
                    self.next_leap = self.now + 1.2;
                    self.leap_to(b.x + f * (self.keep_out(b.unit) + 2.0 * u), self.y);
                    return;
                }
            }
            self.x += f * step.min(room.max(0.0));
        }
        if gap.abs() < 40.0 * u && self.y - 160.0 * u < cy && cy < self.y - 14.0 * u && self.now >= self.next_leap {
            self.next_leap = self.now + 1.2;
            self.leap_to(cx, cy + HEIGHT / 2.0 * u);
            return;
        }
        if let Some(p) = self.plat {
            if !(p.x1 <= self.x && self.x <= p.x2) { self.start_fall(self.facing as f64 * CHASE_SPEED * self.tune.speed * u * LEDGE_KEEP); }
        }
    }

    fn leap_to(&mut self, tx: f64, ty: f64) {
        let u = self.unit;
        let g = GRAVITY * self.tune.gravity * u;
        let apex = self.y.min(ty) - 30.0 * u;
        let rise = (2.0 * (self.y - apex) / g).sqrt();
        let drop = (2.0 * (ty - apex).max(0.0) / g).sqrt();
        self.events.push(Event::Dust { x: self.x, y: self.y });
        self.launch((tx - self.x) / (rise + drop), -g * rise, 0.0, false);
    }

    fn try_hop(&mut self, w: &World) -> bool {
        let margin = 8.0 * self.unit;
        let reach = 220.0 * self.unit;
        let mut options = Vec::new();
        for p in &w.platforms {
            if self.plat.is_some_and(|m| m.key == p.key) { continue; }
            if p.y >= self.y - 12.0 * self.unit || p.x2 - p.x1 <= 2.0 * margin || !self.may_land(p, w) { continue; }
            let tx = clamp(self.x, p.x1 + margin, p.x2 - margin);
            if (tx - self.x).abs() <= reach { options.push((*p, tx)); }
        }
        if options.is_empty() { return false; }
        let (p, tx) = self.rng.pick(&options);
        self.start_hop(tx, p.y, 28.0 * self.unit, HOP_TIME, p, 0.0);
        true
    }

    // ---- drawing ------------------------------------------------------------------

    fn queue_pen(&mut self, segs: Vec<Seg>, done: Option<Event>) -> Result<(), String> {
        if self.mode == Mode::Held { return Err("he is being held by the mouse right now".into()); }
        let job = Job { segs, done };
        if self.pen.is_some() {
            if self.pen_queue.len() >= MAX_QUEUE { return Err("too many drawings waiting; try again in a moment".into()); }
            self.pen_queue.push(job);
            return Ok(());
        }
        self.start_job(job);
        Ok(())
    }

    fn start_job(&mut self, job: Job) {
        let u = self.unit;
        let tip = self.pen_tip.unwrap_or((self.x + self.facing as f64 * 10.0 * u, self.y - 2.5 * u));
        let first = job.segs[0].pts[0];
        let mut segs = vec![Seg { pen: false, pts: vec![tip, first], color: String::new(), width: 0.0, ttl: 0.0, speed: None, event: None }];
        segs.extend(job.segs);
        let total: f64 = segs.iter().map(|s| shapes::length(&s.pts) / if s.pen { 1.0 } else { 2.2 }).sum();
        self.pen = Some(Pen { segs, i: 0, k: 0, pos: tip, began: false, sid: None, done: job.done, speed: (PEN_SPEED * u).max(total / MAX_PEN_TIME) });
        self.pen_tip = Some(tip);
        self.mode = Mode::Pen;
        self.sub = "";
        self.plat = None;
        self.goal = None;
        self.hop = None;
        self.chase_until = 0.0;
        self.angle = 0.0;
        self.spin = 0.0;
    }

    fn cancel_pen(&mut self) {
        if let Some(sid) = self.pen.as_ref().and_then(|p| p.sid) {
            self.events.push(Event::Ink { id: sid, pts: Vec::new(), style: None, done: true });
        }
        self.pen = None;
        self.pen_queue.clear();
        self.pen_tip = None;
    }

    fn pen_step(&mut self, dt: f64, w: &World) {
        let Some(mut pen) = self.pen.take() else { self.start_fall(0.0); return };
        let mut budget = pen.speed * dt;
        while budget > 1e-9 {
            let i = pen.i;
            if !pen.began {
                pen.began = true;
                if let Some(ev) = pen.segs[i].event.take() { self.events.push(ev); }
                let seg = &pen.segs[i];
                if seg.pen {
                    let sid = INK_IDS.fetch_add(1, Ordering::Relaxed);
                    pen.sid = Some(sid);
                    self.events.push(Event::Ink { id: sid, pts: vec![seg.pts[0]],
                        style: Some(InkStyle { color: seg.color.clone(), width: seg.width, ttl: seg.ttl }), done: false });
                }
            }
            let seg = &pen.segs[i];
            let rate = seg.speed.map_or(if seg.pen { 1.0 } else { 2.2 }, |s| s / pen.speed);
            let mut mv = budget * rate;
            let (mut k, mut pos) = (pen.k, pen.pos);
            let mut fresh = Vec::new();
            while k + 1 < seg.pts.len() {
                let target = seg.pts[k + 1];
                let d = shapes::dist(pos, target);
                if d <= mv {
                    mv -= d;
                    pos = target;
                    k += 1;
                    fresh.push(target);
                } else {
                    let t = mv / d;
                    pos = (pos.0 + (target.0 - pos.0) * t, pos.1 + (target.1 - pos.1) * t);
                    fresh.push(pos);
                    mv = 0.0;
                    break;
                }
            }
            budget = mv / rate;
            pen.pos = pos;
            pen.k = k;
            let (is_pen, unfinished) = (seg.pen, k + 1 < seg.pts.len());
            if is_pen && !fresh.is_empty() {
                self.events.push(Event::Ink { id: pen.sid.unwrap_or(0), pts: fresh, style: None, done: false });
            }
            if unfinished { break; }
            if is_pen {
                self.events.push(Event::Ink { id: pen.sid.unwrap_or(0), pts: Vec::new(), style: None, done: true });
                pen.sid = None;
            }
            pen.i += 1;
            pen.k = 0;
            pen.began = false;
            if pen.i >= pen.segs.len() {
                self.ride(pos, dt, w);
                if let Some(done) = pen.done.take() { self.events.push(done); }
                if !self.pen_queue.is_empty() {
                    let job = self.pen_queue.remove(0);
                    self.start_job(job);
                } else {
                    self.pen_tip = None;
                    self.start_fall(0.0);
                }
                return;
            }
            pen.pos = pen.segs[pen.i].pts[0];
        }
        let at = pen.pos;
        self.pen = Some(pen);
        self.ride(at, dt, w);
    }

    fn ride(&mut self, tip: P, dt: f64, w: &World) {
        let u = self.unit;
        if let Some(old) = self.pen_tip {
            let dx = tip.0 - old.0;
            if dx.abs() > 0.8 * u * dt * 30.0 { self.facing = if dx > 0.0 { 1 } else { -1 }; }
        }
        self.pen_tip = Some(tip);
        self.pen_off += (self.facing as f64 * 10.0 - self.pen_off) * (dt * 8.0).min(1.0);
        let (l, t, r, b) = w.bounds;
        self.x = clamp(tip.0 - self.pen_off * u, l, r);
        self.y = clamp(tip.1 + 2.5 * u, t + HEIGHT * u, b);
    }

    // ---- standing still -----------------------------------------------------------

    fn enter(&mut self, mode: Mode, sub: &'static str, timer: f64) {
        self.mode = mode;
        self.sub = sub;
        self.timer = timer;
        if sub != "spin" { self.angle = 0.0; }
    }

    fn set_idle(&mut self, wait: Option<f64>) {
        self.mode = Mode::Idle;
        self.sub = "";
        self.angle = 0.0;
        self.timer = wait.unwrap_or_else(|| self.rng.range(1.0, 3.0) / self.tune.energy.max(0.1));
    }

    fn decide(&mut self, w: &World) {
        if let Some(g) = self.goal {
            self.walk_to(g, w);
            return;
        }
        if self.rng.f() < 0.12 * self.tune.chaos {
            let wild = self.rng.pick(&["spin", "backflip", "dance", "scared", "explode", "jump"]);
            let _ = self.act(wild);
            return;
        }
        if self.tune.climb && self.rng.f() < 0.3 * self.tune.curiosity && self.try_hop(w) { return; }
        let margin = 8.0 * self.unit;
        let roll = self.rng.f();
        match self.plat {
            Some(p) if roll < 0.5 && p.x2 - p.x1 > 2.0 * margin => {
                let goal = self.rng.range(p.x1 + margin, p.x2 - margin);
                self.walk_to(goal, w);
            }
            _ if roll < 0.5 => { let t = self.rng.range(1.0, 3.5); self.set_idle(Some(t)); }
            _ if roll < 0.62 => { let t = self.rng.range(3.0, 8.0); self.enter(Mode::Sit, "", t); }
            _ => { let t = self.rng.range(1.0, 3.5); self.set_idle(Some(t)); }
        }
    }

    fn stand(&mut self, dt: f64, w: &World) {
        match self.mode {
            Mode::Sleep => return,
            Mode::Type => {
                if self.now >= self.typing_until + 0.6 {
                    self.set_idle(None);
                } else if self.rng.f() < dt * 0.6 {
                    self.events.push(Event::Heart);
                }
                return;
            }
            _ => {}
        }
        self.timer -= dt;
        self.flourish(dt);
        if self.timer <= 0.0 && matches!(self.mode, Mode::Idle | Mode::Sit | Mode::Act) {
            if self.mode == Mode::Act && self.sub == "explode" {
                self.events.push(Event::Impact { power: 0.4, x: self.x, y: self.y, wall: false });
            }
            if self.mode == Mode::Idle { self.decide(w); } else { self.set_idle(None); }
            return;
        }
        if self.mode == Mode::Idle { self.idle_extras(w); }
        let after = SLEEP_AFTER / self.tune.sleepy.max(0.1);
        let quiet = self.tune.naps && self.now - self.last_input > after && self.now - self.last_cmd > after;
        if matches!(self.mode, Mode::Idle | Mode::Sit) && quiet {
            self.mode = Mode::Sleep;
            self.sub = "";
        }
    }

    fn flourish(&mut self, dt: f64) {
        if self.mode != Mode::Act { return; }
        let roll = self.rng.f();
        match self.sub {
            "happy" if roll < dt * 2.0 => self.events.push(Event::Heart),
            "love" if roll < dt * 4.0 => self.events.push(Event::Heart),
            "dance" | "celebrate" if roll < dt * 2.5 => {
                let g = self.rng.pick(&["♪", "♫"]);
                self.events.push(Event::Glyph(g));
            }
            "think" if roll < dt * 1.2 => self.events.push(Event::Glyph("?")),
            "spin" => self.angle = (self.angle + 720.0 * dt) % 360.0,
            _ => {}
        }
    }

    fn idle_extras(&mut self, w: &World) {
        let u = self.unit;
        let (cx, cy) = w.cursor;
        let near = (cx - self.x).abs() < 120.0 * u && self.y - 200.0 * u < cy && cy < self.y + 20.0 * u;
        self.looking = near && cy < self.y - HEIGHT * u;
        if near && (cx - self.x).abs() > 4.0 * u { self.facing = if cx > self.x { 1 } else { -1 }; }
        if self.chatty && self.now >= self.next_chat {
            self.next_chat = self.now + self.rng.range(120.0, 300.0) / self.tune.chatter.max(0.1);
            if !w.phrases.is_empty() && self.rng.f() < 0.6 {
                let n = ((self.rng.f() * w.phrases.len() as f64) as usize).min(w.phrases.len() - 1);
                self.speak(&w.phrases[n], 3.2);
                return;
            }
            let late = w.hour < 5;
            let key = if late && self.rng.f() < 0.5 { "night" } else { "idle" };
            self.quip(key, 0.6);
        }
    }

    pub fn quip(&mut self, key: &str, chance: f64) {
        if self.rng.f() < chance {
            let line = self.rng.pick(quips(key));
            self.speak(line, 2.6);
        }
    }

    /// His own remark. Never covers up something Claude is saying.
    pub fn speak(&mut self, text: &str, seconds: f64) {
        if !self.bubble_mine && self.now < self.bubble_until { return; }
        self.bubble_text = text.to_string();
        self.bubble_until = self.now + seconds;
        self.bubble_id += 1;
        self.bubble_mine = true;
    }

    // ---- commands -----------------------------------------------------------------

    pub fn command(&mut self, cmd: &str, args: &Value, w: &World) -> Result<String, String> {
        if !args.is_object() { return Err("args must be an object".into()); }
        match cmd {
            "status" => Ok(self.status().to_string()),
            "say" => self.say(args),
            "walk" => self.walk_cmd(args, w),
            "act" => {
                let a = action_name(args.get("action")).ok_or_else(|| format!("action must be one of: {}", ACTIONS.join(", ")))?;
                self.act(a)
            }
            "draw" => self.draw(args),
            "turtle" => self.turtle(args),
            "leap" => self.leap_cmd(args, w),
            "fx" => self.fx(args),
            "chatty" => {
                self.chatty = args.get("on").and_then(Value::as_bool).unwrap_or(!self.chatty);
                Ok(if self.chatty { "chatty" } else { "quiet" }.into())
            }
            _ => Err(format!("unknown command {cmd:?}")),
        }
    }

    fn touch(&mut self) {
        self.last_cmd = self.now;
        if self.mode == Mode::Sleep { self.set_idle(Some(0.5)); }
    }

    fn say(&mut self, args: &Value) -> Result<String, String> {
        let text: String = args.get("text").and_then(Value::as_str).unwrap_or("").trim().chars().take(MAX_TEXT).collect();
        if text.is_empty() { return Err("say needs some text".into()); }
        let seconds = match args.get("seconds") { Some(v) => clamp(number(Some(v), "seconds")?, 1.0, 60.0), None => 6.0 };
        let action = match args.get("act") {
            Some(v) => Some(action_name(Some(v)).ok_or_else(|| format!("act must be one of: {}", ACTIONS.join(", ")))?),
            None => None,
        };
        self.touch();
        self.bubble_text = text;
        self.bubble_until = self.now + seconds;
        self.bubble_id += 1;
        self.bubble_mine = false;
        if let Some(a) = action { self.act(a)?; }
        Ok("said".into())
    }

    fn walk_cmd(&mut self, args: &Value, w: &World) -> Result<String, String> {
        let (l, _, r, _) = w.bounds;
        let to = args.get("to");
        let x = match (to.and_then(Value::as_str), to.and_then(Value::as_f64)) {
            (Some("cursor"), _) => w.cursor.0,
            (Some("left"), _) => l + 40.0 * self.unit,
            (Some("right"), _) => r - 40.0 * self.unit,
            (Some("center"), _) => (l + r) / 2.0,
            (None, Some(n)) => n,
            _ => return Err("to must be cursor, left, right, center or an x pixel".into()),
        };
        let x = clamp(x, l, r);
        self.touch();
        self.goal = Some(x);
        if matches!(self.mode, Mode::Idle | Mode::Walk | Mode::Sit | Mode::Act | Mode::Chase) {
            self.chase_until = 0.0;
            self.chase_who = None;
            self.walk_to(x, w);
        }
        Ok(format!("walking to x={}", x.round() as i64))
    }

    fn leap_cmd(&mut self, args: &Value, w: &World) -> Result<String, String> {
        let (tx, ty) = (number(args.get("x"), "x")?, number(args.get("y"), "y")?);
        let (l, t, r, b) = w.bounds;
        let tx = clamp(tx, l + HALF_W * self.unit, r - HALF_W * self.unit);
        let ty = clamp(ty, t + HEIGHT * self.unit, b);
        if self.mode == Mode::Held { return Err("he is being held by the mouse right now".into()); }
        self.touch();
        self.cancel_pen();
        self.goal = None;
        self.leap_to(tx, ty);
        Ok(format!("leaping to {},{}", tx.round() as i64, ty.round() as i64))
    }

    pub fn act(&mut self, action: &'static str) -> Result<String, String> {
        self.touch();
        if self.mode.air() {
            self.pending = Some(action);
            return Ok("queued until he lands".into());
        }
        if action != "chase" {
            self.chase_until = 0.0;
            self.chase_who = None;
        }
        match action {
            "idle" => { self.goal = None; self.set_idle(Some(10.0)); return Ok("idle".into()); }
            "sleep" => { self.goal = None; self.enter(Mode::Sleep, "", 0.0); return Ok("sleeping".into()); }
            "sit" => { self.goal = None; self.enter(Mode::Sit, "", act_seconds("sit")); return Ok("sitting".into()); }
            "type" => {
                self.typing_until = self.typing_until.max(self.now + act_seconds("type"));
                self.enter(Mode::Type, "", 0.0);
                return Ok("typing".into());
            }
            "jump" | "backflip" => {
                let Some(p) = self.plat else { return Ok(action.into()) };
                if action == "jump" {
                    self.start_hop(self.x, p.y, 26.0 * self.unit, 0.5, p, 0.0);
                } else {
                    self.start_hop(self.x, p.y, 40.0 * self.unit, 0.75, p, -360.0 * self.facing as f64);
                }
                return Ok(action.into());
            }
            "chase" => {
                self.goal = None;
                self.chase_who = None;
                self.chase_until = self.now + act_seconds("chase");
                self.mode = Mode::Chase;
                self.sub = "";
                return Ok("chasing the cursor".into());
            }
            "happy" => for _ in 0..3 { self.events.push(Event::Heart); },
            "explode" => self.events.push(Event::Explode { seconds: act_seconds("explode") }),
            "celebrate" => self.events.push(Event::Fx { kind: "confetti", x: self.x, y: self.y }),
            "dizzy" => self.events.push(Event::Stars),
            _ => {}
        }
        self.goal = None;
        self.enter(Mode::Act, action, act_seconds(action));
        Ok("ok".into())
    }

    fn style(&self, args: &Value) -> Result<(String, f64, f64), String> {
        let color: String = args.get("color").and_then(Value::as_str).filter(|c| !c.is_empty()).unwrap_or("red").chars().take(24).collect();
        let width = match args.get("width") { Some(v) => number(Some(v), "width")?, None => 4.0 };
        let ttl = match args.get("ttl") { Some(v) => number(Some(v), "ttl")?, None => 10.0 };
        Ok((color, clamp(width, 1.0, 40.0) * self.ui, clamp(ttl, 0.0, 600.0)))
    }

    fn draw(&mut self, args: &Value) -> Result<String, String> {
        let kind = args.get("kind").and_then(Value::as_str).and_then(|k| KINDS.iter().find(|x| **x == k).copied())
            .ok_or_else(|| format!("kind must be one of: {}", KINDS.join(", ")))?;
        if kind == "clear" {
            self.touch();
            self.events.push(Event::Clear);
            return Ok("cleared".into());
        }
        let (color, width, ttl) = self.style(args)?;
        if let Some(prop) = PROPS.iter().find(|p| **p == kind) { return self.draw_prop(prop, args, width); }
        let segs = if kind == "text" {
            let text: String = args.get("text").and_then(Value::as_str).unwrap_or("").trim().chars().take(MAX_TEXT).collect();
            if text.is_empty() { return Err("text needs some text".into()); }
            let (x, y) = points(args.get("pts"), 1)?[0];
            let size = match args.get("size") { Some(v) => number(Some(v), "size")?, None => 28.0 };
            let size = clamp(size, 8.0, 200.0) * self.ui;
            let n = text.chars().count() as f64;
            let duration = clamp(n * 0.07, 0.3, 6.0);
            let span = (n * size * 0.55).max(10.0);
            let event = Event::Text { x, y, text, color, size, ttl, duration };
            let base = y + size * 1.2;
            vec![Seg { pen: false, pts: vec![(x, base), (x + span, base)], color: String::new(), width: 0.0, ttl: 0.0, speed: Some(span / duration), event: Some(event) }]
        } else {
            let path = shapes::path_for(kind, &points(args.get("pts"), 2)?)?;
            vec![Seg { pen: true, pts: path, color, width, ttl, speed: None, event: None }]
        };
        self.touch();
        self.queue_pen(segs, None)?;
        Ok(format!("drawing {kind}"))
    }

    fn draw_prop(&mut self, kind: &'static str, args: &Value, width: f64) -> Result<String, String> {
        let pts = points(args.get("pts"), 1)?;
        let (ink, default_size, default_ttl) = prop_style(kind);
        let ttl = match args.get("ttl") { Some(v) => clamp(number(Some(v), "ttl")?, 0.0, 600.0), None => default_ttl };
        let (x, y) = pts[0];
        let (path, done) = if kind == "platform" {
            if pts.len() != 2 { return Err("platform needs 2 points: its two ends".into()); }
            let (a, b) = if pts[0] <= pts[1] { (pts[0], pts[1]) } else { (pts[1], pts[0]) };
            (vec![a, b], PropSpec { kind, x: a.0, y: a.1, x2: b.0, y2: b.1, size: 0.0, ttl })
        } else {
            let size = match args.get("size") { Some(v) => number(Some(v), "size")?, None => default_size };
            let size = clamp(size, 16.0, 600.0) * self.ui;
            (shapes::prop_outline(kind, x, y, size), PropSpec { kind, x, y, x2: 0.0, y2: 0.0, size, ttl })
        };
        let first = path[0];
        let end = path[path.len() - 1];
        let mut segs = vec![Seg { pen: true, pts: path, color: ink.into(), width: width.max(3.0 * self.ui),
            ttl: if kind == "platform" { 0.05 } else { 0.25 }, speed: None, event: None }];
        if kind == "platform" {
            segs.push(Seg { pen: false, pts: vec![end, (end.0 - 14.0 * self.unit, first.1.min(end.1) - 6.0 * self.unit)],
                color: String::new(), width: 0.0, ttl: 0.0, speed: None, event: None });
        }
        self.touch();
        self.queue_pen(segs, Some(Event::Prop(done)))?;
        Ok(format!("drawing {kind}"))
    }

    fn turtle(&mut self, args: &Value) -> Result<String, String> {
        let (color, width, ttl) = self.style(args)?;
        let tip = self.pen_tip.unwrap_or((self.x + self.facing as f64 * 10.0 * self.unit, self.y - 2.5 * self.unit));
        let x = match args.get("x") { Some(v) if !v.is_null() => number(Some(v), "x")?, _ => tip.0 };
        let y = match args.get("y") { Some(v) if !v.is_null() => number(Some(v), "y")?, _ => tip.1 - 60.0 * self.ui };
        let prog = args.get("prog").and_then(Value::as_str).ok_or("prog needs some turtle commands")?;
        let strokes = logo::run(prog, x, y, &color, width / self.ui)?;
        let drawn: usize = strokes.iter().filter(|s| s.pen).map(|s| s.pts.len()).sum();
        let ui = self.ui;
        let segs = strokes.into_iter().map(|s| Seg { pen: s.pen, pts: s.pts, color: s.color, width: s.width * ui, ttl, speed: None, event: None }).collect();
        self.touch();
        self.queue_pen(segs, None)?;
        Ok(format!("drawing {drawn} points"))
    }

    fn fx(&mut self, args: &Value) -> Result<String, String> {
        let kind = args.get("kind").and_then(Value::as_str).and_then(|k| FX.iter().find(|x| **x == k).copied())
            .ok_or_else(|| format!("kind must be one of: {}", FX.join(", ")))?;
        let x = match args.get("x") { Some(v) if !v.is_null() => number(Some(v), "x")?, _ => self.x };
        let y = match args.get("y") { Some(v) if !v.is_null() => number(Some(v), "y")?, _ => self.y - HEIGHT * self.unit };
        self.touch();
        self.events.push(Event::Fx { kind, x, y });
        Ok(kind.to_string())
    }

    pub fn status(&self) -> Value {
        let mode = if self.mode == Mode::Act { self.sub } else { self.mode.name() };
        json!({"id": self.id, "mode": mode, "on": self.plat.map_or("air", |p| p.kind.name()),
               "x": self.x.round() as i64, "y": self.y.round() as i64, "tint": self.tint})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 30.0;
    fn floor() -> Platform { Platform { key: 0, x1: 0.0, x2: 1920.0, y: 1040.0, kind: Kind::Floor } }

    struct Clock { w: World, t: f64 }
    impl Clock {
        fn new(p: Vec<Platform>) -> Self { Clock { w: World::new(p), t: 100.0 } }
        fn run(&mut self, b: &mut Brain, seconds: f64, presses: u32) {
            for i in 0..(seconds / DT) as usize {
                self.t += DT;
                self.w.key_presses = if i == 0 { presses } else { 0 };
                b.update(self.t, DT, &self.w);
            }
            self.w.key_presses = 0;
        }
        fn settle(&mut self, b: &mut Brain) {
            for _ in 0..450 {
                if !matches!(b.mode, Mode::Fall | Mode::Slide | Mode::Hop) { return; }
                self.run(b, DT * 1.01, 0);
            }
        }
    }

    fn landed() -> (Brain, Clock) {
        let mut b = Brain::new(4.0, 1, 0, "orange", 1.0);
        let mut c = Clock::new(vec![floor()]);
        c.run(&mut b, 1.0, 0);
        c.settle(&mut b);
        b.events.clear();
        (b, c)
    }

    #[test]
    fn falls_and_lands_on_floor() {
        let (b, _) = landed();
        assert_eq!(b.plat.unwrap().kind, Kind::Floor);
        assert!((b.y - 1040.0).abs() < 1e-6);
    }

    #[test]
    fn lands_on_window_above_floor() {
        let mut b = Brain::new(4.0, 1, 0, "orange", 1.0);
        let mut c = Clock::new(vec![floor(), Platform { key: 42, x1: 0.0, x2: 1920.0, y: 500.0, kind: Kind::Window }]);
        c.run(&mut b, 2.0, 0);
        assert_eq!(b.plat.unwrap().key, 42);
    }

    #[test]
    fn walk_command_reaches_goal() {
        let (mut b, mut c) = landed();
        b.command("walk", &json!({"to": 300}), &c.w).unwrap();
        for _ in 0..1800 {
            if b.goal.is_none() { break; }
            c.run(&mut b, DT * 1.01, 0);
        }
        assert!(b.goal.is_none());
        assert!((b.x - 300.0).abs() <= 1.0);
    }

    #[test]
    fn typing_switches_to_type_then_back() {
        let (mut b, mut c) = landed();
        b.set_idle(Some(10.0)); // a bouncy landing can end in a happy pose, which typing doesn't interrupt
        c.run(&mut b, 0.2, 1);
        assert_eq!(b.mode, Mode::Type);
        c.run(&mut b, 4.0, 0);
        assert_ne!(b.mode, Mode::Type);
    }

    #[test]
    fn sleeps_after_long_quiet() {
        let mut b = Brain::new(4.0, 1, 0, "orange", 1.0);
        let mut c = Clock::new(vec![floor()]);
        c.w.idle_s = 999.0;
        c.run(&mut b, 260.0, 0);
        assert_eq!(b.mode, Mode::Sleep);
        c.run(&mut b, 0.1, 1);
        assert_eq!(b.mode, Mode::Type);
    }

    #[test]
    fn action_mid_air_runs_on_landing() {
        let mut b = Brain::new(4.0, 1, 0, "orange", 1.0);
        let mut c = Clock::new(vec![floor()]);
        c.run(&mut b, 0.2, 0);
        assert_eq!(b.command("act", &json!({"action": "dance"}), &c.w).unwrap(), "queued until he lands");
        c.settle(&mut b);
        assert_eq!((b.mode, b.sub), (Mode::Act, "dance"));
    }

    #[test]
    fn he_rides_the_pencil() {
        let (mut b, mut c) = landed();
        b.command("draw", &json!({"kind": "ellipse", "pts": [[100, 100], [300, 200]], "color": "blue", "ttl": 5}), &c.w).unwrap();
        assert_eq!(b.mode, Mode::Pen);
        let mut done = false;
        for _ in 0..900 {
            c.run(&mut b, DT * 1.01, 0);
            done |= b.events.drain(..).any(|e| matches!(e, Event::Ink { done: true, .. }));
            if b.mode != Mode::Pen { break; }
        }
        assert!(done);
        assert_eq!(b.mode, Mode::Fall);
    }

    #[test]
    fn drawing_a_prop_ends_with_a_prop_event() {
        let (mut b, mut c) = landed();
        assert_eq!(b.command("draw", &json!({"kind": "fire", "pts": [[600, 1040]]}), &c.w).unwrap(), "drawing fire");
        let mut prop = None;
        for _ in 0..600 {
            c.run(&mut b, DT * 1.01, 0);
            for e in b.events.drain(..) { if let Event::Prop(p) = e { prop = Some(p); } }
            if b.mode != Mode::Pen { break; }
        }
        let p = prop.unwrap();
        assert_eq!((p.kind, p.x, p.y), ("fire", 600.0, 1040.0));
    }

    #[test]
    fn bad_commands_are_rejected() {
        let (mut b, c) = landed();
        for (cmd, args) in [
            ("say", json!({"text": "   "})), ("say", json!({"text": "x", "seconds": "soon"})), ("walk", json!({"to": "moon"})),
            ("walk", json!({"to": true})), ("act", json!({"action": "fly"})),
            ("draw", json!({"kind": "circle", "pts": [[0, 0], [1, 1]]})), ("draw", json!({"kind": "line", "pts": [[0, 0]]})),
            ("draw", json!({"kind": "rect", "pts": [[0, 0], [1, 2], [3, 4]]})), ("draw", json!({"kind": "rect", "pts": [[0, 0], [1, "x"]]})),
            ("nope", json!({})), ("say", json!("text")), ("fx", json!({"kind": "nuke"})), ("leap", json!({"x": 1})),
            ("turtle", json!({"prog": "fd 10 jump"})), ("draw", json!({"kind": "platform", "pts": [[1, 1]]})),
        ] {
            assert!(b.command(cmd, &args, &c.w).is_err(), "{cmd} {args}");
        }
    }

    #[test]
    fn thrown_buddy_bounces_and_gets_dizzy() {
        let (mut b, mut c) = landed();
        b.x = 900.0;
        let (mut cx, mut cy) = (b.x, b.y - 20.0);
        b.grab(cx, cy);
        for _ in 0..5 {
            cx += 80.0;
            cy -= 60.0;
            c.t += DT;
            b.now = c.t;
            b.hold(cx, cy, &c.w);
        }
        b.release();
        assert!(b.vx > 0.0 && b.spin > 0.0);
        let mut impacts = 0;
        for _ in 0..600 {
            c.run(&mut b, DT * 1.01, 0);
            impacts += b.events.drain(..).filter(|e| matches!(e, Event::Impact { .. })).count();
            if !matches!(b.mode, Mode::Fall | Mode::Slide) { break; }
        }
        assert!(impacts >= 2);
        assert_eq!((b.mode, b.sub), (Mode::Act, "dizzy"));
        assert_eq!(b.angle, 0.0);
    }

    #[test]
    fn poking_five_times_makes_him_angry() {
        let (mut b, mut c) = landed();
        for _ in 0..5 {
            b.poke();
            c.settle(&mut b);
        }
        assert_eq!((b.mode, b.sub), (Mode::Act, "angry"));
    }

    #[test]
    fn chase_catches_the_cursor() {
        let (mut b, mut c) = landed();
        b.x = 300.0;
        c.w.cursor = (700.0, 1020.0);
        b.command("act", &json!({"action": "chase"}), &c.w).unwrap();
        for _ in 0..240 {
            c.run(&mut b, DT * 1.01, 0);
            if b.chase_until == 0.0 { break; }
        }
        assert_eq!(b.chase_until, 0.0);
        assert!(quips("caught").contains(&b.bubble_text.as_str()));
    }
}

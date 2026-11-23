//! How buddies get along: standing on heads, crashing, keeping apart, piling up and hanging out.
use std::collections::{HashMap, HashSet};

use crate::brain::*;

pub const MAX_BUDDIES: usize = 200;
/// Unleashed mode: far beyond what any PC can run, only here so a typo can't exhaust memory at once.
pub const UNLEASHED_BUDDIES: usize = 1_000_000;
const SEPARATE_RATE: f64 = 10.0;
const SEPARATE_SPEED: f64 = 30.0;
const JAM_CLIMB: f64 = 0.6;       // s squeezed in a crowd before he climbs on someone
const MAX_CLIMBS: usize = 2;     // per frame, so a big crowd pours upward instead of jumping at once
const CHATS: [(&str, &str); 9] = [
    ("hi!", "hey :)"),
    ("nice weather on this screen", "it's always 60fps here"),
    ("did you see that window move?", "spooky"),
    ("i'm the original", "we're all clones, buddy"),
    ("what are we building?", "something cool i hope"),
    ("race you!", "you're on!"),
    ("do you ever feel like a sprite?", "every frame"),
    ("i drew a fire earlier", "that explains the smell"),
    ("the user is typing again", "shhh, look busy"),
];

pub struct Frame<'a> {
    pub ground: Vec<Platform>,
    pub cursor: (f64, f64),
    pub bounds: (f64, f64, f64, f64),
    pub key_presses: u32,
    pub idle_s: f64,
    pub hour: u32,
    pub _marker: std::marker::PhantomData<&'a ()>,
}

pub struct Crew {
    pub brains: Vec<Brain>,
    pub world: World,
    rng: Rng,
    next_social: f64,
    bumped: HashMap<(u32, u32), f64>,
}

fn pair(v: &mut [Brain], i: usize, j: usize) -> (&mut Brain, &mut Brain) {
    if i < j {
        let (a, b) = v.split_at_mut(j);
        (&mut a[i], &mut b[0])
    } else {
        let (a, b) = v.split_at_mut(i);
        (&mut b[0], &mut a[j])
    }
}

impl Crew {
    pub fn new(seed: u64) -> Self {
        Crew { brains: Vec::new(), world: World::new(Vec::new()), rng: Rng::new(seed), next_social: 0.0, bumped: HashMap::new() }
    }

    /// Rebuild the shared world: ground, every standing buddy's head, and the crew sorted by x.
    fn prepare(&mut self, f: Frame) {
        let w = &mut self.world;
        w.ground_len = f.ground.len();
        w.platforms.clear();
        w.platforms.extend_from_slice(&f.ground);
        w.riders.clear();
        w.snaps.clear();
        w.order.clear();
        w.xs.clear();
        let mut biggest: f64 = 1.0;
        for (slot, b) in self.brains.iter().enumerate() {
            biggest = biggest.max(b.unit);
            if let Some(p) = b.plat { if p.kind == Kind::Buddy { w.riders.insert(p.key, b.id); } }
            if b.mode.standing() {
                let half = 6.0 * b.unit;
                w.platforms.push(Platform { key: -(b.id as i64 + 1), x1: b.x - half, x2: b.x + half, y: b.head_y(), kind: Kind::Buddy });
            }
            w.snaps.push(b.snap());
            w.order.push((b.x, slot));
        }
        w.order.sort_by(|a, b| a.0.total_cmp(&b.0));
        w.xs.extend(w.order.iter().map(|o| o.0));
        w.pad = (2.0 * BODY_SPACE + 16.0) * biggest;
        w.by_key.clear();
        for p in &w.platforms { w.by_key.insert(p.key, *p); }
        w.cursor = f.cursor;
        w.bounds = f.bounds;
        w.key_presses = f.key_presses;
        w.idle_s = f.idle_s;
        w.hour = f.hour;
    }

    pub fn step(&mut self, now: f64, dt: f64, f: Frame) {
        self.prepare(f);
        for i in 0..self.brains.len() {
            self.brains[i].update(now, dt, &self.world);
            self.world.snaps[i] = self.brains[i].snap();
            self.deliver(i);
        }
        self.crashes(now);
        if now >= self.next_social {
            self.next_social = now + self.rng.range(8.0, 20.0) / self.world.tune.playful.max(0.1);
            if self.world.tune.social { self.socialise(); }
        }
        self.keep_apart(dt);
        self.climb();
    }

    fn deliver(&mut self, i: usize) {
        let notes = std::mem::take(&mut self.brains[i].notes);
        for note in notes {
            match note {
                Note::SayLater { to, text, delay } => {
                    if let Some(v) = self.brains.iter_mut().find(|b| b.id == to) { v.say_later(text, delay); }
                }
                Note::TagBack { to, from, rounds } => {
                    if let Some(v) = self.brains.iter_mut().find(|b| b.id == to) { v.start_tag(from, rounds); }
                }
            }
        }
    }

    fn crashes(&mut self, now: f64) {
        for i in 0..self.brains.len() {
            let a = &self.brains[i];
            if a.mode != Mode::Fall || !a.can_crash || a.vx.hypot(a.vy) / a.unit < 150.0 { continue; }
            let x = a.x;
            let near: Vec<usize> = self.world.between(x, x).iter().map(|o| o.1).collect();
            for j in near {
                if j == i || j >= self.brains.len() { continue; }
                let (a, b) = pair(&mut self.brains, i, j);
                if b.mode == Mode::Held { continue; }
                let key = (a.id.min(b.id), a.id.max(b.id));
                if now - self.bumped.get(&key).copied().unwrap_or(-9.0) < 0.6 { continue; }
                let reach = 8.0 * a.unit.max(b.unit);
                let (ay, by) = (a.y - HEIGHT / 2.0 * a.unit, b.y - HEIGHT / 2.0 * b.unit);
                if (a.x - b.x).abs() < reach * 1.4 && (ay - by).abs() < reach {
                    self.bumped.insert(key, now);
                    b.knock(a.vx * 0.7, (a.vy * 0.5).min(0.0) - 120.0 * b.unit, "bonk");
                    a.vx *= -0.35;
                    a.events.push(Event::Stars);
                }
            }
        }
        if self.bumped.len() > 2000 { self.bumped.retain(|_, t| now - *t < 1.0); }
    }

    fn socialise(&mut self) {
        let mut free: Vec<usize> = (0..self.brains.len())
            .filter(|&i| matches!(self.brains[i].mode, Mode::Idle | Mode::Sit) && self.brains[i].chatty).collect();
        free.sort_by(|&a, &b| self.brains[a].x.total_cmp(&self.brains[b].x));
        let mut pairs = Vec::new();
        'outer: for (n, &i) in free.iter().enumerate() {
            let a = &self.brains[i];
            for &j in &free[n + 1..] {
                let b = &self.brains[j];
                if b.x - a.x >= 220.0 * a.unit { break; }
                if (a.y - b.y).abs() < 30.0 * a.unit {
                    pairs.push((i, j));
                    if pairs.len() >= 50 { break 'outer; }
                }
            }
        }
        if pairs.is_empty() { return; }
        let (i, j) = self.rng.pick(&pairs);
        let roll = self.rng.f();
        let chat = self.rng.pick(&CHATS);
        let (a, b) = pair(&mut self.brains, i, j);
        a.facing = if b.x > a.x { 1 } else { -1 };
        b.facing = -a.facing;
        if roll < 0.35 {
            a.speak(chat.0, 2.6);
            b.say_later(chat.1, 1.6);
            let _ = a.act("wave");
        } else if roll < 0.55 {
            a.speak("dance off!", 2.6);
            b.say_later("oh it's ON", 0.8);
            let _ = a.act("dance");
            let _ = b.act("dance");
        } else if roll < 0.75 {
            a.speak("high five!", 2.6);
            let _ = a.act("jump");
            let _ = b.act("jump");
            a.events.push(Event::Glyph("✦"));
        } else {
            a.speak("tag! you're it!", 2.6);
            b.say_later("hey!!", 0.5);
            b.start_tag(a.id, 3);
        }
    }

    /// Bodies don't stay inside each other: any that touch slide apart over a few frames.
    fn keep_apart(&mut self, dt: f64) {
        let n = self.brains.len();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| self.brains[a].x.total_cmp(&self.brains[b].x));
        let reach = 2.0 * BODY_SPACE * self.brains.iter().map(|b| b.unit).fold(1.0, f64::max);
        let mut squeezed = vec![false; n];
        for s in 0..n {
            let i = order[s];
            for &j in &order[s + 1..] {
                if self.brains[j].x - self.brains[i].x >= reach { break; }
                if !self.brains[i].collides(&self.brains[j]) { continue; }
                let world = &self.world;
                let (a, b) = pair(&mut self.brains, i, j);
                let gap = a.keep_out(b.unit) - (b.x - a.x).abs();
                let step = gap.min((gap * SEPARATE_RATE + SEPARATE_SPEED * a.unit.min(b.unit)) * dt);
                if gap > 0.2 * a.keep_out(b.unit) {
                    squeezed[i] = true;
                    squeezed[j] = true;
                }
                if step <= 0.0 { continue; }
                let side = if b.x >= a.x { 1.0 } else { -1.0 };
                let mut rest = step - a.nudge(-side * step / 2.0, world) - b.nudge(side * step / 2.0, world);
                if rest > 1e-9 {
                    rest -= a.nudge(-side * rest, world);
                    b.nudge(side * rest, world);
                }
            }
        }
        for (b, s) in self.brains.iter_mut().zip(squeezed) {
            b.jam = if s { b.jam + dt } else { (b.jam - dt * 2.0).max(0.0) };
        }
    }

    /// A crowd too big for the floor piles up: anyone squeezed for a while hops onto the nearest free head.
    fn climb(&mut self) {
        let mut claimed: HashSet<i64> = self.brains.iter().filter_map(|b| b.hop.map(|h| h.target.key)).collect();
        let mut climbs = 0;
        for i in 0..self.brains.len() {
            if climbs >= MAX_CLIMBS { break; }
            let b = &self.brains[i];
            if b.jam < JAM_CLIMB || !matches!(b.mode, Mode::Idle | Mode::Walk | Mode::Sit) { continue; }
            let head = -(b.id as i64 + 1);
            if self.world.riders.contains_key(&head) || claimed.contains(&head) { continue; }  // someone is on him or landing on him
            let mut best: Option<Platform> = None;
            for p in &self.world.platforms[self.world.ground_len..] {
                if claimed.contains(&p.key) || !b.may_land(p, &self.world) || p.y >= b.y { continue; }
                let score = |q: &Platform| ((q.x1 + q.x2) / 2.0 - b.x).abs() + (b.y - q.y) * 2.0;
                if best.map_or(true, |q| score(p) < score(&q)) { best = Some(*p); }
            }
            // Nobody higher yet: step up onto the closest neighbour on his own level.
            if best.is_none() {
                for p in &self.world.platforms[self.world.ground_len..] {
                    if claimed.contains(&p.key) || !b.may_land(p, &self.world) { continue; }
                    let d = |q: &Platform| ((q.x1 + q.x2) / 2.0 - b.x).abs();
                    if best.map_or(true, |q| d(p) < d(&q)) { best = Some(*p); }
                }
            }
            let Some(p) = best else { continue };
            claimed.insert(p.key);
            let unit = b.unit;
            let b = &mut self.brains[i];
            b.jam = 0.0;
            b.goal = None;
            b.start_hop((p.x1 + p.x2) / 2.0, p.y, 22.0 * unit, 0.5, p, 0.0);
            climbs += 1;
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    const DT: f64 = 1.0 / 30.0;
    const FLOOR: Platform = Platform { key: 0, x1: 0.0, x2: 1920.0, y: 1040.0, kind: Kind::Floor };

    pub struct Town { pub crew: Crew, pub t: f64 }
    impl Town {
        pub fn new(xs: &[f64]) -> Self {
            let mut crew = Crew::new(1);
            crew.next_social = 1e9;
            let t = 100.0;
            for (i, &x) in xs.iter().enumerate() {
                let mut b = Brain::new(4.0, i as u64 + 1, i as u32, "orange", 1.0);
                b.x = x;
                b.y = 1040.0;
                b.started = true;
                b.now = t;
                b.last_input = t;
                b.last_cmd = t;
                b.plat = Some(FLOOR);
                b.mode = Mode::Idle;
                b.timer = 50.0;
                b.next_chat = 1e9;
                crew.brains.push(b);
            }
            Town { crew, t }
        }
        pub fn run(&mut self, seconds: f64) {
            for _ in 0..(seconds / DT).round() as usize {
                self.t += DT;
                let f = Frame { ground: vec![FLOOR], cursor: (0.0, 0.0), bounds: (0.0, 0.0, 1920.0, 1080.0), key_presses: 0,
                                idle_s: 0.0, hour: 12, _marker: Default::default() };
                self.crew.step(self.t, DT, f);
                for b in &mut self.crew.brains { b.events.clear(); }
            }
        }
    }

    fn fall_from(b: &mut Brain, x: f64, y: f64) {
        b.x = x;
        b.y = y;
        b.drop_off();
    }

    #[test]
    fn dropped_buddy_lands_on_a_head_and_rides_along() {
        let mut t = Town::new(&[800.0, 300.0]);
        fall_from(&mut t.crew.brains[1], 800.0, 700.0);
        t.run(1.5);
        assert_eq!(t.crew.brains[1].plat.unwrap().kind, Kind::Buddy);
        t.crew.brains[0].goal = None;
        let w = World::new(vec![FLOOR]);
        t.crew.brains[0].command("walk", &serde_json::json!({"to": 900}), &w).unwrap();
        t.run(2.0);
        assert!(t.crew.brains[1].x > 830.0);
    }

    #[test]
    fn taken_head_sends_the_next_one_to_the_top() {
        let mut t = Town::new(&[800.0, 300.0, 200.0]);
        fall_from(&mut t.crew.brains[1], 800.0, 700.0);
        t.run(1.5);
        fall_from(&mut t.crew.brains[2], 800.0, 500.0);
        t.run(1.5);
        assert_eq!(t.crew.brains[1].plat.unwrap().key, -1);
        assert_eq!(t.crew.brains[2].plat.unwrap().key, -2);
    }

    #[test]
    fn thrown_buddy_knocks_another_over() {
        let mut t = Town::new(&[1000.0, 400.0]);
        let b = &mut t.crew.brains[1];
        b.x = 900.0;
        b.y = 1035.0;
        b.knock(1200.0, -40.0, "bonk");
        b.can_crash = true;
        t.run(0.3);
        assert_eq!(t.crew.brains[0].mode, Mode::Fall);
        assert!(t.crew.brains[0].vx > 0.0);
    }

    #[test]
    fn tag_passes_back_and_forth() {
        let mut t = Town::new(&[600.0, 700.0]);
        t.crew.brains[1].start_tag(0, 2);
        for _ in 0..300 {
            t.run(DT);
            if t.crew.brains[0].chase_who == Some(1) { return; }
        }
        panic!("after being caught, a should chase b");
    }

    #[test]
    fn overlapping_buddies_slide_apart() {
        let mut t = Town::new(&[600.0, 610.0]);
        t.run(DT);
        let gap = (t.crew.brains[1].x - t.crew.brains[0].x).abs();
        assert!(gap > 10.0 && gap < 72.0);
        t.run(1.0);
        assert!((t.crew.brains[1].x - t.crew.brains[0].x).abs() >= 71.5);
    }

    #[test]
    fn held_buddy_stays_put_and_the_other_makes_room() {
        let mut t = Town::new(&[800.0, 300.0]);
        t.crew.brains[1].grab(300.0, 1040.0);
        t.crew.brains[1].x = 810.0;
        t.run(1.0);
        assert_eq!(t.crew.brains[1].x, 810.0);
        assert!((t.crew.brains[0].x - 810.0).abs() >= 71.5);
    }

    #[test]
    fn two_hundred_popping_out_of_one_buddy_all_settle() {
        let mut t = Town::new(&[960.0]);
        for i in 1..200u32 {
            let mut b = Brain::new(4.0, i as u64 * 7 + 3, i, "blue", 1.0);
            b.started = true;
            b.now = t.t;
            b.last_input = t.t;
            b.last_cmd = t.t;
            b.next_chat = 1e9;
            b.pop_out(960.0, 1020.0);
            t.crew.brains.push(b);
        }
        t.run(25.0);
        let flying = t.crew.brains.iter().filter(|b| b.plat.is_none()).count();
        assert!(flying <= 10, "{flying} still in the air");
    }

    #[test]
    fn a_crowd_too_big_for_the_floor_piles_up() {
        let xs: Vec<f64> = (0..60).map(|i| 300.0 + i as f64 * 20.0).collect();
        let mut t = Town::new(&xs);
        t.run(10.0);
        let riders = t.crew.brains.iter().filter(|b| b.plat.is_some_and(|p| p.kind == Kind::Buddy)).count();
        assert!(riders >= 30, "only {riders} climbed");
        let mut floor: Vec<f64> = t.crew.brains.iter().filter(|b| b.plat.is_some_and(|p| p.kind == Kind::Floor)).map(|b| b.x).collect();
        floor.sort_by(f64::total_cmp);
        assert!(floor.windows(2).all(|w| w[1] - w[0] >= 60.0), "the floor row should not overlap");
    }
}

//! Drawings that come alive: fire, cloud, bolt, bomb and platform. Pure logic, like the brain.
use crate::brain::{Brain, Kind, Platform, PropSpec, GRAVITY};

const MAX_PROPS: usize = 30;
const FUSE: f64 = 3.0;
const STRIKE_AT: f64 = 0.35;

#[derive(Clone, Debug)]
pub struct Prop {
    pub id: u32, pub kind: &'static str, pub x: f64, pub y: f64, pub size: f64, pub born: f64, pub ttl: f64,
    pub x2: f64, pub y2: f64, pub vy: f64, pub landed: f64, pub struck: bool,
}

impl Prop {
    fn platform(&self) -> Platform {
        Platform { key: -1_000_000 - self.id as i64, x1: self.x, x2: self.x2, y: self.y.min(self.y2), kind: Kind::Drawn }
    }
}

#[derive(Clone, Debug)]
pub enum PropEvent {
    Steam { x: f64, y: f64, size: f64 },
    Strike { x: f64, y: f64 },
    Boom { x: f64, y: f64, size: f64 },
}

#[derive(Default)]
pub struct Props { pub items: Vec<Prop>, next_id: u32, pub events: Vec<PropEvent> }

impl Props {
    pub fn add(&mut self, s: &PropSpec, now: f64) {
        self.next_id += 1;
        self.items.push(Prop { id: self.next_id, kind: s.kind, x: s.x, y: s.y, size: s.size, born: now, ttl: s.ttl, x2: s.x2, y2: s.y2,
                               vy: 0.0, landed: -1.0, struck: false });
        if self.items.len() > MAX_PROPS { self.items.remove(0); }
    }

    pub fn clear(&mut self) { self.items.clear(); }

    pub fn platforms(&self) -> impl Iterator<Item = Platform> + '_ {
        self.items.iter().filter(|p| p.kind == "platform").map(Prop::platform)
    }

    pub fn update(&mut self, now: f64, dt: f64, brains: &mut [Brain], platforms: &[Platform], unit: f64) {
        let ids: Vec<u32> = self.items.iter().map(|p| p.id).collect();
        let mut gone = Vec::new();
        let mut lit = Vec::new();
        for id in ids {
            let Some(i) = self.items.iter().position(|p| p.id == id) else { continue }; // put out by a cloud this frame
            let mut p = self.items[i].clone();
            let keep = (p.ttl == 0.0 || now - p.born < p.ttl) && match p.kind {
                "fire" => { fire(&p, brains); true }
                "cloud" => { self.rain(&p); true }
                "bolt" => { bolt(&mut p, now, brains, platforms, &mut self.events, &mut lit); true }
                "bomb" => bomb(&mut p, now, dt, brains, platforms, unit, &mut self.events),
                _ => true,
            };
            if let Some(slot) = self.items.iter_mut().find(|q| q.id == id) { *slot = p; }
            if !keep { gone.push(id); }
        }
        self.items.retain(|p| !gone.contains(&p.id));
        for spec in lit { self.add(&spec, now); }
    }

    fn rain(&mut self, cloud: &Prop) {
        let events = &mut self.events;
        self.items.retain(|f| {
            let hit = f.kind == "fire" && (f.x - cloud.x).abs() < cloud.size * 0.6 && f.y > cloud.y;
            if hit { events.push(PropEvent::Steam { x: f.x, y: f.y, size: f.size }); }
            !hit
        });
    }
}

fn fire(p: &Prop, brains: &mut [Brain]) {
    for b in brains {
        if (b.x - p.x).abs() < p.size * 0.45 + 4.0 * b.unit && p.y - p.size * 1.2 < b.y && b.y <= p.y + 6.0 * b.unit {
            b.burn(if b.x >= p.x { 1.0 } else { -1.0 });
        }
    }
}

fn bolt(p: &mut Prop, now: f64, brains: &mut [Brain], platforms: &[Platform], events: &mut Vec<PropEvent>, lit: &mut Vec<PropSpec>) {
    if p.struck || now - p.born < STRIKE_AT { return; }
    p.struck = true;
    let (bx, by) = (p.x - 0.22 * p.size, p.y + 1.6 * p.size);
    events.push(PropEvent::Strike { x: bx, y: by });
    for b in brains {
        if (b.x - bx).abs() < p.size * 0.5 && p.y < b.y && b.y < by + p.size * 0.6 { b.zap(); }
    }
    let floor = platforms.iter().filter(|q| q.x1 <= bx && bx <= q.x2 && q.y >= by - 2.0).map(|q| q.y).fold(f64::INFINITY, f64::min);
    if floor.is_finite() && floor - by < p.size * 0.8 {
        lit.push(PropSpec { kind: "fire", x: bx, y: floor, x2: 0.0, y2: 0.0, size: p.size * 0.45, ttl: 15.0 });
    }
}

#[allow(clippy::too_many_arguments)]
fn bomb(p: &mut Prop, now: f64, dt: f64, brains: &mut [Brain], platforms: &[Platform], unit: f64, events: &mut Vec<PropEvent>) -> bool {
    let r = p.size * 0.4;
    if p.landed < 0.0 {
        let prev = p.y + r;
        p.vy += GRAVITY * unit * dt;
        p.y += p.vy * dt;
        let below = platforms.iter().filter(|q| q.x1 <= p.x && p.x <= q.x2 && prev <= q.y && q.y <= p.y + r).map(|q| q.y).fold(f64::INFINITY, f64::min);
        if below.is_finite() {
            p.y = below - r;
            p.vy = 0.0;
            p.landed = now;
        }
        return p.y < 50000.0;
    }
    if now - p.landed < FUSE { return true; }
    let reach = p.size * 4.0;
    events.push(PropEvent::Boom { x: p.x, y: p.y, size: p.size });
    for b in brains {
        let (dx, dy) = (b.x - p.x, (b.y - 5.0 * b.unit) - p.y);
        let d = dx.hypot(dy);
        if d < reach {
            let push = (1.0 - d / reach) * 700.0 * b.unit + 150.0 * b.unit;
            let nx = if d > 1.0 { dx / d } else { 0.0 };
            b.knock(nx * push, -push.abs() * 0.9, "boom");
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::{Mode, World};

    const DT: f64 = 1.0 / 30.0;
    const FLOOR: Platform = Platform { key: 0, x1: 0.0, x2: 1920.0, y: 1040.0, kind: Kind::Floor };

    fn buddy(x: f64) -> Brain {
        let mut b = Brain::new(4.0, 1, 0, "orange", 1.0);
        b.x = x;
        b.y = 1040.0;
        b.started = true;
        b.now = 100.0;
        b.plat = Some(FLOOR);
        b.mode = Mode::Idle;
        b.timer = 50.0;
        b
    }

    fn spec(kind: &'static str, x: f64, y: f64, size: f64) -> PropSpec {
        PropSpec { kind, x, y, x2: 0.0, y2: 0.0, size, ttl: 0.0 }
    }

    #[test]
    fn fire_burns_a_buddy() {
        let mut props = Props::default();
        props.add(&spec("fire", 610.0, 1040.0, 70.0), 100.0);
        let mut crew = [buddy(600.0)];
        props.update(100.0, DT, &mut crew, &[FLOOR], 4.0);
        assert!(crew[0].charred());
        assert_eq!(crew[0].mode, Mode::Fall);
        assert!(crew[0].vx < 0.0);
    }

    #[test]
    fn rain_puts_out_a_fire() {
        let mut props = Props::default();
        props.add(&spec("fire", 600.0, 1040.0, 70.0), 100.0);
        props.add(&spec("cloud", 620.0, 700.0, 160.0), 100.0);
        props.update(100.0, DT, &mut [], &[FLOOR], 4.0);
        assert_eq!(props.items.len(), 1);
        assert_eq!(props.items[0].kind, "cloud");
    }

    #[test]
    fn bomb_falls_fizzes_and_booms() {
        let mut props = Props::default();
        props.add(&spec("bomb", 650.0, 500.0, 50.0), 100.0);
        let mut crew = [buddy(700.0)];
        let w = World::new(vec![FLOOR]);
        let mut t = 100.0;
        for _ in 0..(4.5 / DT) as usize {
            t += DT;
            crew[0].update(t, DT, &w);
            props.update(t, DT, &mut crew, &[FLOOR], 4.0);
        }
        assert!(props.items.is_empty());
        assert!(matches!(props.events.last(), Some(PropEvent::Boom { .. })));
    }

    #[test]
    fn lightning_zaps_and_starts_a_fire() {
        let mut props = Props::default();
        props.add(&spec("bolt", 627.0, 800.0, 120.0), 100.0);
        let mut crew = [buddy(600.0)];
        crew[0].now = 100.4;
        props.update(100.4, DT, &mut crew, &[FLOOR], 4.0);
        assert!(crew[0].charred());
        assert!(props.items.iter().any(|p| p.kind == "fire"));
    }
}

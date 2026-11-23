//! Clawd's pixel art, built from body, eye, arm and leg parts so every pose stays on-model.
use crate::brain::Mode;

pub const W: usize = 16;
pub const H: usize = 10;
pub const CLEAR: u8 = 0;
pub const BODY: u8 = 1;
pub const EYE: u8 = 2;
pub const BLUSH: u8 = 3;
pub const LOVE: u8 = 4;

pub const TINTS: [(&str, u32); 10] = [
    ("orange", 0xD77757), ("blue", 0x5B8DEF), ("green", 0x5DBB63), ("purple", 0x9B6BDF), ("pink", 0xE86FA6),
    ("yellow", 0xE8B73A), ("teal", 0x3FB8AF), ("red", 0xD9534F), ("white", 0xE8E4DC), ("black", 0x3A3A3A),
];

pub const HEART: [&str; 6] = [".hh.hh.", "hhhhhhh", "hhhhhhh", ".hhhhh.", "..hhh..", "...h..."];
pub const STAR: [&str; 5] = ["...y...", "..yyy..", "yyyyyyy", ".yyyyy.", ".yy.yy."];

/// Accessories as pixel art. Each is symmetric, so it needs no flipping. `.` is empty.
pub const HAT_ART: [&[&str]; 5] = [
    &[],
    &["...y...", "..rrr..", ".ryrry.", "rrrrrrr"],
    &["..kkkkk..", "..kkkkk..", "..rrrrr..", "kkkkkkkkk"],
    &["g..g..g", "g.ggg.g", "ggggggg"],
    &[".bbbbb.", "bbbbbbb", "wwwwwww"],
];
pub const GLASSES_ART: [&[&str]; 3] = [
    &[],
    &["............", "kkkkkkkkkkkk", ".kkk....kkk."],
    &[".kkk....kkk.", "k...kkkk...k", ".kkk....kkk."],
];

pub fn wear_color(c: u8) -> u32 {
    match c { b'r' => 0xE8443A, b'y' => 0xFFD23F, b'g' => 0xF5C542, b'b' => 0x3F6FD8, b'w' => 0xF2F2F2, _ => 0x17171B }
}

/// How many rows a pose's body sits below the standing position.
pub fn head_drop(frame: usize) -> usize {
    match frame { 5..=8 => 2, 14 => 1, _ => 0 }
}

pub fn tint_rgb(name: &str) -> [f32; 3] {
    let hex = TINTS.iter().find(|t| t.0 == name).map_or(TINTS[0].1, |t| t.1);
    rgb(hex)
}

pub fn rgb(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0]
}

#[derive(Clone, Copy)]
enum Eyes { Open, Up, Wide, Shut, Happy, Dizzy, Angry, Love }
#[derive(Clone, Copy)]
enum Arm { Mid, Up, Low, None }
#[derive(Clone, Copy)]
enum Legs { Stand, WalkA, WalkB, Tuck, None }

fn eye_pixels(e: Eyes) -> (&'static [(i32, i32)], u8) {
    match e {
        Eyes::Open => (&[(0, 1), (0, 2)], EYE),
        Eyes::Up => (&[(0, 0), (0, 1)], EYE),
        Eyes::Wide => (&[(0, 1), (1, 1), (0, 2), (1, 2)], EYE),
        Eyes::Shut => (&[(0, 2), (1, 2)], EYE),
        Eyes::Happy => (&[(-1, 2), (0, 1), (1, 2)], EYE),
        Eyes::Dizzy => (&[(-1, 1), (1, 1), (0, 2), (-1, 3), (1, 3)], EYE),
        Eyes::Angry => (&[(-1, 1), (0, 2), (1, 2)], EYE),
        Eyes::Love => (&[(-1, 1), (1, 1), (-1, 2), (0, 2), (1, 2), (0, 3)], LOVE),
    }
}

fn arm_pixels(a: Arm) -> &'static [(i32, i32)] {
    match a {
        Arm::Mid => &[(0, 2), (1, 2), (0, 3), (1, 3)],
        Arm::Up => &[(0, -2), (1, -2), (0, -1), (1, -1), (1, 0)],
        Arm::Low => &[(1, 3), (0, 4), (1, 4), (0, 5)],
        Arm::None => &[],
    }
}

fn leg_rows(l: Legs, leg: usize) -> &'static [i32] {
    match l {
        Legs::Stand => &[0, 1],
        Legs::WalkA => if leg % 2 == 0 { &[0, 1] } else { &[0] },
        Legs::WalkB => if leg % 2 == 0 { &[0] } else { &[0, 1] },
        Legs::Tuck => &[0],
        Legs::None => &[],
    }
}

type Grid = [[u8; W]; H];

fn build(eyes: Eyes, arms: (Arm, Arm), legs: Legs, drop: i32, splay: bool) -> Grid {
    let mut g = [[CLEAR; W]; H];
    let mut put = |x: i32, y: i32, v: u8| {
        if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) { g[y as usize][x as usize] = v; }
    };
    let top = 2 + drop;
    for y in top..top + 6 { for x in 2..=13 { put(x, y, BODY); } }
    for &(x, dy) in arm_pixels(arms.0) { put(x, top + dy, BODY); }
    for &(x, dy) in arm_pixels(arms.1) { put(W as i32 - 1 - x, top + dy, BODY); }
    let (pixels, value) = eye_pixels(eyes);
    for &(dx, dy) in pixels {
        put(4 + dx, top + dy, value);
        put(W as i32 - 5 - dx, top + dy, value);
    }
    if matches!(eyes, Eyes::Happy | Eyes::Love) {
        put(3, top + 4, BLUSH);
        put(W as i32 - 4, top + 4, BLUSH);
    }
    for (i, base) in [3, 5, 10, 12].into_iter().enumerate() {
        let x = if splay { base + [-1, 0, 0, 1][i] } else { base };
        for &r in leg_rows(legs, i) { put(x, top + 6 + r, BODY); }
    }
    g
}

pub const IDLE: usize = 0;
const BLINK: usize = 1;
const LOOK: usize = 2;
const WALK_A: usize = 3;
const WALK_B: usize = 4;
const SIT: usize = 5;
const TYPE_A: usize = 6;
const TYPE_B: usize = 7;
const SLEEP: usize = 8;
const WAVE_A: usize = 9;
const WAVE_B: usize = 10;
const DANCE_A: usize = 11;
const DANCE_B: usize = 12;
const HAPPY: usize = 13;
const SQUAT: usize = 14;
const JUMP: usize = 15;
const FALL: usize = 16;
const HELD_A: usize = 17;
const HELD_B: usize = 18;
const PEN: usize = 19;
const DIZZY: usize = 20;
const ANGRY_A: usize = 21;
const ANGRY_B: usize = 22;
const LOVE_F: usize = 23;
const THINK: usize = 24;
const SCARED: usize = 25;
pub const FRAME_COUNT: usize = 26;

fn frames() -> [Grid; FRAME_COUNT] {
    use Arm as A;
    use Eyes as E;
    use Legs as L;
    let m = (A::Mid, A::Mid);
    let up = (A::Up, A::Up);
    let low = (A::Low, A::Low);
    [
        build(E::Open, m, L::Stand, 0, false), build(E::Shut, m, L::Stand, 0, false), build(E::Up, m, L::Stand, 0, false),
        build(E::Open, m, L::WalkA, 0, false), build(E::Open, m, L::WalkB, 0, false), build(E::Open, m, L::None, 2, false),
        build(E::Open, (A::Low, A::Mid), L::None, 2, false), build(E::Open, (A::Mid, A::Low), L::None, 2, false),
        build(E::Shut, (A::None, A::None), L::None, 2, false), build(E::Open, (A::Mid, A::Up), L::Stand, 0, false),
        build(E::Happy, m, L::Stand, 0, false), build(E::Happy, (A::Up, A::Low), L::WalkA, 0, false),
        build(E::Happy, (A::Low, A::Up), L::WalkB, 0, false), build(E::Happy, up, L::Stand, 0, false),
        build(E::Open, low, L::Tuck, 1, false), build(E::Happy, up, L::Tuck, 0, false), build(E::Wide, up, L::Stand, 0, true),
        build(E::Wide, up, L::WalkA, 0, true), build(E::Wide, up, L::WalkB, 0, true), build(E::Happy, (A::Up, A::Mid), L::Tuck, 0, false),
        build(E::Dizzy, low, L::Stand, 0, false), build(E::Angry, (A::Up, A::Mid), L::Stand, 0, false),
        build(E::Angry, (A::Mid, A::Up), L::Stand, 0, false), build(E::Love, low, L::Stand, 0, false),
        build(E::Up, (A::Mid, A::Up), L::Stand, 0, false), build(E::Wide, up, L::Tuck, 0, false),
    ]
}

/// One row of frames: frame f occupies x = f * W .. (f + 1) * W.
pub fn atlas() -> Vec<u8> {
    let all = frames();
    let mut out = vec![CLEAR; W * FRAME_COUNT * H];
    for (f, grid) in all.iter().enumerate() {
        for (y, row) in grid.iter().enumerate() {
            out[y * W * FRAME_COUNT + f * W..y * W * FRAME_COUNT + (f + 1) * W].copy_from_slice(row);
        }
    }
    out
}

pub const GLOW_PAD: usize = 4;   // cells of empty space around the body for the light to spread into
pub const GLOW_RES: usize = 4;   // texture pixels per sprite cell
pub const GLOW_W: usize = (W + 2 * GLOW_PAD) * GLOW_RES;
pub const GLOW_H: usize = (H + 2 * GLOW_PAD) * GLOW_RES;

fn blur_pass(src: &[u16], dst: &mut [u16], w: usize, h: usize, r: usize, across: bool) {
    let (lines, len) = if across { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| if across { line * w + i } else { i * w + line };
    for line in 0..lines {
        let mut sum = 0u32;
        for i in 0..=r.min(len - 1) { sum += src[at(line, i)] as u32; }
        for i in 0..len {
            dst[at(line, i)] = (sum / (2 * r as u32 + 1)).min(u16::MAX as u32) as u16;
            if i + r + 1 < len { sum += src[at(line, i + r + 1)] as u32; }
            if i >= r { sum -= src[at(line, i - r)] as u32; }
        }
    }
}

/// A soft, blurred silhouette of every frame, so a glow follows Clawd's real shape.
pub fn glow_atlas() -> Vec<u8> {
    let all = frames();
    let mut out = vec![0u8; GLOW_W * FRAME_COUNT * GLOW_H];
    let (mut a, mut b) = (vec![0u16; GLOW_W * GLOW_H], vec![0u16; GLOW_W * GLOW_H]);
    for (f, grid) in all.iter().enumerate() {
        a.fill(0);
        for (y, row) in grid.iter().enumerate() {
            for (x, &v) in row.iter().enumerate() {
                if v == CLEAR { continue; }
                for py in 0..GLOW_RES {
                    for px in 0..GLOW_RES {
                        a[((y + GLOW_PAD) * GLOW_RES + py) * GLOW_W + (x + GLOW_PAD) * GLOW_RES + px] = u16::MAX;
                    }
                }
            }
        }
        // Three box blurs look like a gaussian, and together they stay inside the padding.
        for _ in 0..3 {
            blur_pass(&a, &mut b, GLOW_W, GLOW_H, 4, true);
            blur_pass(&b, &mut a, GLOW_W, GLOW_H, 4, false);
        }
        for y in 0..GLOW_H {
            for x in 0..GLOW_W { out[y * GLOW_W * FRAME_COUNT + f * GLOW_W + x] = (a[y * GLOW_W + x] >> 8) as u8; }
        }
    }
    out
}

/// Solid pixels of a frame as (column, row, value), for the explode effect.
pub fn pixels(frame: usize, flip: bool) -> Vec<(usize, usize, u8)> {
    let grid = frames()[frame.min(FRAME_COUNT - 1)];
    let mut out = Vec::new();
    for (r, row) in grid.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            if v != CLEAR { out.push((if flip { W - 1 - c } else { c }, r, v)); }
        }
    }
    out
}

fn flip2(clock: f64, rate: f64, a: usize, b: usize) -> usize {
    if (clock * rate) as i64 % 2 == 0 { a } else { b }
}

pub fn pose(mode: Mode, sub: &str, clock: f64, looking: bool) -> usize {
    match mode {
        Mode::Idle => {
            if clock % 3.2 < 0.15 { BLINK } else if looking { LOOK } else { IDLE }
        }
        Mode::Walk => flip2(clock, 8.0, WALK_A, WALK_B),
        Mode::Chase => flip2(clock, 12.0, WALK_A, WALK_B),
        Mode::Type => flip2(clock, 7.0, TYPE_A, TYPE_B),
        Mode::Sleep => SLEEP,
        Mode::Sit => SIT,
        Mode::Held => flip2(clock, 6.0, HELD_A, HELD_B),
        Mode::Fall | Mode::Slide => FALL,
        Mode::Hop => JUMP,
        Mode::Pen => PEN,
        Mode::Act => match sub {
            "wave" => flip2(clock, 5.0, WAVE_A, WAVE_B),
            "dance" | "celebrate" => flip2(clock, 6.0, DANCE_A, DANCE_B),
            "angry" => flip2(clock, 8.0, ANGRY_A, ANGRY_B),
            "happy" => HAPPY,
            "dizzy" => DIZZY,
            "love" => LOVE_F,
            "think" => THINK,
            "scared" => SCARED,
            "squat" => SQUAT,
            "spin" => JUMP,
            _ => IDLE,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_has_every_frame() {
        assert_eq!(atlas().len(), W * H * FRAME_COUNT);
    }

    #[test]
    fn glow_follows_the_body_and_fades_before_the_edge() {
        let g = glow_atlas();
        assert_eq!(g.len(), GLOW_W * FRAME_COUNT * GLOW_H);
        let at = |x: usize, y: usize| g[y * GLOW_W * FRAME_COUNT + x] as u32;
        assert!(at(GLOW_W / 2, GLOW_H / 2) > 200, "bright over the body");
        for x in 0..GLOW_W { assert_eq!(at(x, 0), 0, "no light at the top edge"); }
        for y in 0..GLOW_H { assert_eq!(at(0, y), 0, "no light at the left edge"); }
    }

    #[test]
    fn every_pose_is_a_valid_frame() {
        let modes = [Mode::Idle, Mode::Walk, Mode::Type, Mode::Sleep, Mode::Sit, Mode::Fall, Mode::Hop, Mode::Held,
                     Mode::Slide, Mode::Chase, Mode::Act, Mode::Pen];
        for m in modes {
            for sub in ["", "wave", "dance", "happy", "other", "squat", "spin", "angry", "love"] {
                for c in [0.0, 0.2, 1.0, 5.5] { assert!(pose(m, sub, c, false) < FRAME_COUNT); }
            }
        }
    }
}

//! Which parts of the desktop actually have something on them. The overlay covers the whole desktop,
//! but only these few tiles are cleared and handed to Windows to composite, which is what keeps the GPU quiet.

pub const CELL: i32 = 128;
pub const MAX_TILES: usize = 8;

const FRAME_STEP: i32 = 256;       // the buffer's edges snap to this, so it resizes rarely
const SHRINK_AFTER: u32 = 180;     // frames of mostly wasted space before it shrinks

pub type Tile = (i32, i32, i32, i32);

/// Picks the part of the desktop the drawing buffer covers. A small buffer is what saves the GPU,
/// and reallocating one is slow, so it only changes when the content leaves it or it's far too big.
#[derive(Clone, Debug, Default)]
pub struct Fit { frame: Option<Tile>, roomy: u32 }

impl Fit {
    pub fn next(&mut self, mask: &Mask) -> Option<Tile> {
        let need = mask.bounds()?;
        let snug = (snap_down(need.0 - FRAME_STEP / 2), snap_down(need.1 - FRAME_STEP / 2),
                    (snap_up(need.2 + FRAME_STEP / 2)).min(mask.w), (snap_up(need.3 + FRAME_STEP / 2)).min(mask.h));
        let Some(frame) = self.frame else {
            self.frame = Some(snug);
            return self.frame;
        };
        if !(frame.0 <= need.0 && frame.1 <= need.1 && frame.2 >= need.2 && frame.3 >= need.3) {
            // Keep the old area too if that's cheap: someone walking back and forth then causes no resizes.
            let both = join(frame, snug);
            self.frame = Some(if area(both) <= 3 * area(snug) { both } else { snug });
            self.roomy = 0;
            return self.frame;
        }
        self.roomy = if area(frame) > 3 * area(snug) { self.roomy + 1 } else { 0 };
        if self.roomy >= SHRINK_AFTER {
            self.roomy = 0;
            self.frame = Some(snug);
            return self.frame;
        }
        None
    }
}

fn snap_down(v: i32) -> i32 { v.max(0) / FRAME_STEP * FRAME_STEP }
fn snap_up(v: i32) -> i32 { (v + FRAME_STEP - 1) / FRAME_STEP * FRAME_STEP }

#[derive(Clone, Debug, PartialEq)]
pub struct Mask { w: i32, h: i32, cols: usize, rows: usize, cells: Vec<bool> }

impl Mask {
    pub fn new(w: i32, h: i32) -> Self {
        let (cols, rows) = (((w.max(1) + CELL - 1) / CELL) as usize, ((h.max(1) + CELL - 1) / CELL) as usize);
        Mask { w: w.max(1), h: h.max(1), cols, rows, cells: vec![false; cols * rows] }
    }

    pub fn full(w: i32, h: i32) -> Self {
        let mut m = Mask::new(w, h);
        m.fill();
        m
    }

    pub fn fill(&mut self) { self.cells.fill(true); }

    /// Marks every cell this box touches. Anything off the desktop is ignored.
    pub fn mark(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        if !(x1.is_finite() && y1.is_finite() && x2.is_finite() && y2.is_finite()) { self.fill(); return; }
        if x2 < 0.0 || y2 < 0.0 || x1 >= self.w as f32 || y1 >= self.h as f32 { return; }
        let cell = CELL as f32;
        let (c1, r1) = ((x1.max(0.0) / cell) as usize, (y1.max(0.0) / cell) as usize);
        let (c2, r2) = (((x2.max(0.0) / cell) as usize).min(self.cols - 1), ((y2.max(0.0) / cell) as usize).min(self.rows - 1));
        for r in r1..=r2 { self.cells[r * self.cols + c1..=r * self.cols + c2].fill(true); }
    }

    pub fn with(&self, other: &Mask) -> Mask {
        let mut out = self.clone();
        for (a, b) in out.cells.iter_mut().zip(&other.cells) { *a |= *b; }
        out
    }

    /// One rectangle around everything marked.
    pub fn bounds(&self) -> Option<Tile> {
        let mut out: Option<Tile> = None;
        for r in 0..self.rows {
            for c in 0..self.cols {
                if !self.cells[r * self.cols + c] { continue; }
                let t = (c as i32 * CELL, r as i32 * CELL, ((c as i32 + 1) * CELL).min(self.w), ((r as i32 + 1) * CELL).min(self.h));
                out = Some(out.map_or(t, |o| join(o, t)));
            }
        }
        out
    }

    /// The marked cells as a few rectangles that together cover all of them.
    pub fn tiles(&self) -> Vec<Tile> {
        let mut open: Vec<(usize, usize, usize, usize)> = Vec::new();   // first col, end col, first row, end row
        for r in 0..self.rows {
            let mut c = 0;
            while c < self.cols {
                if !self.cells[r * self.cols + c] { c += 1; continue; }
                let start = c;
                while c < self.cols && self.cells[r * self.cols + c] { c += 1; }
                match open.iter_mut().find(|t| t.0 == start && t.1 == c && t.3 == r) {
                    Some(t) => t.3 = r + 1,
                    None => open.push((start, c, r, r + 1)),
                }
            }
        }
        let mut tiles: Vec<Tile> = open.into_iter().map(|t| {
            (t.0 as i32 * CELL, t.2 as i32 * CELL, (t.1 as i32 * CELL).min(self.w), (t.3 as i32 * CELL).min(self.h))
        }).collect();
        // Too many: join whichever two waste the least space.
        while tiles.len() > MAX_TILES {
            let (mut best, mut waste) = ((0, 1), i64::MAX);
            for i in 0..tiles.len() {
                for j in i + 1..tiles.len() {
                    let extra = area(join(tiles[i], tiles[j])) - area(tiles[i]) - area(tiles[j]);
                    if extra < waste { (best, waste) = ((i, j), extra); }
                }
            }
            let merged = join(tiles[best.0], tiles[best.1]);
            tiles.swap_remove(best.1);
            tiles[best.0] = merged;
        }
        tiles
    }
}

fn join(a: Tile, b: Tile) -> Tile { (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)) }
fn area(t: Tile) -> i64 { (t.2 - t.0) as i64 * (t.3 - t.1) as i64 }

#[cfg(test)]
mod tests {
    use super::*;

    fn covers(tiles: &[Tile], x: i32, y: i32) -> bool { tiles.iter().any(|t| x >= t.0 && x < t.2 && y >= t.1 && y < t.3) }

    #[test]
    fn one_small_buddy_needs_a_tiny_part_of_the_desktop() {
        let mut m = Mask::new(3840, 1080);
        m.mark(1000.0, 900.0, 1100.0, 1000.0);
        let tiles = m.tiles();
        assert_eq!(tiles.len(), 1);
        assert!(area(tiles[0]) * 50 < 3840 * 1080);
        assert!(covers(&tiles, 1000, 900) && covers(&tiles, 1100, 1000));
    }

    #[test]
    fn edges_and_off_screen_boxes_are_safe() {
        let mut m = Mask::new(1000, 500);
        m.mark(-50.0, -50.0, 10.0, 10.0);
        m.mark(990.0, 490.0, 5000.0, 5000.0);
        m.mark(-500.0, 100.0, -400.0, 200.0);
        m.mark(2000.0, 100.0, 2100.0, 200.0);
        let tiles = m.tiles();
        assert_eq!(tiles.len(), 2);
        assert!(tiles.iter().all(|t| t.0 >= 0 && t.1 >= 0 && t.2 <= 1000 && t.3 <= 500));
    }

    #[test]
    fn a_scattered_crowd_stays_within_the_tile_limit_and_loses_nothing() {
        let mut m = Mask::new(3840, 1080);
        let spots: Vec<(i32, i32)> = (0..40).map(|i| (i * 311 % 3800, i * 173 % 1050)).collect();
        for &(x, y) in &spots { m.mark(x as f32, y as f32, x as f32 + 10.0, y as f32 + 10.0); }
        let tiles = m.tiles();
        assert!(tiles.len() <= MAX_TILES);
        for (x, y) in spots { assert!(covers(&tiles, x, y)); }
    }

    #[test]
    fn the_buffer_stays_small_follows_content_and_rarely_resizes() {
        let mut fit = Fit::default();
        let at = |x: f32| { let mut m = Mask::new(3840, 1080); m.mark(x, 900.0, x + 100.0, 1000.0); m };
        let first = fit.next(&at(1000.0)).unwrap();
        assert!(area(first) * 4 < 3840 * 1080, "one buddy gets a small buffer");
        assert_eq!(fit.next(&at(1010.0)), None, "small moves don't resize it");
        let moved = fit.next(&at(2500.0)).unwrap();
        assert!(moved.0 <= 2500 && moved.2 >= 2600, "it follows him");
        assert!(moved.2 <= 3840 && moved.3 <= 1080 && moved.0 >= 0 && moved.1 >= 0);
    }

    #[test]
    fn an_oversized_buffer_shrinks_only_after_a_while() {
        let mut fit = Fit::default();
        assert_eq!(fit.next(&Mask::full(3840, 1080)), Some((0, 0, 3840, 1080)));
        let mut small = Mask::new(3840, 1080);
        small.mark(100.0, 100.0, 120.0, 120.0);
        for _ in 0..SHRINK_AFTER - 1 { assert_eq!(fit.next(&small), None); }
        assert!(area(fit.next(&small).unwrap()) * 4 < 3840 * 1080);
    }

    #[test]
    fn nothing_on_screen_leaves_the_buffer_alone() {
        assert_eq!(Fit::default().next(&Mask::new(1920, 1080)), None);
    }

    #[test]
    fn a_broken_position_shows_everything_rather_than_hiding_it() {
        let mut m = Mask::new(640, 480);
        m.mark(f32::NAN, 0.0, 1.0, 1.0);
        assert_eq!(m, Mask::full(640, 480));
    }
}

//! Pen strokes as polylines in screen pixels, so a buddy can trace them out over time.
use std::f64::consts::{PI, TAU};

pub type P = (f64, f64);
pub const MAX_POINTS: usize = 400;

pub fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

pub fn length(path: &[P]) -> f64 {
    path.windows(2).map(|w| dist(w[0], w[1])).sum()
}

pub fn path_for(kind: &str, pts: &[P]) -> Result<Vec<P>, String> {
    if kind == "line" || kind == "curve" {
        if pts.len() < 2 { return Err(format!("{kind} needs at least 2 points")); }
        return Ok(if kind == "curve" { smooth(pts) } else { pts.to_vec() });
    }
    if pts.len() != 2 { return Err(format!("{kind} needs exactly 2 points")); }
    let ((x1, y1), (x2, y2)) = (pts[0], pts[1]);
    let (cx, cy) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    let (sx, sy) = ((x2 - x1).abs() / 2.0, (y2 - y1).abs() / 2.0);
    match kind {
        "rect" => Ok(vec![(x1, y1), (x2, y1), (x2, y2), (x1, y2), (x1, y1)]),
        "ellipse" => Ok((0..=48).map(|i| {
            let a = TAU * i as f64 / 48.0;
            (cx + sx * a.cos(), cy + sy * a.sin())
        }).collect()),
        "arrow" => {
            let len = (x2 - x1).hypot(y2 - y1);
            if len < 1.0 { return Err("arrow needs two different points".into()); }
            let angle = (y2 - y1).atan2(x2 - x1);
            let head = (len * 0.3).min(40.0);
            let wing = |d: f64| (x2 - head * (angle + d).cos(), y2 - head * (angle + d).sin());
            Ok(vec![(x1, y1), (x2, y2), wing(-0.45), (x2, y2), wing(0.45)])
        }
        "heart" => Ok((0..=64).map(|i| {
            let t = TAU * i as f64 / 64.0;
            let hx = 16.0 * t.sin().powi(3);
            let hy = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            (cx + hx / 17.0 * sx, cy - (hy + 2.0) / 15.0 * sy)
        }).collect()),
        "star" => Ok((0..11).map(|i| {
            let r = if i % 2 == 0 { 1.0 } else { 0.42 };
            let a = -PI / 2.0 + PI * i as f64 / 5.0;
            (cx + a.cos() * r * sx, cy + a.sin() * r * sy)
        }).collect()),
        _ => Err(format!("unknown kind {kind:?}")),
    }
}

/// A Catmull-Rom curve through every point, sampled about every 6 pixels.
pub fn smooth(pts: &[P]) -> Vec<P> {
    if pts.len() < 3 { return pts.to_vec(); }
    let mut ext = vec![pts[0]];
    ext.extend_from_slice(pts);
    ext.push(pts[pts.len() - 1]);
    let mut out = vec![pts[0]];
    for i in 1..ext.len() - 2 {
        let (p0, p1, p2, p3) = (ext[i - 1], ext[i], ext[i + 1], ext[i + 2]);
        let n = ((dist(p1, p2) / 6.0) as usize).clamp(2, 40);
        for j in 1..=n {
            let t = j as f64 / n as f64;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f64, b: f64, c: f64, d: f64| {
                0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3)
            };
            out.push((f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    out
}

const FLAME: [P; 13] = [(0.0, 0.0), (-0.42, -0.2), (-0.48, -0.55), (-0.3, -0.95), (-0.18, -0.6), (-0.08, -1.25),
    (0.08, -0.75), (0.22, -1.05), (0.3, -0.6), (0.45, -0.75), (0.46, -0.35), (0.35, -0.08), (0.0, 0.0)];
pub const BOLT: [P; 8] = [(0.05, 0.0), (-0.3, 0.75), (0.02, 0.72), (-0.22, 1.6), (0.38, 0.6), (0.06, 0.62), (0.3, 0.0), (0.05, 0.0)];

/// The pen path for a prop: fire anchored at its base, cloud and bomb at the middle, bolt at its top.
pub fn prop_outline(kind: &str, x: f64, y: f64, size: f64) -> Vec<P> {
    let place = |p: &[P]| -> Vec<P> { p.iter().map(|q| (x + q.0 * size, y + q.1 * size)).collect() };
    match kind {
        "fire" => smooth(&place(&FLAME)),
        "bolt" => place(&BOLT),
        "cloud" => (0..73).map(|i| {
            let a = TAU * i as f64 / 72.0;
            let bump = if a.sin() < 0.15 { 1.0 + 0.16 * (a * 3.5).sin().abs() } else { 1.0 };
            (x + a.cos() * 0.62 * size * bump, y + a.sin() * 0.3 * size * bump)
        }).collect(),
        _ => {
            let r = size * 0.4;
            let mut pts: Vec<P> = (0..49).map(|i| {
                let a = -PI / 2.0 + TAU * i as f64 / 48.0;
                (x + r * a.cos(), y + r * a.sin())
            }).collect();
            pts.extend(smooth(&[(x, y - r), (x + 0.12 * size, y - r - 0.18 * size), (x + 0.3 * size, y - r - 0.22 * size)]));
            pts
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_perimeter() {
        assert!((length(&path_for("rect", &[(0.0, 0.0), (10.0, 20.0)]).unwrap()) - 60.0).abs() < 1e-9);
    }

    #[test]
    fn arrow_has_head() {
        let path = path_for("arrow", &[(0.0, 0.0), (100.0, 0.0)]).unwrap();
        assert_eq!(path.len(), 5);
        assert_eq!(path[1], (100.0, 0.0));
    }

    #[test]
    fn bad_shapes_are_refused() {
        assert!(path_for("line", &[(0.0, 0.0)]).is_err());
        assert!(path_for("rect", &[(0.0, 0.0), (1.0, 2.0), (3.0, 4.0)]).is_err());
        assert!(path_for("circle", &[(0.0, 0.0), (1.0, 1.0)]).is_err());
    }
}

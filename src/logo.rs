//! A tiny turtle language, so Claude can draw a lot with very few tokens.
//!
//!     repeat 36 [fd 200 rt 170]                 a starburst
//!     color blue repeat 100 [fd :i*3 rt 91]     a square spiral; :i counts the innermost repeat from 1
//!     pu goto 300 200 pd circle 50              move without drawing, then a circle
use crate::shapes::P;

const MAX_POINTS: usize = 20_000;
const MAX_OPS: usize = 50_000;
const MAX_DEPTH: usize = 6;

#[derive(Clone, Debug)]
pub struct Stroke { pub pen: bool, pub pts: Vec<P>, pub color: String, pub width: f64 }

enum Cmd { Plain(String, Vec<String>), Repeat(String, Vec<Cmd>) }

fn arg_count(word: &str) -> Option<usize> {
    Some(match word {
        "fd" | "forward" | "bk" | "back" | "rt" | "right" | "lt" | "left" | "seth" | "setheading"
        | "color" | "pencolor" | "width" | "pensize" | "circle" => 1,
        "pu" | "penup" | "pd" | "pendown" => 0,
        "goto" | "setpos" | "arc" => 2,
        _ => return None,
    })
}

fn parse(t: &[String], mut i: usize, depth: usize) -> Result<(Vec<Cmd>, usize), String> {
    if depth > MAX_DEPTH { return Err("repeats are nested too deeply".into()); }
    let mut out = Vec::new();
    while i < t.len() {
        let word = t[i].to_lowercase();
        if word == "]" {
            if depth == 0 { return Err("unexpected ]".into()); }
            return Ok((out, i + 1));
        }
        if word == "repeat" {
            if i + 2 >= t.len() || t[i + 2] != "[" { return Err("repeat needs a count and a [ block ]".into()); }
            let (body, next) = parse(t, i + 3, depth + 1)?;
            out.push(Cmd::Repeat(t[i + 1].clone(), body));
            i = next;
            continue;
        }
        let n = arg_count(&word).ok_or_else(|| format!("unknown turtle command {:?}", t[i]))?;
        if i + n >= t.len() + usize::from(n == 0) || t.len() < i + 1 + n { return Err(format!("{word} needs {n} value(s)")); }
        out.push(Cmd::Plain(word, t[i + 1..i + 1 + n].to_vec()));
        i += 1 + n;
    }
    if depth > 0 { return Err("a repeat block is missing its ]".into()); }
    Ok((out, i))
}

/// Arithmetic on digits, + - * / and parentheses.
struct Calc<'a> { s: &'a [u8], i: usize }
impl Calc<'_> {
    fn space(&mut self) { while self.s.get(self.i) == Some(&b' ') { self.i += 1; } }
    fn peek(&mut self) -> Option<u8> { self.space(); self.s.get(self.i).copied() }
    fn expr(&mut self, depth: usize) -> Option<f64> {
        let mut v = self.term(depth)?;
        while let Some(c @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.term(depth)?;
            v = if c == b'+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(&mut self, depth: usize) -> Option<f64> {
        let mut v = self.atom(depth)?;
        while let Some(c @ (b'*' | b'/')) = self.peek() {
            self.i += 1;
            let r = self.atom(depth)?;
            if c == b'/' && r == 0.0 { return None; }
            v = if c == b'*' { v * r } else { v / r };
        }
        Some(v)
    }
    fn atom(&mut self, depth: usize) -> Option<f64> {
        if depth > 32 { return None; }
        match self.peek()? {
            b'-' => { self.i += 1; Some(-self.atom(depth + 1)?) }
            b'+' => { self.i += 1; self.atom(depth + 1) }
            b'(' => {
                self.i += 1;
                let v = self.expr(depth + 1)?;
                if self.peek()? != b')' { return None; }
                self.i += 1;
                Some(v)
            }
            _ => {
                let start = self.i;
                while matches!(self.s.get(self.i), Some(b'0'..=b'9' | b'.')) { self.i += 1; }
                std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
            }
        }
    }
}

fn num(text: &str, counter: u32) -> Result<f64, String> {
    let bad = || format!("not a number: {text:?}");
    let expr = text.replace(":i", &counter.to_string());
    let mut calc = Calc { s: expr.as_bytes(), i: 0 };
    let value = calc.expr(0).ok_or_else(bad)?;
    if calc.peek().is_some() || !value.is_finite() { return Err(bad()); }
    Ok(value)
}

struct Turtle { x: f64, y: f64, h: f64, down: bool, color: String, width: f64, segs: Vec<Stroke>, points: usize, ops: usize }

impl Turtle {
    fn fresh(&mut self) {
        self.segs.push(Stroke { pen: self.down, pts: vec![(self.x, self.y)], color: self.color.clone(), width: self.width });
    }
    fn restyle(&mut self) {
        let (down, color, width) = (self.down, self.color.clone(), self.width);
        let seg = self.segs.last_mut().unwrap();
        if seg.pts.len() == 1 {
            seg.pen = down;
            seg.color = color;
            seg.width = width;
        } else {
            self.fresh();
        }
    }
    fn to(&mut self, x: f64, y: f64) -> Result<(), String> {
        self.points += 1;
        if self.points > MAX_POINTS { return Err("drawing is too big".into()); }
        self.x = x.clamp(-20000.0, 20000.0);
        self.y = y.clamp(-20000.0, 20000.0);
        let at = (self.x, self.y);
        self.segs.last_mut().unwrap().pts.push(at);
        Ok(())
    }
    fn forward(&mut self, d: f64) -> Result<(), String> {
        let a = self.h.to_radians();
        self.to(self.x + d * a.cos(), self.y - d * a.sin())
    }
    fn arc(&mut self, degrees: f64, radius: f64) -> Result<(), String> {
        let steps = ((degrees.abs() / 5.0) as i64 + 1).clamp(2, 360);
        let turn = degrees / steps as f64;
        let chord = 2.0 * radius.abs() * (turn.abs().to_radians() / 2.0).sin();
        for _ in 0..steps {
            self.h += turn / 2.0;
            self.forward(chord)?;
            self.h += turn / 2.0;
        }
        Ok(())
    }
    fn run(&mut self, cmds: &[Cmd], counter: u32) -> Result<(), String> {
        for cmd in cmds {
            self.ops += 1;
            if self.ops > MAX_OPS { return Err("program runs too long".into()); }
            match cmd {
                Cmd::Repeat(count, body) => {
                    let times = num(count, counter)? as i64;
                    if !(0..=2000).contains(&times) { return Err("repeat count must be 0 to 2000".into()); }
                    for k in 1..=times { self.run(body, k as u32)?; }
                }
                Cmd::Plain(word, args) => match word.as_str() {
                    "pu" | "penup" | "pd" | "pendown" => { self.down = word == "pd" || word == "pendown"; self.restyle(); }
                    "color" | "pencolor" => { self.color = args[0].chars().take(24).collect(); self.restyle(); }
                    "width" | "pensize" => { self.width = num(&args[0], counter)?.clamp(1.0, 40.0); self.restyle(); }
                    "goto" | "setpos" => { let (x, y) = (num(&args[0], counter)?, num(&args[1], counter)?); self.to(x, y)?; }
                    "fd" | "forward" => self.forward(num(&args[0], counter)?)?,
                    "bk" | "back" => self.forward(-num(&args[0], counter)?)?,
                    "rt" | "right" => self.h -= num(&args[0], counter)?,
                    "lt" | "left" => self.h += num(&args[0], counter)?,
                    "seth" | "setheading" => self.h = num(&args[0], counter)?,
                    "arc" => { let (d, r) = (num(&args[0], counter)?, num(&args[1], counter)?); self.arc(d, r)?; }
                    _ => { let r = num(&args[0], counter)?; self.arc(360.0, r)?; }
                },
            }
        }
        Ok(())
    }
}

pub fn run(prog: &str, x: f64, y: f64, color: &str, width: f64) -> Result<Vec<Stroke>, String> {
    if prog.trim().is_empty() { return Err("prog needs some turtle commands".into()); }
    if prog.len() > 4000 { return Err("prog is too long".into()); }
    let spaced = prog.replace('[', " [ ").replace(']', " ] ").replace(',', " ");
    let tokens: Vec<String> = spaced.split_whitespace().map(String::from).collect();
    let (cmds, _) = parse(&tokens, 0, 0)?;
    let mut t = Turtle { x, y, h: 0.0, down: true, color: color.into(), width, segs: Vec::new(), points: 0, ops: 0 };
    t.fresh();
    t.run(&cmds, 0)?;
    let segs: Vec<Stroke> = t.segs.into_iter().filter(|s| s.pts.len() >= 2).collect();
    if !segs.iter().any(|s| s.pen) { return Err("that program draws nothing".into()); }
    Ok(segs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_closes() {
        let segs = run("repeat 4 [fd 100 rt 90]", 0.0, 0.0, "red", 4.0).unwrap();
        let end = *segs[0].pts.last().unwrap();
        assert!(end.0.abs() < 1e-6 && end.1.abs() < 1e-6);
    }

    #[test]
    fn counter_and_pen_up() {
        let segs = run("fd 10 pu fd 10 pd repeat 3 [fd :i*10]", 0.0, 0.0, "red", 4.0).unwrap();
        assert_eq!(segs.iter().map(|s| s.pen).collect::<Vec<_>>(), vec![true, false, true]);
        assert!((segs[2].pts.last().unwrap().0 - 80.0).abs() < 1e-9);
    }

    #[test]
    fn circle_returns_to_start() {
        let pts = run("circle 50", 100.0, 100.0, "red", 4.0).unwrap().remove(0).pts;
        let end = *pts.last().unwrap();
        assert!((end.0 - 100.0).abs() < 1e-6 && (end.1 - 100.0).abs() < 1e-6);
    }

    #[test]
    fn bad_programs_are_refused() {
        for prog in ["fd 10 jump", "repeat 3 [fd 1", "pu fd 100", "fd 9**9**9", "fd", "]", "fd 1/0", ""] {
            assert!(run(prog, 0.0, 0.0, "red", 4.0).is_err(), "{prog}");
        }
    }
}

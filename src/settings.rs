//! The user's choices, in %APPDATA%\Deskling\settings.json.
//! A missing or damaged file just means defaults: settings must never stop the buddy from starting.
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::brain::{Speech, Tune, LINE_KEYS};
use crate::menu::{MenuLook, THEMES};
use crate::sprites;

pub const PATTERNS: [&str; 4] = ["Solid", "Gradient", "Stripes", "Spots"];
pub const HATS: [&str; 5] = ["None", "Party", "Top hat", "Crown", "Beanie"];
pub const GLASSES: [&str; 3] = ["None", "Shades", "Nerd"];
pub const TRAILS: [&str; 3] = ["None", "Sparkles", "Hearts"];
pub const MAX_PHRASES: usize = 30;
pub const MAX_LINE: usize = 80;
pub const DEFAULT_SIZE: f64 = 9.0;
pub const SPEECH_MODES: [&str; 3] = ["Mix both", "Only my lines", "Built-in"];

/// How the whole crew looks. With `custom` off, each buddy keeps its own colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skin {
    pub custom: bool, pub color: u32, pub color2: u32, pub pattern: u8, pub hat: u8, pub glasses: u8, pub trail: u8, pub glow: bool,
    pub glow_power: f64, pub hat_size: f64, pub hat_lift: f64, pub glasses_size: f64, pub glasses_lift: f64,
}

impl Default for Skin {
    fn default() -> Self { Skin { custom: false, color: 0xD77757, color2: 0xFFD23F, pattern: 0, hat: 0, glasses: 0, trail: 0, glow: false,
               glow_power: 1.0, hat_size: 1.0, hat_lift: 0.0, glasses_size: 1.0, glasses_lift: 0.0 } }
}

/// How far each control can go. Unleashed mode opens them all up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits { pub size: f64, pub gravity: (f64, f64), pub speed: (f64, f64), pub bounce: f64, pub traits: f64, pub chaos: f64, pub friction: (f64, f64), pub max_speed: (f64, f64) }

pub const NORMAL: Limits = Limits { size: 12.0, gravity: (-2.5, 2.5), speed: (0.25, 3.0), bounce: 1.6, traits: 3.0, chaos: 1.0, friction: (-1.0, 3.0), max_speed: (100.0, 3000.0) };
pub const UNLEASHED: Limits = Limits { size: 300.0, gravity: (-12.0, 12.0), speed: (0.05, 25.0), bounce: 2.5, traits: 12.0, chaos: 5.0, friction: (-5.0, 12.0), max_speed: (50.0, 20000.0) };
/// The furthest a typed number can go, after the warning. Keeps the physics from breaking outright.
pub const TYPED: Limits = Limits { size: 300.0, gravity: (-50.0, 50.0), speed: (0.01, 100.0), bounce: 5.0, traits: 12.0, chaos: 5.0, friction: (-50.0, 50.0), max_speed: (10.0, 50000.0) };

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub type_along: bool,
    pub chatty: bool,
    pub pause_menu: bool,
    pub sounds: bool,
    pub pins: Vec<String>,
    pub size: f64,
    pub skin: Skin,
    pub tune: Tune,
    pub menu: MenuLook,
    pub phrases: Vec<String>,
    pub speech_mode: u8,
    pub lines: [String; 8],
    pub voice: bool,
    pub voice_volume: f64,
    pub voice_overlap: bool,
    pub unleashed: bool,
    pub custom_hat: String,
    pub custom_glasses: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { type_along: false, chatty: true, pause_menu: false, sounds: false, pins: Vec::new(), size: DEFAULT_SIZE,
                   skin: Skin::default(), tune: Tune::default(), menu: MenuLook::default(), phrases: Vec::new(), speech_mode: 0, lines: Default::default(), voice: false, voice_volume: 0.8, voice_overlap: true,
                   unleashed: false, custom_hat: String::new(), custom_glasses: String::new() }
    }
}

impl Settings {
    pub fn limits(&self) -> Limits { if self.unleashed { UNLEASHED } else { NORMAL } }

    /// Bring everything back inside the current limits, for when Unleashed mode is turned off.
    pub fn fit(&mut self) {
        let l = self.limits();
        let t = &mut self.tune;
        self.size = self.size.clamp(1.0, l.size).round();
        t.gravity = t.gravity.clamp(l.gravity.0, l.gravity.1);
        t.speed = t.speed.clamp(l.speed.0, l.speed.1);
        t.bounce = t.bounce.clamp(0.0, l.bounce);
        t.friction = t.friction.clamp(l.friction.0, l.friction.1);
        t.max_speed = t.max_speed.clamp(l.max_speed.0, l.max_speed.1);
        for v in [&mut t.energy, &mut t.chatter, &mut t.sleepy, &mut t.playful, &mut t.curiosity] { *v = v.min(l.traits); }
        t.chaos = t.chaos.clamp(0.0, l.chaos);
    }

    /// What the crew may say, without half-typed blank rows.
    pub fn speech(&self) -> Speech {
        Speech { mode: self.speech_mode, phrases: clean_lines(self.phrases.iter().map(String::as_str)), lines: self.lines.clone() }
    }
}

/// One-click personalities. Physics stays as the user set it.
pub fn apply_preset(t: &mut Tune, name: &str) {
    let (energy, chatter, curiosity, sleepy, playful, chaos) = match name {
        "calm" => (0.5, 0.4, 0.5, 1.8, 0.4, 0.0),
        "party" => (2.2, 1.8, 1.6, 0.4, 2.5, 0.6),
        "chaos" => (2.5, 2.0, 2.0, 0.25, 3.0, 1.0),
        _ => (1.0, 1.0, 1.0, 1.0, 1.0, 0.0),
    };
    t.energy = energy;
    t.chatter = chatter;
    t.curiosity = curiosity;
    t.sleepy = sleepy;
    t.playful = playful;
    t.chaos = chaos;
}

/// Trimmed, non-empty lines of a sensible length.
pub fn clean_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    lines.map(|p| p.trim().chars().take(MAX_LINE).collect::<String>()).filter(|p| !p.is_empty()).take(MAX_PHRASES).collect()
}

/// Deskling used to be called Desktop Buddy: bring its settings and imported images along once.
pub fn move_old_settings() {
    if std::env::var_os("DESKLING_SETTINGS").is_some() { return; }
    let Some(base) = std::env::var_os("APPDATA").map(PathBuf::from) else { return };
    let (old, new) = (base.join("DesktopBuddy"), base.join("Deskling"));
    if old.is_dir() && !new.exists() { let _ = std::fs::rename(old, new); }
}

pub fn default_path() -> PathBuf {
    // For testing without touching your real settings.
    if let Some(p) = std::env::var_os("DESKLING_SETTINGS") { return PathBuf::from(p); }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from)).unwrap_or_default();
    base.join("Deskling").join("settings.json")
}

fn flag(v: &Value, key: &str, out: &mut bool) {
    if let Some(x) = v.get(key).and_then(Value::as_bool) { *out = x; }
}

fn num(v: &Value, key: &str, lo: f64, hi: f64, out: &mut f64) {
    if let Some(x) = v.get(key).and_then(Value::as_f64) { if x.is_finite() { *out = x.clamp(lo, hi); } }
}

fn pick(v: &Value, key: &str, count: usize, out: &mut u8) {
    if let Some(x) = v.get(key).and_then(Value::as_u64) { if (x as usize) < count { *out = x as u8; } }
}

fn colour(v: &Value, key: &str, out: &mut u32) {
    if let Some(x) = v.get(key).and_then(Value::as_u64) { if x <= 0xFF_FFFF { *out = x as u32; } }
}

pub fn load(path: &Path) -> Settings {
    let mut out = Settings::default();
    let Ok(text) = std::fs::read_to_string(path) else { return out };
    let Ok(saved) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return out };
    flag(&saved, "type_along", &mut out.type_along);
    flag(&saved, "chatty", &mut out.chatty);
    flag(&saved, "pause_menu", &mut out.pause_menu);
    flag(&saved, "sounds", &mut out.sounds);
    flag(&saved, "unleashed", &mut out.unleashed);
    flag(&saved, "voice", &mut out.voice);
    flag(&saved, "voice_overlap", &mut out.voice_overlap);
    num(&saved, "voice_volume", 0.0, 1.0, &mut out.voice_volume);
    let l = out.limits();
    if let Some(v) = saved.get("custom_hat").and_then(Value::as_str) { out.custom_hat = v.chars().take(120).collect(); }
    if let Some(v) = saved.get("custom_glasses").and_then(Value::as_str) { out.custom_glasses = v.chars().take(120).collect(); }
    if let Some(v) = saved.get("pins").and_then(Value::as_array) {
        out.pins = v.iter().filter_map(Value::as_str).take(6).map(String::from).collect();
    }
    if let Some(v) = saved.get("phrases").and_then(Value::as_array) {
        out.phrases = clean_lines(v.iter().filter_map(Value::as_str));
    }
    pick(&saved, "speech_mode", SPEECH_MODES.len(), &mut out.speech_mode);
    if let Some(v) = saved.get("lines") {
        for (slot, (key, _)) in out.lines.iter_mut().zip(LINE_KEYS) {
            if let Some(t) = v.get(key).and_then(Value::as_str) { *slot = t.trim().chars().take(MAX_LINE).collect(); }
        }
    }
    num(&saved, "size", 1.0, l.size, &mut out.size);
    out.size = out.size.round();
    if let Some(v) = saved.get("look") {
        let s = &mut out.skin;
        flag(v, "custom", &mut s.custom);
        colour(v, "color", &mut s.color);
        colour(v, "color2", &mut s.color2);
        pick(v, "pattern", PATTERNS.len(), &mut s.pattern);
        pick(v, "hat", sprites::HAT_ART.len(), &mut s.hat);
        pick(v, "glasses", sprites::GLASSES_ART.len(), &mut s.glasses);
        pick(v, "trail", TRAILS.len(), &mut s.trail);
        flag(v, "glow", &mut s.glow);
        num(v, "glow_power", 0.2, 2.5, &mut s.glow_power);
        num(v, "hat_size", 0.3, 3.0, &mut s.hat_size);
        num(v, "hat_lift", -6.0, 6.0, &mut s.hat_lift);
        num(v, "glasses_size", 0.3, 3.0, &mut s.glasses_size);
        num(v, "glasses_lift", -6.0, 6.0, &mut s.glasses_lift);
    }
    if let Some(v) = saved.get("world") {
        let t = &mut out.tune;
        // The user may have confirmed a typed number beyond the sliders.
        num(v, "gravity", TYPED.gravity.0, TYPED.gravity.1, &mut t.gravity);
        num(v, "speed", TYPED.speed.0, TYPED.speed.1, &mut t.speed);
        num(v, "bounce", 0.0, TYPED.bounce, &mut t.bounce);
        num(v, "friction", TYPED.friction.0, TYPED.friction.1, &mut t.friction);
        num(v, "max_speed", TYPED.max_speed.0, TYPED.max_speed.1, &mut t.max_speed);
        flag(v, "collide", &mut t.collide);
        num(v, "energy", 0.25, l.traits, &mut t.energy);
        num(v, "chatter", 0.2, l.traits, &mut t.chatter);
        num(v, "curiosity", 0.0, l.traits, &mut t.curiosity);
        num(v, "sleepy", 0.25, l.traits, &mut t.sleepy);
        num(v, "playful", 0.2, l.traits, &mut t.playful);
        num(v, "chaos", 0.0, l.chaos, &mut t.chaos);
        flag(v, "climb", &mut t.climb);
        flag(v, "social", &mut t.social);
        flag(v, "naps", &mut t.naps);
    }
    if let Some(v) = saved.get("menu") {
        let m = &mut out.menu;
        flag(v, "list", &mut m.list);
        pick(v, "theme", THEMES.len(), &mut m.theme);
        flag(v, "rainbow", &mut m.rainbow);
        colour(v, "accent", &mut m.accent);
        num(v, "size", 0.85, 1.35, &mut m.size);
        num(v, "opacity", 0.6, 1.0, &mut m.opacity);
        num(v, "speed", 0.5, 2.0, &mut m.speed);
    }
    out
}

pub fn save(path: &Path, s: &Settings) -> bool {
    let temp = path.with_extension("json.tmp");
    let (k, t, m) = (&s.skin, &s.tune, &s.menu);
    let lines: serde_json::Map<String, Value> = LINE_KEYS.iter().zip(&s.lines).filter(|(_, l)| !l.trim().is_empty())
        .map(|(key, l)| (key.0.to_string(), json!(l.trim()))).collect();
    let phrases = clean_lines(s.phrases.iter().map(String::as_str));
    let body = serde_json::to_string_pretty(&json!({
        "type_along": s.type_along, "chatty": s.chatty, "pause_menu": s.pause_menu, "sounds": s.sounds, "pins": s.pins,
        "size": s.size, "phrases": phrases, "speech_mode": s.speech_mode, "lines": lines, "voice": s.voice, "voice_volume": s.voice_volume, "voice_overlap": s.voice_overlap, "unleashed": s.unleashed, "custom_hat": s.custom_hat, "custom_glasses": s.custom_glasses,
        "look": {"custom": k.custom, "color": k.color, "color2": k.color2, "pattern": k.pattern, "hat": k.hat,
                 "glasses": k.glasses, "trail": k.trail, "glow": k.glow, "glow_power": k.glow_power, "hat_size": k.hat_size,
                 "hat_lift": k.hat_lift, "glasses_size": k.glasses_size, "glasses_lift": k.glasses_lift},
        "world": {"gravity": t.gravity, "speed": t.speed, "bounce": t.bounce, "friction": t.friction, "collide": t.collide, "max_speed": t.max_speed, "energy": t.energy, "chatter": t.chatter,
                  "curiosity": t.curiosity, "sleepy": t.sleepy, "playful": t.playful, "chaos": t.chaos,
                  "climb": t.climb, "social": t.social, "naps": t.naps},
        "menu": {"list": m.list, "theme": m.theme, "rainbow": m.rainbow, "accent": m.accent, "size": m.size,
                 "opacity": m.opacity, "speed": m.speed},
    })).unwrap_or_default();
    path.parent().map_or(true, |d| std::fs::create_dir_all(d).is_ok())
        && std::fs::write(&temp, body + "\n").is_ok()
        && std::fs::rename(&temp, path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("buddy-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("settings.json")
    }

    #[test]
    fn missing_file_gives_defaults_with_typing_off() {
        let s = load(&temp("missing"));
        assert_eq!(s, Settings::default());
        assert!(!s.type_along);
        assert!(!s.pause_menu, "buddies keep moving while the menu is open unless the user asks");
        assert!(!s.sounds, "a desktop pet stays silent unless the user asks");
        assert!(!s.voice, "the robot voice is off unless the user asks");
        assert!(!s.menu.list, "the wheel is the default menu");
    }

    #[test]
    fn values_survive_a_restart() {
        let p = temp("save");
        let mut s = Settings { type_along: true, chatty: false, pause_menu: true, sounds: true, pins: vec!["Bomb".into(), "Dance".into()],
                               size: 6.0, phrases: vec!["hello".into()], ..Settings::default() };
        s.skin = Skin { custom: true, color: 0x112233, color2: 0x445566, pattern: 2, hat: 3, glasses: 1, trail: 2, glow: true, glow_power: 1.5, hat_lift: 2.0, ..Skin::default() };
        s.custom_hat = "cap.png".into();
        s.tune.naps = false;
        s.tune.friction = -0.5;
        s.tune.collide = false;
        s.tune.gravity = -7.0;
        s.tune.max_speed = 4000.0;
        s.voice = true;
        s.voice_overlap = false;
        s.speech_mode = 1;
        s.lines[0] = "zzzz".into();
        s.menu.list = true;
        s.menu.theme = 2;
        s.menu.accent = 0x00FF00;
        assert!(save(&p, &s));
        assert_eq!(load(&p), s);
    }

    #[test]
    fn damaged_or_wrong_types_give_defaults() {
        let p = temp("bad");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{not json").unwrap();
        assert_eq!(load(&p), Settings::default());
        std::fs::write(&p, r#"{"type_along": 1, "chatty": false, "other": 5}"#).unwrap();
        assert_eq!(load(&p), Settings { chatty: false, ..Settings::default() });
    }

    #[test]
    fn hand_edited_values_are_kept_in_range() {
        let p = temp("range");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"size": 900, "look": {"hat": 99, "color": 99999999}, "world": {"gravity": -500, "chaos": 40}, "menu": {"theme": 50, "size": 9}}"#).unwrap();
        let s = load(&p);
        assert_eq!(s.size, 12.0);
        assert_eq!(s.skin.hat, 0);
        assert_eq!(s.skin.color, Skin::default().color);
        assert_eq!(s.tune.gravity, TYPED.gravity.0);
        assert_eq!(s.tune.chaos, 1.0);
        assert_eq!(s.menu.theme, 0);
        assert_eq!(s.menu.size, 1.35);
    }

    #[test]
    fn unleashed_values_survive_and_are_pulled_back_when_it_ends() {
        let p = temp("wild");
        let mut s = Settings { unleashed: true, size: 150.0, ..Settings::default() };
        s.tune.gravity = 9.0;
        s.tune.chaos = 4.0;
        assert!(save(&p, &s));
        assert_eq!(load(&p), s);
        s.unleashed = false;
        s.fit();
        assert_eq!(s.size, 12.0);
        assert_eq!(s.tune.gravity, 2.5);
        assert_eq!(s.tune.chaos, 1.0);
    }

    #[test]
    fn lines_are_cleaned_and_may_contain_semicolons() {
        assert_eq!(clean_lines([" hi; you ", "", "  ", "there"].into_iter()), ["hi; you", "there"]);
        assert_eq!(clean_lines(std::iter::repeat("x").take(100)).len(), MAX_PHRASES);
    }

    #[test]
    fn presets_change_personality_but_not_physics() {
        let mut t = Tune { gravity: 2.0, ..Tune::default() };
        apply_preset(&mut t, "party");
        assert!(t.energy > 1.0);
        assert_eq!(t.gravity, 2.0);
        apply_preset(&mut t, "default");
        assert_eq!(t, Tune { gravity: 2.0, ..Tune::default() });
    }
}

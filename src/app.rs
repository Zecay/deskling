//! Main loop: commands from Claude, the surfaces around the buddies, the mouse, the menu and the frame clock.
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

use serde_json::{json, Value};
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{CreateEllipticRgn, CreateRoundRectRgn, SetWindowRgn};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::brain::{Brain, Event, Kind, Mode, Platform, Rng, Tune, HEIGHT};
use crate::crowd::{Crew, Frame, MAX_BUDDIES, UNLEASHED_BUDDIES};
use crate::gpu::Gpu;
use crate::ipc::{self, Request};
use crate::menu::{self, Action, Flags, Marquee, Outcome, Wheel};
use crate::props::Props;
use crate::scene::Scene;
use crate::settings::{self, Settings};
use crate::sound::{self, Cue};
use crate::sprites;
use crate::studio::Studio;
use crate::voice::{self, Voice};
use crate::wear::{self, Loaded, Slot, Wardrobe};
use crate::win;

const STEP: f64 = 1.0 / 60.0;         // the simulation always advances in these steps
const MAX_STEPS: u32 = 5;
const PLATFORM_EVERY: f64 = 1.0;
const KEYS_EVERY: f64 = 0.05;
const MIN_SCALE: f64 = 1.0;
const DOODLES: [&str; 6] = [
    "color #FF4D6D width 5 repeat 36 [fd 140 rt 170]",
    "color #4D9DFF width 3 repeat 60 [fd :i*3 rt 89]",
    "color #4DD599 width 4 repeat 12 [circle 40 rt 30]",
    "color #B36BFF width 5 repeat 5 [fd 160 rt 144]",
    "color #FF8A3D width 4 repeat 6 [repeat 3 [fd 100 rt 120] rt 60]",
    "color #FFD23F width 4 repeat 72 [fd :i*2 rt 59]",
];

#[derive(Clone, Copy)]
enum Mouse { Press, Release, Double, Menu, Wheel(i32) }

#[derive(Clone, Copy)]
enum MenuInput { Click, RightDown, RightUp, MiddleDown, Wheel(i32), Key(u16), Focus(bool) }

#[derive(Clone, Copy)]
enum SelectInput { Down, Up, Cancel }

#[derive(Clone, Copy)]
enum Shell { TrayClick, TrayMenu, Hotkey(i32) }

thread_local! {
    static MOUSE: RefCell<Vec<Mouse>> = const { RefCell::new(Vec::new()) };
    static MENU_INPUT: RefCell<Vec<MenuInput>> = const { RefCell::new(Vec::new()) };
    static SHELL: RefCell<Vec<Shell>> = const { RefCell::new(Vec::new()) };
    static SELECT_INPUT: RefCell<Vec<SelectInput>> = const { RefCell::new(Vec::new()) };
    static DISPLAY_CHANGED: Cell<bool> = const { Cell::new(false) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let shell = |e| SHELL.with(|q| q.borrow_mut().push(e));
    unsafe {
        if msg != 0 && msg == TASKBAR_CREATED.with(Cell::get) {   // Explorer restarted, so the tray is empty
            win::add_tray(hwnd);
            return LRESULT(0);
        }
        match msg {
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_DISPLAYCHANGE | WM_DPICHANGED => { DISPLAY_CHANGED.with(|d| d.set(true)); LRESULT(0) }
            WM_HOTKEY => { shell(Shell::Hotkey(wp.0 as i32)); LRESULT(0) }
            win::TRAY_MESSAGE => {
                match lp.0 as u32 {
                    WM_LBUTTONUP => shell(Shell::TrayClick),
                    WM_RBUTTONUP => shell(Shell::TrayMenu),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => { PostQuitMessage(0); LRESULT(0) }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

extern "system" fn menu_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let push = |m| MENU_INPUT.with(|q| q.borrow_mut().push(m));
    unsafe {
        match msg {
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => { push(MenuInput::Click); LRESULT(0) }
            WM_RBUTTONDOWN => { push(MenuInput::RightDown); LRESULT(0) }
            WM_RBUTTONUP => { push(MenuInput::RightUp); LRESULT(0) }
            WM_MBUTTONDOWN => { push(MenuInput::MiddleDown); LRESULT(0) }
            WM_MOUSEWHEEL => { push(MenuInput::Wheel(((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32)); LRESULT(0) }
            WM_KEYDOWN => { push(MenuInput::Key(wp.0 as u16)); LRESULT(0) }
            WM_ACTIVATE => { push(MenuInput::Focus((wp.0 & 0xFFFF) as u32 != WA_INACTIVE)); LRESULT(0) }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

extern "system" fn select_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let push = |m| SELECT_INPUT.with(|q| q.borrow_mut().push(m));
    unsafe {
        match msg {
            WM_LBUTTONDOWN => { let _ = SetCapture(hwnd); push(SelectInput::Down); LRESULT(0) }
            WM_LBUTTONUP => { let _ = ReleaseCapture(); push(SelectInput::Up); LRESULT(0) }
            WM_RBUTTONDOWN => { push(SelectInput::Cancel); LRESULT(0) }
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => { push(SelectInput::Cancel); LRESULT(0) }
            WM_ACTIVATE if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE => { push(SelectInput::Cancel); LRESULT(0) }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

extern "system" fn grab_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let push = |m| MOUSE.with(|q| q.borrow_mut().push(m));
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONDOWN => { let _ = SetCapture(hwnd); push(Mouse::Press); LRESULT(0) }
            WM_LBUTTONUP => { let _ = ReleaseCapture(); push(Mouse::Release); LRESULT(0) }
            WM_LBUTTONDBLCLK => { push(Mouse::Double); LRESULT(0) }
            WM_RBUTTONDOWN => { push(Mouse::Menu); LRESULT(0) }
            WM_MOUSEWHEEL => { push(Mouse::Wheel(((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32)); LRESULT(0) }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe fn make_windows(b: (i32, i32, i32, i32), ui: f64) -> windows::core::Result<(HWND, HWND, HWND, HWND)> {
    let module = GetModuleHandleW(None)?;
    RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(overlay_proc), hInstance: module.into(), lpszClassName: w!("DesklingOverlay"), ..Default::default() });
    RegisterClassW(&WNDCLASSW { style: CS_DBLCLKS, lpfnWndProc: Some(grab_proc), hInstance: module.into(), lpszClassName: w!("DesklingGrab"),
                                hCursor: LoadCursorW(None, IDC_HAND)?, ..Default::default() });
    let overlay = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        w!("DesklingOverlay"), w!("Deskling"), WS_POPUP, b.0, b.1, b.2 - b.0, b.3 - b.1, None, None, module, None)?;
    SetLayeredWindowAttributes(overlay, COLORREF(0), 255, LWA_ALPHA)?;
    let _ = ShowWindow(overlay, SW_SHOWNOACTIVATE);
    let grab = CreateWindowExW(WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        w!("DesklingGrab"), w!("Deskling grab"), WS_POPUP, 0, 0, 10, 10, None, None, module, None)?;
    SetLayeredWindowAttributes(grab, COLORREF(0), 1, LWA_ALPHA)?;
    // The wheel's input: almost invisible and round, so clicks beside the wheel reach the app under it.
    RegisterClassW(&WNDCLASSW { style: CS_DBLCLKS, lpfnWndProc: Some(menu_proc), hInstance: module.into(), lpszClassName: w!("DesklingMenu"),
                                hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() });
    let side = (2.0 * menu::REACH as f64 * ui).round() as i32;
    let menu = CreateWindowExW(WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        w!("DesklingMenu"), w!("Deskling menu"), WS_POPUP, 0, 0, side, side, None, None, module, None)?;
    SetLayeredWindowAttributes(menu, COLORREF(0), 1, LWA_ALPHA)?;
    // Only shown while dragging a selection box, so the desktop stays clickable the rest of the time.
    RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(select_proc), hInstance: module.into(), lpszClassName: w!("DesklingSelect"),
                                hCursor: LoadCursorW(None, IDC_CROSS)?, ..Default::default() });
    let select = CreateWindowExW(WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        w!("DesklingSelect"), w!("Deskling select"), WS_POPUP, b.0, b.1, b.2 - b.0, b.3 - b.1, None, None, module, None)?;
    SetLayeredWindowAttributes(select, COLORREF(0), 1, LWA_ALPHA)?;
    TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(w!("TaskbarCreated"))));
    win::add_tray(overlay);
    win::add_hotkeys(overlay);
    Ok((overlay, grab, menu, select))
}

struct Screen { gpu: Gpu, scene: Scene, overlay: HWND, grab: HWND, menu: HWND, select: HWND, grab_at: Option<(i32, i32, i32, i32)>, grab_id: Option<u32>, grab_lock: Option<u32> }

pub struct Options { pub port: u16, pub scale: Option<f64>, pub unleashed: bool, pub software: bool, pub background: bool, pub max_buddies: usize, pub headless: bool, pub bench: Option<Vec<usize>>, pub bench_menu: bool }

struct App {
    inbox: Receiver<Request>,
    running: bool,
    crew: Crew,
    props: Props,
    screen: Option<Screen>,
    settings: Settings,
    settings_path: Option<std::path::PathBuf>,
    max_buddies: usize,
    base_max: usize,
    closet: Wardrobe,
    closet_tx: std::sync::mpsc::Sender<Loaded>,
    closet_rx: Receiver<Loaded>,
    scale: f64,
    ui: f64,
    bounds: (f64, f64, f64, f64),
    platforms: Vec<Platform>,
    next_id: u32,
    rng: Rng,
    now: f64,
    next_platforms: f64,
    next_keys: f64,
    key_presses: u32,
    last_tick: u32,
    last_key_cursor: (i32, i32),
    press: Option<(u32, f64, f64, f64)>,
    dragging: bool,
    pet_rub: f64,
    last_cursor: (f64, f64),
    hidden: bool,
    user_hidden: bool,
    wheel: Option<Wheel>,
    studio: Option<Studio>,
    voice: Option<Voice>,
    save_at: Option<Instant>,
    marquee: Option<Marquee>,
    before_menu: HWND,
    paused: f64,                 // simulated time skipped while the menu held the buddies still
    work_ms: VecDeque<f32>,
    gap_ms: VecDeque<f32>,
    perf_count: usize,
}

fn local_hour() -> u32 {
    unsafe { windows::Win32::System::SystemInformation::GetLocalTime() }.wHour as u32
}

impl App {
    fn max_scale(&self) -> f64 { self.settings.limits().size }

    fn unit_for(&self, scale: f64) -> f64 { (scale * self.ui).round().max(1.0) }

    fn spawn(&mut self, near: Option<usize>, tint: Option<&str>) -> Result<(), String> {
        if self.crew.brains.len() >= self.max_buddies { return Err(format!("there can be at most {} buddies", self.max_buddies)); }
        let tint = match tint {
            Some(t) if sprites::TINTS.iter().any(|x| x.0 == t) => t.to_string(),
            Some(_) => return Err(format!("color must be one of: {}", sprites::TINTS.iter().map(|t| t.0).collect::<Vec<_>>().join(", "))),
            None => {
                // Past the palette's size every colour is taken, so skip the search in big crowds.
                let free = if self.crew.brains.len() >= sprites::TINTS.len() { None } else { sprites::TINTS.iter().find(|t| !self.crew.brains.iter().any(|b| b.tint == t.0)) };
                free.map_or_else(|| self.rng.pick(&sprites::TINTS).0, |t| t.0).to_string()
            }
        };
        let unit = self.unit_for(self.scale);
        let seed = (self.next_id as u64 + 1).wrapping_mul(0x51_7C_C1) ^ (self.rng.f() * 1e15) as u64;
        let mut b = Brain::new(unit, seed, self.next_id, &tint, self.ui);
        b.chatty = self.settings.chatty;
        b.tune = self.settings.tune;
        self.next_id += 1;
        if let Some(i) = near {
            let (nx, ny, nu) = { let n = &self.crew.brains[i]; (n.x, n.y, n.unit) };
            b.pop_out(nx, ny - HEIGHT * nu * 0.5);
            self.crew.brains[i].events.push(Event::Fx { kind: "sparkle", x: nx, y: ny - HEIGHT * nu });
        }
        self.crew.brains.push(b);
        Ok(())
    }

    fn remove(&mut self, id: u32) -> Result<(), String> {
        if self.crew.brains.len() <= 1 { return Err("the last buddy stays".into()); }
        let Some(i) = self.crew.brains.iter().position(|b| b.id == id) else { return Err(self.no_buddy(id)) };
        let b = self.crew.brains.remove(i);
        for o in &mut self.crew.brains { o.forget(id); }
        self.props.events.push(crate::props::PropEvent::Steam { x: b.x, y: b.y - 4.0 * b.unit, size: 12.0 * b.unit });
        // Slots moved, so the shared snapshot has to match before the next command reads it.
        self.crew.world.snaps = self.crew.brains.iter().map(Brain::snap).collect();
        self.crew.world.order.clear();
        self.crew.world.xs.clear();
        Ok(())
    }

    /// Keep only the first `n` buddies, all at once.
    fn keep_crew(&mut self, n: usize) {
        if n == 0 { if let Some(v) = &self.voice { v.stop(); } }
        if self.crew.brains.len() <= n { return; }
        let gone: std::collections::HashSet<u32> = self.crew.brains[n..].iter().map(|b| b.id).collect();
        self.crew.brains.truncate(n);
        for b in &mut self.crew.brains {
            if b.chase_who.is_some_and(|id| gone.contains(&id)) { b.chase_who = None; b.chase_until = 0.0; }
            if b.plat.is_some_and(|p| p.key < 0 && gone.contains(&((-p.key - 1) as u32))) { b.drop_off(); }
        }
        self.crew.world.snaps = self.crew.brains.iter().map(Brain::snap).collect();
        self.crew.world.order.clear();
        self.crew.world.xs.clear();
        if let Some(s) = &mut self.screen { s.scene.touch(); }
    }

    fn no_buddy(&self, id: u32) -> String {
        format!("no buddy with id {id}; ids are {}", self.ids())
    }

    fn ids(&self) -> String { self.crew.brains.iter().map(|b| b.id.to_string()).collect::<Vec<_>>().join(",") }

    fn targets(&self, id: Option<&Value>) -> Result<Vec<usize>, String> {
        let bad = || "id must be a buddy id or 'all'".to_string();
        match id {
            None | Some(Value::Null) if self.crew.brains.is_empty() => Err("there are no buddies right now; add one with crew".into()),
            None | Some(Value::Null) => Ok(vec![0]),
            Some(Value::String(s)) if s == "all" => Ok((0..self.crew.brains.len()).collect()),
            Some(Value::Bool(_)) => Err(bad()),
            Some(v) => {
                let n = v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).ok_or_else(bad)? as u32;
                self.crew.brains.iter().position(|b| b.id == n).map(|i| vec![i]).ok_or_else(|| self.no_buddy(n))
            }
        }
    }

    fn crew_cmd(&mut self, args: &Value) -> Result<String, String> {
        if let Some(add) = args.get("add") {
            let n = add.as_f64().ok_or("add must be a number")?.clamp(1.0, self.max_buddies as f64) as usize;
            let tint = args.get("color").and_then(Value::as_str).map(String::from);
            for _ in 0..n {
                if self.crew.brains.len() >= self.max_buddies { break; }
                let near = (!self.crew.brains.is_empty()).then_some(0);
                self.spawn(near, tint.as_deref())?;
            }
        }
        if let Some(remove) = args.get("remove") {
            let gone: Vec<u32> = if remove.as_str() == Some("all") {
                self.crew.brains.iter().skip(1).map(|b| b.id).collect()
            } else {
                self.targets(Some(remove))?.into_iter().map(|i| self.crew.brains[i].id).collect()
            };
            for id in gone { self.remove(id)?; }
        }
        if let Some(size) = args.get("size") {
            let scale = size.as_f64().ok_or("size must be a number")?.clamp(MIN_SCALE, self.max_scale());
            let unit = self.unit_for(scale);
            let default_all = json!("all");
            for i in self.targets(Some(args.get("id").unwrap_or(&default_all)))? { self.crew.brains[i].resize(unit); }
        }
        Ok(format!("{} buddies, ids {}", self.crew.brains.len(), self.ids()))
    }

    fn status(&self) -> String {
        let b = self.bounds;
        json!({"buddies": self.crew.brains.iter().map(Brain::status).collect::<Vec<_>>(),
               "cursor": [self.last_cursor.0, self.last_cursor.1], "screen": [b.0, b.1, b.2, b.3],
               "typing": self.crew.brains.iter().any(|x| x.now < x.typing_until)}).to_string()
    }

    fn perf(&self) -> String {
        let sorted = |d: &VecDeque<f32>| { let mut v: Vec<f32> = d.iter().copied().collect(); v.sort_by(f32::total_cmp); if v.is_empty() { v.push(0.0); } v };
        let (work, gaps) = (sorted(&self.work_ms), sorted(&self.gap_ms));
        let avg = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        format!("tick avg {:.1}ms max {:.1}ms; frame gap avg {:.1}ms p95 {:.1}ms max {:.1}ms", avg(&work), work[work.len() - 1],
                avg(&gaps), gaps[((gaps.len() as f32 * 0.95) as usize).saturating_sub(1)], gaps[gaps.len() - 1])
    }

    fn command(&mut self, cmd: &str, args: &Value) -> Result<String, String> {
        if !args.is_object() { return Err("args must be an object".into()); }
        match cmd {
            "status" => return Ok(self.status()),
            "crew" => return self.crew_cmd(args),
            "perf" => return Ok(self.perf()),
            "studio" => { self.open_studio(0); return Ok("studio open".into()); }
            _ => {}
        }
        let mut clean = args.clone();
        let id = clean.as_object_mut().and_then(|m| m.remove("id"));
        let targets = self.targets(id.as_ref())?;
        let mut first = String::new();
        for &i in &targets {
            let reply = self.crew.brains[i].command(cmd, &clean, &self.crew.world)?;
            if first.is_empty() { first = reply; }
        }
        Ok(if targets.len() == 1 { first } else { format!("{} buddies: {first}", targets.len()) })
    }

    fn drain(&mut self) {
        while let Ok(req) = self.inbox.try_recv() {
            if req.cmd == "quit" {
                let _ = req.reply.send((true, "bye".into()));
                self.running = false;
                continue;
            }
            let (ok, msg) = match self.command(&req.cmd, &req.args) { Ok(m) => (true, m), Err(e) => (false, e) };
            let _ = req.reply.send((ok, msg));
        }
    }

    fn remember(&mut self) {
        if let Some(p) = &self.settings_path { settings::save(p, &self.settings); }
    }

    // ---- the world ------------------------------------------------------------------

    fn refresh_platforms(&mut self) {
        let (l, t, r, b) = self.bounds;
        let Some(screen) = &self.screen else {
            self.platforms = vec![Platform { key: 0, x1: l, x2: r, y: b, kind: Kind::Floor }];
            return;
        };
        let own = [screen.overlay.0 as isize, screen.grab.0 as isize, screen.menu.0 as isize, screen.select.0 as isize];
        let floors: Vec<(f64, f64, f64)> = win::floors().into_iter().map(|f| ((f.0 as f64).max(l), (f.1 as f64).min(r), f.2 as f64)).collect();
        let mut list: Vec<Platform> = floors.iter().enumerate().map(|(i, f)| Platform { key: i as i64, x1: f.0, x2: f.1, y: f.2, kind: Kind::Floor }).collect();
        if list.is_empty() { list.push(Platform { key: 0, x1: l, x2: r, y: b, kind: Kind::Floor }); }
        let wide = 40.0 * self.unit_for(self.scale);
        for e in win::window_edges(own, (t + 70.0 * self.ui) as i32, b as i32) {
            let (x1, x2) = ((e.left as f64).max(l), (e.right as f64).min(r));
            // The taskbar's top edge is the floor: one surface, or buddies on each would overlap.
            if e.taskbar && floors.iter().any(|f| f.0 < x2 && x1 < f.1 && (e.top as f64 - f.2).abs() <= 6.0 * self.ui) { continue; }
            if x2 - x1 >= wide {
                list.push(Platform { key: e.hwnd as i64, x1, x2, y: e.top as f64, kind: if e.taskbar { Kind::Taskbar } else { Kind::Window } });
            }
        }
        self.platforms = list;
        if let Some(st) = &self.studio { st.raise(); }   // only while it's the active window
        for b in &mut self.crew.brains {
            if let Some(p) = b.plat {
                if matches!(p.kind, Kind::Window | Kind::Taskbar) {
                    let hit = win::window_at(b.x as i32, p.y as i32 + 2);
                    if hit != 0 && !own.contains(&hit) && hit != p.key as isize { b.drop_off(); }
                }
            }
        }
    }

    /// Windows reports *that* input happened. No pointer movement or click means it was the keyboard.
    fn poll_typing(&mut self) {
        let (tick, _) = win::last_input();
        let cursor = win::cursor();
        if tick != self.last_tick && cursor == self.last_key_cursor && !win::any_button_down() { self.key_presses += 1; }
        self.last_tick = tick;
        self.last_key_cursor = cursor;
    }

    // ---- the mouse ------------------------------------------------------------------

    fn index_of(&self, id: u32) -> Option<usize> { self.crew.brains.iter().position(|b| b.id == id) }

    fn mouse_events(&mut self) {
        let events: Vec<Mouse> = MOUSE.with(|q| std::mem::take(&mut *q.borrow_mut()));
        for ev in events {
            let Some(id) = self.screen.as_ref().and_then(|s| s.grab_id) else { continue };
            let Some(i) = self.index_of(id) else { continue };
            match ev {
                Mouse::Press => {
                    let c = win::cursor();
                    self.press = Some((id, c.0 as f64, c.1 as f64, self.now));
                    self.dragging = false;
                    if let Some(s) = &mut self.screen { s.grab_lock = Some(id); }
                }
                Mouse::Release => self.release(),
                Mouse::Double => self.crew.brains[i].trick(),
                Mouse::Menu => {
                    let b = &self.crew.brains[i];
                    let at = (b.x, b.y - sprites::H as f64 * b.unit * 0.5);
                    self.open_menu(id, at);
                }
                Mouse::Wheel(delta) => {
                    let step = if delta > 0 { 1.0 } else { -1.0 };
                    let scale = ((self.crew.brains[i].unit / self.ui).round() + step).clamp(MIN_SCALE, self.max_scale());
                    let unit = self.unit_for(scale);
                    self.crew.brains[i].resize(unit);
                }
            }
        }
    }

    fn release(&mut self) {
        if let Some((id, _, _, at)) = self.press.take() {
            if let Some(i) = self.index_of(id) {
                if self.dragging { self.crew.brains[i].release(); } else if self.now - at < 0.5 { self.crew.brains[i].poke(); }
            }
        }
        self.dragging = false;
        if let Some(s) = &mut self.screen { s.grab_lock = None; }
    }

    fn mouse(&mut self, cursor: (f64, f64)) {
        let pressed = self.press.and_then(|p| self.index_of(p.0));
        if self.press.is_some() && pressed.is_none() { self.press = None; }
        if let (Some(i), Some(p)) = (pressed, self.press) {
            if !win::left_down() {
                self.release();
            } else {
                if !self.dragging && (cursor.0 - p.1).abs() + (cursor.1 - p.2).abs() > 5.0 * self.ui {
                    self.dragging = true;
                    self.crew.brains[i].grab(p.1, p.2);
                }
                if self.dragging { self.crew.brains[i].hold(cursor.0, cursor.1, &self.crew.world); }
            }
        } else {
            let moved = (cursor.0 - self.last_cursor.0).abs();
            let on_studio = self.studio.as_ref().is_some_and(|st| st.covers(cursor.0 as i32, cursor.1 as i32));
            let over = if on_studio { None } else { self.crew.brains.iter().position(|o| (cursor.0 - o.x).abs() < 8.0 * o.unit && o.y - 10.0 * o.unit < cursor.1 && cursor.1 < o.y) };
            self.pet_rub = self.pet_rub * 0.96 + if over.is_some() { moved } else { 0.0 };
            if let Some(i) = over {
                if self.pet_rub > 260.0 * self.ui {
                    self.pet_rub = 0.0;
                    self.crew.brains[i].pet();
                }
            }
        }
        self.last_cursor = cursor;
    }

    fn place_grab(&mut self) {
        let (cx, cy) = self.last_cursor;
        // Studio and the taskbar always win: a giant buddy must never block the way to turn him off.
        let reserved = self.studio.as_ref().is_some_and(|st| st.covers(cx as i32, cy as i32))
            || self.platforms.iter().filter(|p| p.kind == Kind::Floor && p.x1 <= cx && cx <= p.x2).map(|p| p.y).fold(f64::NEG_INFINITY, f64::max) < cy
                && self.platforms.iter().any(|p| p.kind == Kind::Floor && p.x1 <= cx && cx <= p.x2);
        let above = self.studio.as_ref().filter(|st| st.is_on_top()).map_or(HWND_TOPMOST, Studio::hwnd);
        let brains = &self.crew.brains;
        let Some(s) = &mut self.screen else { return };
        let margin = (3.0 * self.ui).round();
        if s.grab_lock.is_some() && !win::left_down() { s.grab_lock = None; }
        let mut target = s.grab_lock.and_then(|id| brains.iter().find(|b| b.id == id));
        if target.is_none() && !reserved && self.wheel.is_none() && self.marquee.is_none() {
            target = brains.iter().rev().find(|b| {
                (cx - b.x).abs() <= sprites::W as f64 * b.unit / 2.0 + margin && b.y - sprites::H as f64 * b.unit - margin <= cy && cy <= b.y + margin
            });
        }
        s.grab_id = target.map(|b| b.id);
        let wanted = target.map(|b| {
            let (w, h) = ((sprites::W as f64 * b.unit).round(), (sprites::H as f64 * b.unit).round());
            ((b.x - w / 2.0 - margin).round() as i32, (b.y - h - margin).round() as i32, (w + 2.0 * margin) as i32, (h + 2.0 * margin) as i32)
        });
        if wanted != s.grab_at {
            s.grab_at = wanted;
            unsafe {
                match wanted {
                    Some((x, y, w, h)) => { let _ = SetWindowPos(s.grab, above, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW); }
                    None => { let _ = ShowWindow(s.grab, SW_HIDE); }
                }
            }
        }
    }

    // ---- the radial menu ------------------------------------------------------------

    fn menu_flags(&self) -> Flags {
        let n = self.crew.brains.len();
        Flags { can_add: n < self.max_buddies, can_remove: n > 1 }
    }

    fn open_menu(&mut self, id: u32, at: (f64, f64)) {
        if self.wheel.is_some() { self.close_menu(); }
        let flags = self.menu_flags();
        let look = self.settings.menu;
        let ui = self.ui * look.size;
        let Some(s) = &mut self.screen else { return };
        // Keep the whole menu, and the label above it, on the monitor the buddy is on.
        let area = win::work_area_at(at.0 as i32, at.1 as i32);
        let (hw, hh) = menu::half_size(&look, ui as f32);
        let (hw, hh) = (hw as f64, hh as f64);
        let top = hh + 36.0 * ui;
        let x = at.0.min(area.2 as f64 - hw).max(area.0 as f64 + hw);
        let y = at.1.min(area.3 as f64 - hh).max(area.1 as f64 + top);
        let (side_w, side_h) = ((hw * 2.0).round() as i32, (hh * 2.0).round() as i32);
        MENU_INPUT.with(|q| q.borrow_mut().clear());
        unsafe {
            let front = GetForegroundWindow();
            if front != s.select { self.before_menu = front; }    // after a box select, keep the app from before it
            // Clicks just outside the shape reach whatever app is under it. Windows owns the region once it's set.
            let round = (16.0 * ui) as i32;
            let shape = if look.list { CreateRoundRectRgn(0, 0, side_w + 1, side_h + 1, round, round) } else { CreateEllipticRgn(0, 0, side_w + 1, side_h + 1) };
            SetWindowRgn(s.menu, shape, false);
            let _ = SetWindowPos(s.menu, HWND_TOPMOST, (x - hw).round() as i32, (y - hh).round() as i32, side_w, side_h, SWP_SHOWWINDOW);
            let _ = SetForegroundWindow(s.menu);
            let _ = SetFocus(s.menu);
            let _ = ShowWindow(s.grab, SW_HIDE);
        }
        s.grab_at = None;
        s.grab_lock = None;
        self.press = None;
        self.dragging = false;
        let mut wheel = Wheel::new(id, x as f32, y as f32, ui as f32, flags);
        wheel.look = look;
        wheel.set_pins(&self.settings.pins);
        self.wheel = Some(wheel);
        self.cue(Cue::Open);
    }

    fn cue(&self, c: Cue) {
        if self.settings.sounds && self.screen.is_some() { sound::play(c); }
    }

    // ---- selecting a group ----------------------------------------------------------

    fn start_select(&mut self) {
        // Focus goes straight to the selection window. Handing it back first would cancel the selection.
        self.shut_menu(false);
        let (l, t, r, b) = self.bounds;
        let c = win::cursor();
        let Some(s) = &mut self.screen else { return };
        SELECT_INPUT.with(|q| q.borrow_mut().clear());
        unsafe {
            let front = GetForegroundWindow();
            if front != s.menu { self.before_menu = front; }
            let _ = SetWindowPos(s.select, HWND_TOPMOST, l as i32, t as i32, (r - l) as i32, (b - t) as i32, SWP_SHOWWINDOW);
            let _ = SetForegroundWindow(s.select);
            let _ = SetFocus(s.select);
            let _ = ShowWindow(s.grab, SW_HIDE);
        }
        s.grab_at = None;
        s.grab_lock = None;
        self.press = None;
        self.dragging = false;
        self.marquee = Some(Marquee { from: None, to: (c.0 as f32, c.1 as f32), age: 0.0 });
    }

    fn end_select(&mut self, restore: bool) {
        if self.marquee.take().is_none() { return; }
        SELECT_INPUT.with(|q| q.borrow_mut().clear());
        if let Some(s) = &mut self.screen {
            s.scene.touch();
            unsafe {
                let _ = ReleaseCapture();
                let ours = GetForegroundWindow() == s.select;
                let _ = ShowWindow(s.select, SW_HIDE);
                if restore && ours && !self.before_menu.0.is_null() { let _ = SetForegroundWindow(self.before_menu); }
            }
        }
    }

    fn finish_select(&mut self) {
        let Some(m) = &self.marquee else { return };
        let mut ids: Vec<u32> = self.crew.brains.iter().filter(|b| {
            m.touches(b.x as f32, (b.y - sprites::H as f64 * b.unit) as f32, b.y as f32, (sprites::W as f64 * b.unit / 2.0) as f32)
        }).map(|b| b.id).collect();
        ids.sort_unstable();
        let Some(&first) = ids.first() else {
            // Nobody in the box: stay in selection mode so they can simply drag again.
            if let Some(m) = &mut self.marquee {
                m.from = None;
                m.age = 0.0;
            }
            return;
        };
        let centre = m.rect().map_or((0.0, 0.0), |r| (((r.0 + r.2) / 2.0) as f64, ((r.1 + r.3) / 2.0) as f64));
        self.open_menu(first, centre);
        self.end_select(false);
        if let Some(w) = &mut self.wheel { if ids.len() > 1 { w.group = ids; } }
    }

    fn select_step(&mut self, cursor: (f64, f64)) {
        let inputs: Vec<SelectInput> = SELECT_INPUT.with(|q| std::mem::take(&mut *q.borrow_mut()));
        let at = (cursor.0 as f32, cursor.1 as f32);
        if let Some(m) = &mut self.marquee {
            m.to = at;
            m.age += STEP as f32;
        }
        for input in inputs {
            match input {
                SelectInput::Down => if let Some(m) = &mut self.marquee { m.from = Some(at); },
                // The release of the click that chose "Select" arrives here too, and isn't a drag.
                SelectInput::Up => if self.marquee.as_ref().is_some_and(|m| m.from.is_some()) {
                    self.finish_select();
                    return;
                },
                SelectInput::Cancel => { self.end_select(true); return; }
            }
        }
        // This window covers the whole desktop, so it must never linger.
        let Some(m) = &self.marquee else { return };
        if m.from.is_none() && m.age > 15.0 { self.end_select(true); }
        else if m.from.is_some() && !win::left_down() { self.finish_select(); }
    }

    fn close_menu(&mut self) { self.shut_menu(true); }

    fn shut_menu(&mut self, give_focus_back: bool) {
        if self.wheel.take().is_none() { return; }
        MENU_INPUT.with(|q| q.borrow_mut().clear());
        if let Some(s) = &mut self.screen {
            s.scene.touch();
            unsafe {
                let ours = GetForegroundWindow() == s.menu;
                let _ = ShowWindow(s.menu, SW_HIDE);
                if give_focus_back && ours && !self.before_menu.0.is_null() { let _ = SetForegroundWindow(self.before_menu); }
            }
        }
    }

    fn menu_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Nothing => {}
            Outcome::Close => self.close_menu(),
            Outcome::Run(action) => {
                let Some((id, group)) = self.wheel.as_ref().map(|w| (w.buddy, w.group.clone())) else { return };
                if action.per_buddy() && !group.is_empty() {
                    for g in group { self.perform(action, g); }
                } else {
                    self.perform(action, id);
                }
                self.cue(Cue::Pick);
                if !action.keeps_open() { self.close_menu(); }
            }
        }
    }

    fn menu_step(&mut self, cursor: (f64, f64)) {
        let inputs: Vec<MenuInput> = MENU_INPUT.with(|q| std::mem::take(&mut *q.borrow_mut()));
        let (cx, cy) = (cursor.0 as f32, cursor.1 as f32);
        for input in inputs {
            let Some(w) = self.wheel.as_mut() else { break };
            let mut pins_changed = false;
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let outcome = match input {
                MenuInput::Click => w.click(cx, cy),
                MenuInput::RightDown => w.back(),
                MenuInput::RightUp => w.right_release(cx, cy),
                MenuInput::MiddleDown => { w.pointer(cx, cy); pins_changed = w.toggle_pin(); Outcome::Nothing }
                MenuInput::Wheel(delta) => { w.cycle(if delta > 0 { -1 } else { 1 }); Outcome::Nothing }
                MenuInput::Focus(true) => { w.had_focus = true; Outcome::Nothing }
                MenuInput::Focus(false) => if w.had_focus { Outcome::Close } else { Outcome::Nothing },
                MenuInput::Key(k) => {
                    let k = VIRTUAL_KEY(k);
                    if k == VK_ESCAPE || k == VK_BACK { w.back() }
                    else if k == VK_RETURN || k == VK_SPACE { w.confirm() }
                    else if k == VK_P { pins_changed = w.toggle_pin(); Outcome::Nothing }
                    else if shift && (k == VK_RIGHT || k == VK_DOWN) { pins_changed = w.shift_pin(1); Outcome::Nothing }
                    else if shift && (k == VK_LEFT || k == VK_UP) { pins_changed = w.shift_pin(-1); Outcome::Nothing }
                    else if k == VK_RIGHT || k == VK_DOWN || k == VK_TAB { w.cycle(1); Outcome::Nothing }
                    else if k == VK_LEFT || k == VK_UP { w.cycle(-1); Outcome::Nothing }
                    else { Outcome::Nothing }
                }
            };
            if pins_changed {
                self.settings.pins = w.pin_labels();
                self.remember();
            }
            self.menu_outcome(outcome);
        }
        let flags = self.menu_flags();
        let alive = self.wheel.as_ref().is_some_and(|w| self.crew.brains.iter().any(|b| b.id == w.buddy));
        if self.wheel.is_some() && !alive { self.close_menu(); }
        let moved = cursor != self.last_cursor;
        let down = win::any_button_down();
        let mut outside_click = false;
        let mut reaction = None;
        if let Some(w) = self.wheel.as_mut() {
            w.flags = flags;
            if moved { w.pointer(cx, cy); }
            w.step(STEP as f32);
            if let Some(item) = w.hover_change() { reaction = Some((w.buddy, item)); }
            if !down { w.armed = true; }
            // A safety net in case Windows refused to give the wheel keyboard focus.
            let (hw, hh) = menu::half_size(&w.look, w.ui);
            let beyond = if w.look.list { (cx - w.x).abs() > hw || (cy - w.y).abs() > hh } else { (cx - w.x).hypot(cy - w.y) > hw };
            outside_click = w.armed && down && beyond;
        }
        if let Some((id, item)) = reaction {
            self.cue(Cue::Tick);
            // The buddy shows what you're pointing at. While paused it stays still.
            if let (Some(it), false, Some(i)) = (item, self.settings.pause_menu, self.index_of(id)) {
                self.crew.brains[i].events.push(Event::Glyph(it.icon));
            }
        }
        if outside_click { self.close_menu(); }
    }

    fn perform(&mut self, action: Action, id: u32) {
        let Some(i) = self.index_of(id) else { return };
        let (x, y, facing) = { let b = &self.crew.brains[i]; (b.x, b.y, b.facing as f64) };
        let ui = self.ui;
        let aim = menu::aim(action, x, y, facing, ui);
        let order = match action {
            Action::Act(a) => Some(("act", json!({"action": a}))),
            Action::Doodle => { let prog = self.rng.pick(&DOODLES); Some(("turtle", json!({"prog": prog, "ttl": 15}))) }
            Action::Heart | Action::Star => {
                let (kind, color) = if action == Action::Heart { ("heart", "#FF4D6D") } else { ("star", "#FFD23F") };
                aim.map(|p| ("draw", json!({"kind": kind, "pts": [[p[0].0, p[0].1], [p[1].0, p[1].1]], "color": color, "ttl": 15})))
            }
            Action::Ledge => aim.map(|p| ("draw", json!({"kind": "platform", "pts": [[p[0].0, p[0].1], [p[1].0, p[1].1]]}))),
            Action::Fire | Action::Rain | Action::Bomb => {
                let kind = match action { Action::Fire => "fire", Action::Rain => "cloud", _ => "bomb" };
                aim.map(|p| ("draw", json!({"kind": kind, "pts": [[p[0].0, p[0].1]]})))
            }
            Action::Fireworks => Some(("fx", json!({"kind": "fireworks"}))),
            Action::Snow => Some(("fx", json!({"kind": "snow"}))),
            Action::ClearInk => Some(("draw", json!({"kind": "clear"}))),
            _ => None,
        };
        if let Some((cmd, args)) = order { let _ = self.crew.brains[i].command(cmd, &args, &self.crew.world); }
        match action {
            Action::Add => { let _ = self.spawn(Some(i), None); }
            Action::AddTen => for _ in 0..10 { let _ = self.spawn(Some(i), None); },
            Action::Remove => { let _ = self.remove(id); }
            Action::RemoveAll => self.keep_crew(0),
            Action::Bigger | Action::Smaller => {
                let step = if action == Action::Bigger { 1.0 } else { -1.0 };
                let scale = ((self.crew.brains[i].unit / ui).round() + step).clamp(MIN_SCALE, self.max_scale());
                let unit = self.unit_for(scale);
                self.crew.brains[i].resize(unit);
            }
            Action::Summon => self.summon(),
            Action::Studio(page) => self.open_studio(page as usize),
            Action::Select => self.start_select(),
            Action::HideAll => self.set_user_hidden(true),
            Action::Quit => self.running = false,
            _ => {}
        }
    }

    // ---- Deskling Studio ---------------------------------------------------------------

    fn open_studio(&mut self, page: usize) {
        if self.screen.is_none() { return; }
        self.close_menu();
        match &mut self.studio {
            Some(st) => st.show(page),
            None => match Studio::new(self.ui, page) {
                Ok(st) => self.studio = Some(st),
                Err(e) => win::alert(&format!("Could not open Deskling Studio: {e}")),
            },
        }
    }

    fn studio_frame(&mut self) {
        let Some(st) = &mut self.studio else {
            self.flush_settings(false);
            return;
        };
        // Above the buddies only while you're using it, so it never covers your other apps.
        // The wheel and box select are drawn on the overlay, so it steps back for those too.
        st.keep_on_top(st.focused() && self.wheel.is_none() && self.marquee.is_none());
        let out = st.frame(&mut self.settings, &self.closet, self.crew.brains.len());
        if out.add > 0 {
            for _ in 0..out.add {
                let near = (!self.crew.brains.is_empty()).then_some(0);
                if self.spawn(near, None).is_err() { break; }
            }
        }
        if out.remove_all { self.keep_crew(0); }
        if out.quit { self.running = false; }
        if out.test_voice { self.voice.get_or_insert_with(Voice::new).say("Hello. I am Clawd.", self.settings.voice_volume, true, 1.0); }
        if let Some(slot) = out.import { wear::import(slot, self.closet_tx.clone()); }
        if let Some((slot, name)) = &out.delete {
            self.closet.remove(*slot, name);
            let picked = if *slot == Slot::Hat { &mut self.settings.custom_hat } else { &mut self.settings.custom_glasses };
            if picked == name { picked.clear(); }
            self.apply_settings();
            self.save_at = Some(Instant::now());
        }
        if out.changed {
            self.apply_settings();
            self.save_at = Some(Instant::now());   // written shortly after, so dragging a slider doesn't hammer the disk
        }
        if out.try_menu {
            let c = win::cursor();
            if let Some(id) = self.crew.brains.first().map(|b| b.id) { self.open_menu(id, (c.0 as f64, c.1 as f64)); }
        }
        if out.close {
            self.studio = None;
            self.flush_settings(true);
        } else {
            self.flush_settings(false);
        }
    }

    /// Imported pictures arriving from the background thread.
    fn closet_events(&mut self) {
        let mut any = false;
        while let Ok(item) = self.closet_rx.try_recv() {
            if item.select {
                if item.slot == Slot::Hat {
                    self.settings.custom_hat.clone_from(&item.wear.name);
                    self.settings.skin.hat = 0;
                } else {
                    self.settings.custom_glasses.clone_from(&item.wear.name);
                    self.settings.skin.glasses = 0;
                }
                self.save_at = Some(Instant::now());
            }
            self.closet.add(item.slot, item.wear);
            any = true;
        }
        if any { self.apply_settings(); }
    }

    fn flush_settings(&mut self, now: bool) {
        if self.save_at.is_some_and(|t| now || t.elapsed().as_millis() > 500) {
            self.save_at = None;
            self.remember();
        }
    }

    /// Push the saved choices into the running crew.
    fn apply_settings(&mut self) {
        self.max_buddies = if self.settings.unleashed { UNLEASHED_BUDDIES } else { self.base_max };
        self.keep_crew(self.max_buddies);
        let chatty = self.settings.chatty;
        let tune = Tune { wild: self.settings.unleashed, ..self.settings.tune };
        for b in &mut self.crew.brains {
            b.chatty = chatty;
            b.tune = tune;
        }
        self.crew.world.tune = tune;
        self.crew.world.speech = std::sync::Arc::new(self.settings.speech());
        if !self.settings.voice { if let Some(v) = &self.voice { v.stop(); } }
        if (self.settings.size - self.scale).abs() > 0.01 {
            self.scale = self.settings.size.clamp(MIN_SCALE, self.max_scale());
            let unit = self.unit_for(self.scale);
            for b in &mut self.crew.brains { b.resize(unit); }
        }
        if let Some(s) = &mut self.screen {
            s.scene.skin = self.settings.skin;
            s.scene.set_wear(&s.gpu, self.closet.find(Slot::Hat, &self.settings.custom_hat), self.closet.find(Slot::Glasses, &self.settings.custom_glasses));
            s.scene.touch();
        }
    }

    /// Everyone leaps to the pointer, spread out so they land side by side.
    fn summon(&mut self) {
        let c = win::cursor();
        let n = self.crew.brains.len();
        let gap = 18.0 * self.unit_for(self.scale);
        let (l, r) = (self.bounds.0 + 20.0, self.bounds.2 - 20.0);
        for k in 0..n {
            let x = (c.0 as f64 + (k as f64 - (n as f64 - 1.0) / 2.0) * gap).clamp(l, r);
            let _ = self.crew.brains[k].command("leap", &json!({"x": x, "y": c.1}), &self.crew.world);
        }
    }

    fn set_user_hidden(&mut self, hide: bool) {
        self.user_hidden = hide;
        self.set_hidden(hide);
    }

    /// Tray clicks and global hotkeys. These must still work while everyone is hidden.
    fn shell_events(&mut self) {
        let events: Vec<Shell> = SHELL.with(|q| std::mem::take(&mut *q.borrow_mut()));
        for ev in events {
            match ev {
                // With nobody left, the tray is how you get a buddy back.
                Shell::TrayClick if self.crew.brains.is_empty() => {
                    self.set_user_hidden(false);
                    let _ = self.spawn(None, None);
                }
                Shell::TrayMenu | Shell::Hotkey(1) if self.crew.brains.is_empty() => {
                    self.set_user_hidden(false);
                    self.open_studio(2);
                }
                Shell::TrayClick | Shell::Hotkey(2) => { let hide = !self.user_hidden; self.set_user_hidden(hide); }
                Shell::TrayMenu | Shell::Hotkey(1) => {
                    self.set_user_hidden(false);
                    let c = win::cursor();
                    if let Some(id) = self.crew.brains.first().map(|b| b.id) { self.open_menu(id, (c.0 as f64, c.1 as f64)); }
                }
                Shell::Hotkey(3) => { self.set_user_hidden(false); self.summon(); }
                Shell::Hotkey(4) => {
                    if !self.crew.brains.is_empty() { let _ = self.crew.brains[0].command("draw", &json!({"kind": "clear"}), &self.crew.world); }
                }
                Shell::Hotkey(6) => self.keep_crew(0),
                Shell::Hotkey(5) => { self.set_user_hidden(false); self.start_select(); }
                Shell::Hotkey(_) => {}
            }
        }
    }

    // ---- one fixed step -------------------------------------------------------------

    fn step(&mut self, now: f64) {
        self.now = now;
        self.drain();
        if now >= self.next_keys {
            self.next_keys = now + KEYS_EVERY;
            if self.screen.is_some() && self.settings.type_along { self.poll_typing(); }
        }
        if now >= self.next_platforms {
            self.next_platforms = now + PLATFORM_EVERY;
            self.refresh_platforms();
        }
        let (cursor, idle) = if self.screen.is_some() {
            let c = win::cursor();
            self.mouse_events();
            let c = (c.0 as f64, c.1 as f64);
            self.menu_step(c);
            if self.marquee.is_some() { self.select_step(c); }
            self.mouse(c);
            (c, win::last_input().1)
        } else {
            (self.last_cursor, 0.0)
        };
        if self.frozen() {          // the user chose to hold everyone still while the menu is open
            self.paused += STEP;
            self.place_grab();
            return;
        }
        let now = now - self.paused;
        let mut ground = self.platforms.clone();
        ground.extend(self.props.platforms());
        let surfaces = ground.clone();
        let frame = Frame { ground, cursor, bounds: self.bounds, key_presses: self.key_presses, idle_s: idle, hour: local_hour(), _marker: Default::default() };
        self.crew.step(now, STEP, frame);
        self.key_presses = 0;
        self.speak_bubbles(now);
        let mut events = Vec::with_capacity(self.crew.brains.len());
        for b in &mut self.crew.brains {
            let evs = std::mem::take(&mut b.events);
            for ev in &evs {
                match ev {
                    Event::Prop(spec) => self.props.add(spec, now),
                    Event::Clear => self.props.clear(),
                    _ => {}
                }
            }
            if !evs.is_empty() { events.push((b.id, evs)); }
        }
        let unit = self.unit_for(self.scale);
        self.props.update(now, STEP, &mut self.crew.brains, &surfaces, unit);
        let prop_events = std::mem::take(&mut self.props.events);
        if let Some(s) = &mut self.screen {
            s.scene.step(&self.crew.brains, &self.props, events, prop_events, now, STEP, Some(&s.gpu));
        }
        self.place_grab();
    }

    /// Reads one new speech bubble aloud, if the robot voice is on.
    fn speak_bubbles(&mut self, now: f64) {
        if !self.settings.voice || self.screen.is_none() || self.hidden { return; }
        let (volume, overlap) = (self.settings.voice_volume, self.settings.voice_overlap);
        let voice = self.voice.get_or_insert_with(Voice::new);
        for b in &mut self.crew.brains {
            if b.bubble_id != b.voiced {
                b.voiced = b.bubble_id;
                if now < b.bubble_until { voice.say(&b.bubble_text, volume, overlap, voice::pitch_for(b.id)); }
            }
        }
    }

    fn frozen(&self) -> bool { self.wheel.is_some() && self.settings.pause_menu }

    /// Our own time this frame, leaving out the wait for the display's next refresh.
    fn last_work(&self, began: Instant) -> f32 {
        let waited = self.screen.as_ref().map_or(0.0, |s| s.gpu.present_ms.get());
        (began.elapsed().as_secs_f32() * 1000.0 - waited).max(0.0)
    }

    fn set_hidden(&mut self, hide: bool) {
        if hide == self.hidden { return; }
        if hide {
            self.close_menu();
            self.end_select(false);
        }
        self.hidden = hide;
        if let Some(s) = &mut self.screen {
            unsafe {
                let _ = ShowWindow(s.overlay, if hide { SW_HIDE } else { SW_SHOWNOACTIVATE });
                if hide {
                    let _ = ShowWindow(s.grab, SW_HIDE);
                    s.grab_at = None;
                } else {
                    s.scene.touch();
                }
            }
        }
    }

    fn display_changed(&mut self) {
        let b = win::virtual_screen();
        self.bounds = (b.0 as f64, b.1 as f64, b.2 as f64, b.3 as f64);
        self.next_platforms = 0.0;
        self.close_menu();
        self.end_select(true);
        if let Some(s) = &mut self.screen {
            unsafe { let _ = SetWindowPos(s.overlay, HWND_TOPMOST, b.0, b.1, b.2 - b.0, b.3 - b.1, SWP_NOACTIVATE); }
            let _ = s.gpu.resize((b.2 - b.0).max(1) as u32, (b.3 - b.1).max(1) as u32);
            s.scene.set_bounds(b);
        }
    }
}

pub fn run(opt: Options) -> i32 {
    let (tx, inbox) = channel();
    if ipc::serve(opt.port, tx).is_err() {
        if opt.background || opt.bench.is_some() || opt.headless {
            eprintln!("Deskling is already running on port {}.", opt.port);
            return 1;
        }
        // Launching it a second time is how many people will look for the settings.
        if ipc::send(opt.port, "studio", &json!({})).is_ok_and(|r| r.0) { return 0; }
        win::alert(&format!("Port {} is being used by another app.\nSet the DESKLING_PORT environment variable to a different number.", opt.port));
        return 1;
    }
    let (bounds, ui, screen) = if opt.headless {
        ((0, 0, 1920, 1080), 1.0, None)
    } else {
        unsafe { let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); }
        let b = win::virtual_screen();
        let ui = unsafe { GetDpiForSystem() } as f64 / 96.0;
        let made = unsafe { make_windows(b, ui) }.and_then(|(overlay, grab, menu, select)| {
            let gpu = Gpu::new(overlay, (b.2 - b.0) as u32, (b.3 - b.1) as u32, ui as f32, opt.software)?;
            Ok(Screen { gpu, scene: Scene::new(b, ui as f32), overlay, grab, menu, select, grab_at: None, grab_id: None, grab_lock: None })
        });
        match made {
            Ok(s) => (b, ui, Some(s)),
            Err(e) => {
                eprintln!("could not start the graphics: {e}");
                win::alert(&format!("Could not start the graphics.\nDeskling needs Windows 10 or 11.\n\n{e}"));
                return 1;
            }
        }
    };
    let settings_path = (!opt.headless).then(settings::default_path);
    let mut settings = settings_path.as_deref().map_or_else(Settings::default, settings::load);
    if opt.unleashed { settings.unleashed = true; }
    if let Some(v) = opt.scale { settings.size = v.clamp(MIN_SCALE, settings.limits().size).round(); }
    let scale = settings.size;
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64);
    let (closet_tx, closet_rx) = channel();
    if !opt.headless { wear::load_saved(closet_tx.clone()); }
    let mut app = App {
        inbox, running: true, crew: Crew::new(seed), props: Props::default(), screen, settings, settings_path,
        max_buddies: opt.max_buddies.clamp(1, MAX_BUDDIES), base_max: opt.max_buddies.clamp(1, MAX_BUDDIES),
        closet: Wardrobe::default(), closet_tx, closet_rx, scale, ui,
        bounds: (bounds.0 as f64, bounds.1 as f64, bounds.2 as f64, bounds.3 as f64), platforms: Vec::new(), next_id: 0, rng: Rng::new(seed ^ 0xABCD),
        now: 0.0, next_platforms: 0.0, next_keys: 0.0, key_presses: 0, last_tick: 0, last_key_cursor: (0, 0), press: None, dragging: false,
        pet_rub: 0.0, last_cursor: (0.0, 0.0), hidden: false, user_hidden: false, wheel: None, studio: None, voice: None, save_at: None, marquee: None, before_menu: HWND::default(), paused: 0.0, work_ms: VecDeque::with_capacity(300), gap_ms: VecDeque::with_capacity(300), perf_count: 0,
    };
    app.apply_settings();
    let _ = app.spawn(None, None);
    unsafe { timeBeginPeriod(1); }
    let code = main_loop(&mut app, opt.bench, opt.bench_menu);
    unsafe { timeEndPeriod(1); }
    app.flush_settings(true);
    app.studio = None;
    if let Some(s) = &app.screen { win::remove_tray(s.overlay); }
    code
}

fn main_loop(app: &mut App, bench: Option<Vec<usize>>, bench_menu: bool) -> i32 {
    let started = Instant::now();
    let (mut sim, mut last) = (started.elapsed().as_secs_f64(), started);
    let mut acc = 0.0;
    let mut next_busy = 0.0;
    let mut bench = bench.map(|counts| Bench { counts, stage: 0, began: 0.0, with_menu: bench_menu, start_x: Vec::new() });
    let mut msg = MSG::default();
    while app.running {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT { app.running = false; }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if DISPLAY_CHANGED.with(|d| d.replace(false)) { app.display_changed(); }
        app.shell_events();
        app.closet_events();
        app.studio_frame();
        let frame_start = Instant::now();
        let t = frame_start.duration_since(started).as_secs_f64();
        if app.screen.is_some() && t >= next_busy {
            next_busy = t + 1.0;
            let own = app.screen.as_ref().map_or([0, 0, 0, 0], |s| [s.overlay.0 as isize, s.grab.0 as isize, s.menu.0 as isize, s.select.0 as isize]);
            let hide = app.user_hidden || (bench.is_none() && win::busy_fullscreen(own));
            app.set_hidden(hide);
        }
        if app.hidden {              // a fullscreen game is running: use nothing, but still answer Claude
            app.drain();
            last = frame_start;
            acc = 0.0;
            std::thread::sleep(std::time::Duration::from_millis(if app.studio.is_some() { 16 } else { 100 }));
            continue;
        }
        let gap = frame_start.duration_since(last).as_secs_f64();
        last = frame_start;
        acc += gap.min(MAX_STEPS as f64 * STEP);
        let work = Instant::now();
        let mut steps = 0;
        while acc >= STEP && steps < MAX_STEPS {
            sim += STEP;
            app.step(sim);
            acc -= STEP;
            steps += 1;
        }
        if steps == MAX_STEPS { acc = 0.0; }
        if app.perf_count != app.crew.brains.len() {   // numbers for one crew size never mix with another
            app.perf_count = app.crew.brains.len();
            app.work_ms.clear();
            app.gap_ms.clear();
        }
        let mut presented = false;
        let blend = if app.frozen() { 1.0 } else { acc / STEP };   // no shimmer between two held frames
        let world = sim - app.paused;
        if let Some(s) = &mut app.screen {
            let drawn = s.scene.draw(&mut s.gpu, &app.crew.brains, &app.props, app.wheel.as_ref(), app.marquee.as_ref(), world, blend);
            match drawn {
                Ok(p) => presented = p,
                Err(e) => {
                    eprintln!("drawing failed: {e}");
                    return 1;
                }
            }
        }
        let spent = app.last_work(work);
        if app.work_ms.len() >= 300 { app.work_ms.pop_front(); app.gap_ms.pop_front(); }
        app.work_ms.push_back(spent);
        app.gap_ms.push_back(gap as f32 * 1000.0);
        if let Some(b) = &mut bench { if b.tick(app, t) { app.running = false; } }
        if !presented { std::thread::sleep(std::time::Duration::from_millis(if app.screen.is_some() { 12 } else { 16 })); }
    }
    0
}

/// `--bench 10,50,100,200`: fills the crew, lets it settle, measures, and prints one line per size.
struct Bench { counts: Vec<usize>, stage: usize, began: f64, with_menu: bool, start_x: Vec<f64> }

impl Bench {
    fn tick(&mut self, app: &mut App, t: f64) -> bool {
        let Some(&want) = self.counts.get(self.stage) else { return true };
        if self.with_menu && app.wheel.is_none() {   // drawn and animated, but it never takes the keyboard
            let flags = app.menu_flags();
            let mut w = Wheel::new(app.crew.brains[0].id, 960.0 * app.ui as f32, 500.0 * app.ui as f32, app.ui as f32, flags);
            w.open = Some(2);
            app.wheel = Some(w);
        }
        if app.crew.brains.len() < want {
            for _ in 0..(want - app.crew.brains.len()).min(want / 40 + 5) { let _ = app.spawn(Some(0), None); }
            self.began = t;
            self.start_x.clear();
            return false;
        }
        if self.start_x.is_empty() && t - self.began >= 3.0 { self.start_x = app.crew.brains.iter().map(|b| b.x).collect(); }
        if t - self.began < 8.0 { return false; }
        let moved = app.crew.brains.iter().zip(&self.start_x).filter(|(b, x)| (b.x - **x).abs() > 0.5).count();
        let on_screen = app.crew.brains.iter().filter(|b| b.mode != Mode::Fall).count();
        println!("{want:>4} buddies ({on_screen} settled, {moved} moved): {}", app.perf());
        self.stage += 1;
        self.began = t;
        self.start_x.clear();
        self.stage >= self.counts.len()
    }
}

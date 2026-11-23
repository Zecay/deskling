//! The Win32 calls for the world around the buddies: screen, taskbar, windows, cursor and input.
use std::ffi::c_void;
use std::mem::size_of;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        let (l, t) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        (l, t, l + GetSystemMetrics(SM_CXVIRTUALSCREEN), t + GetSystemMetrics(SM_CYVIRTUALSCREEN))
    }
}

pub fn work_bottom() -> i32 {
    let mut rc = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut rc as *mut _ as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    rc.bottom
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe { let _ = GetCursorPos(&mut p); }
    (p.x, p.y)
}

pub fn left_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

pub fn any_button_down() -> bool {
    unsafe { ((GetAsyncKeyState(VK_LBUTTON.0 as i32) | GetAsyncKeyState(VK_RBUTTON.0 as i32)) as u16 & 0x8000) != 0 }
}

/// (tick of the last input anywhere, seconds since then). Windows only says *that* input happened,
/// never what it was, so this can't see which keys are pressed.
pub fn last_input() -> (u32, f64) {
    let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if !GetLastInputInfo(&mut info).as_bool() { return (0, 0.0); }
        (info.dwTime, GetTickCount().wrapping_sub(info.dwTime) as f64 / 1000.0)
    }
}

/// A fullscreen game, video or presentation is running, so the buddies should get out of the way.
/// (QUNS_BUSY isn't used: our own screen-sized topmost overlay would count as that.)
pub fn busy_fullscreen(own: [isize; 4]) -> bool {
    unsafe {
        if SHQueryUserNotificationState().is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE) { return true; }
        let hwnd = GetForegroundWindow();
        // A maximized window's rectangle overshoots the screen a little, but it isn't fullscreen.
        if hwnd.0.is_null() || own.contains(&(hwnd.0 as isize)) || IsZoomed(hwnd).as_bool() { return false; }
        let class = class_name(hwnd);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") { return false; }
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() { return false; }
        let mut rc = RECT::default();
        if GetWindowRect(hwnd, &mut rc).is_err() { return false; }
        let m = info.rcMonitor;
        rc.left <= m.left && rc.top <= m.top && rc.right >= m.right && rc.bottom >= m.bottom
    }
}

pub fn window_at(x: i32, y: i32) -> isize {
    unsafe {
        let hwnd = WindowFromPoint(POINT { x, y });
        if hwnd.0.is_null() { return 0; }
        let root = GetAncestor(hwnd, GA_ROOT);
        (if root.0.is_null() { hwnd } else { root }).0 as isize
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn frame_rect(hwnd: HWND) -> RECT {
    let mut rc = RECT::default();
    unsafe {
        if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rc as *mut _ as *mut c_void, size_of::<RECT>() as u32).is_err() {
            let _ = GetWindowRect(hwnd, &mut rc);
        }
    }
    rc
}

fn cloaked(hwnd: HWND) -> bool {
    let mut value = 0i32;
    unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut value as *mut _ as *mut c_void, 4).is_ok() && value != 0
    }
}

fn is_app_window(hwnd: HWND) -> bool {
    unsafe { GetWindowTextLengthW(hwnd) > 0 && (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0) == 0 }
}

pub struct Edge { pub hwnd: isize, pub left: i32, pub top: i32, pub right: i32, pub taskbar: bool }

struct Search { own: [isize; 4], min_top: i32, max_bottom: i32, found: Vec<Edge> }

unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> windows::Win32::Foundation::BOOL {
    let s = &mut *(lparam.0 as *mut Search);
    if s.own.contains(&(hwnd.0 as isize)) || !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() { return TRUE; }
    let rc = frame_rect(hwnd);
    if rc.top < s.min_top || rc.top > s.max_bottom { return TRUE; }
    let class = class_name(hwnd);
    let taskbar = class == "Shell_TrayWnd" || class == "Shell_SecondaryTrayWnd";
    if !taskbar {
        const SKIP: [&str; 5] = ["Progman", "WorkerW", "Windows.UI.Core.CoreWindow", "NotifyIconOverflowWindow", "TopLevelWindowForOverflowXamlIsland"];
        if SKIP.contains(&class.as_str()) || !is_app_window(hwnd) || cloaked(hwnd) { return TRUE; }
        if rc.right - rc.left < 160 || rc.bottom - rc.top < 60 { return TRUE; }
    }
    s.found.push(Edge { hwnd: hwnd.0 as isize, left: rc.left, top: rc.top, right: rc.right, taskbar });
    TRUE
}

/// Top edges a buddy can stand on, for taskbars and app windows.
pub fn window_edges(own: [isize; 4], min_top: i32, max_bottom: i32) -> Vec<Edge> {
    let mut search = Search { own, min_top, max_bottom, found: Vec::new() };
    unsafe { let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut _ as isize)); }
    search.found
}

/// The usable area (no taskbar) of the monitor under a point.
pub fn work_area_at(x: i32, y: i32) -> (i32, i32, i32, i32) {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST), &mut info);
    }
    let (r, m) = (info.rcWork, info.rcMonitor);
    let mut area = (r.left, r.top, r.right, r.bottom);
    // An auto-hidden taskbar isn't part of the "work area", but it still slides over whatever is there.
    for class in [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")] {
        let mut after = HWND::default();
        while let Ok(bar) = unsafe { FindWindowExW(None, after, class, None) } {
            if bar.0.is_null() { break; }
            after = bar;
            let mut b = RECT::default();
            if unsafe { GetWindowRect(bar, &mut b) }.is_err() { continue; }
            let (bw, bh) = (b.right - b.left, b.bottom - b.top);
            if bw >= bh {
                let mid = (b.left + b.right) / 2;
                if mid < m.left || mid >= m.right { continue; }
                if (b.bottom - m.bottom).abs() <= bh { area.3 = area.3.min(m.bottom - bh); }
                else if (b.top - m.top).abs() <= bh { area.1 = area.1.max(m.top + bh); }
            } else {
                let mid = (b.top + b.bottom) / 2;
                if mid < m.top || mid >= m.bottom { continue; }
                if (b.right - m.right).abs() <= bw { area.2 = area.2.min(m.right - bw); }
                else if (b.left - m.left).abs() <= bw { area.0 = area.0.max(m.left + bw); }
            }
        }
    }
    area
}

pub const TRAY_MESSAGE: u32 = WM_APP + 1;

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() }
}

pub fn add_tray(hwnd: HWND) {
    let mut data = tray_data(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    for (slot, ch) in data.szTip.iter_mut().zip("Deskling: click to show or hide, right-click for menu".encode_utf16()) { *slot = ch; }
    unsafe {
        // The Clawd icon built into the .exe, at the size this display wants.
        if let Ok(module) = GetModuleHandleW(None) {
            let (w, h) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
            if let Ok(icon) = LoadImageW(module, PCWSTR(1 as *const u16), IMAGE_ICON, w, h, LR_DEFAULTCOLOR) { data.hIcon = HICON(icon.0); }
        }
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
    }
}

pub fn remove_tray(hwnd: HWND) {
    unsafe { let _ = Shell_NotifyIconW(NIM_DELETE, &tray_data(hwnd)); }
}

/// A plain Windows message box, for problems when there's no console to print to.
pub fn alert(text: &str) {
    use windows::core::HSTRING;
    unsafe { let _ = MessageBoxW(None, &HSTRING::from(text), w!("Deskling"), MB_OK | MB_ICONWARNING); }
}

/// Ctrl+Alt+Shift, because plain Ctrl+Alt is AltGr on many keyboards and would eat typed letters.
pub const HOTKEYS: [(i32, u32); 6] = [(6, b'R' as u32), (1, b'B' as u32), (2, b'H' as u32), (3, b'S' as u32), (4, b'C' as u32), (5, b'G' as u32)];

pub fn add_hotkeys(hwnd: HWND) {
    for (id, key) in HOTKEYS {
        unsafe { let _ = RegisterHotKey(hwnd, id, MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, key); }
    }
}

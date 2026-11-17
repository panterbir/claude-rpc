//! System tray icon + right-click menu (raw Win32, no crates).
//! Runs on the main thread; the Discord loop runs on a worker thread.
#![allow(non_snake_case, non_camel_case_types)]

use crate::config;
use crate::win::{self, wide, HANDLE};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub static PAUSED: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static NOTIFY: AtomicBool = AtomicBool::new(true);

const WM_APP_TRAY: u32 = 0x0401; // WM_USER + 1
const WM_RBUTTONUP: u32 = 0x0205;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_DESTROY: u32 = 0x0002;
const NIM_ADD: u32 = 0;
const NIM_DELETE: u32 = 2;
const NIF_MESSAGE: u32 = 1;
const NIF_ICON: u32 = 2;
const NIF_TIP: u32 = 4;
const NIF_INFO: u32 = 0x10;
const NIIF_INFO: u32 = 1;
const MF_GRAYED: u32 = 1;
const MF_SEPARATOR: u32 = 0x800;
const TPM_RETURNCMD: u32 = 0x100;
const TPM_RIGHTBUTTON: u32 = 2;
const TPM_BOTTOMALIGN: u32 = 0x20;

const ID_PAUSE: usize = 2;
const ID_CONFIG: usize = 3;
const ID_FOLDER: usize = 4;
const ID_QUIT: usize = 5;

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HANDLE,
    message: u32,
    wParam: usize,
    lParam: isize,
    time: u32,
    pt: POINT,
    lPrivate: u32,
}

type WNDPROC = unsafe extern "system" fn(HANDLE, u32, usize, isize) -> isize;

#[repr(C)]
struct WNDCLASSEXW {
    cbSize: u32,
    style: u32,
    lpfnWndProc: WNDPROC,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: HANDLE,
    hIcon: HANDLE,
    hCursor: HANDLE,
    hbrBackground: HANDLE,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
    hIconSm: HANDLE,
}

#[repr(C)]
struct NOTIFYICONDATAW {
    cbSize: u32,
    hWnd: HANDLE,
    uID: u32,
    uFlags: u32,
    uCallbackMessage: u32,
    hIcon: HANDLE,
    szTip: [u16; 128],
    dwState: u32,
    dwStateMask: u32,
    szInfo: [u16; 256],
    uVersion: u32,
    szInfoTitle: [u16; 64],
    dwInfoFlags: u32,
    guidItem: [u8; 16],
    hBalloonIcon: HANDLE,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(c: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HANDLE,
        menu: HANDLE,
        inst: HANDLE,
        param: *mut c_void,
    ) -> HANDLE;
    fn DefWindowProcW(h: HANDLE, m: u32, w: usize, l: isize) -> isize;
    fn GetMessageW(msg: *mut MSG, h: HANDLE, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> isize;
    fn PostQuitMessage(code: i32);
    fn PostMessageW(h: HANDLE, m: u32, w: usize, l: isize) -> i32;
    fn CreatePopupMenu() -> HANDLE;
    fn AppendMenuW(menu: HANDLE, flags: u32, id: usize, text: *const u16) -> i32;
    fn TrackPopupMenu(menu: HANDLE, flags: u32, x: i32, y: i32, reserved: i32, h: HANDLE, rc: *const c_void) -> i32;
    fn DestroyMenu(menu: HANDLE) -> i32;
    fn GetCursorPos(p: *mut POINT) -> i32;
    fn SetForegroundWindow(h: HANDLE) -> i32;
    fn CreateIcon(inst: HANDLE, w: i32, h: i32, planes: u8, bpp: u8, and_bits: *const u8, xor_bits: *const u8) -> HANDLE;
    fn RegisterWindowMessageW(s: *const u16) -> u32;
    pub fn MessageBoxW(h: HANDLE, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(msg: u32, data: *const NOTIFYICONDATAW) -> i32;
    fn ShellExecuteW(h: HANDLE, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> HANDLE;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HANDLE;
}

/// Modal info box (used for fatal setup problems; the exe has no console).
pub fn message_box(text: &str, caption: &str) {
    unsafe {
        MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide(caption).as_ptr(), 0x40); // MB_ICONINFORMATION
    }
}

fn copy_wide<const N: usize>(dst: &mut [u16; N], s: &str) {
    for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
        dst[i] = c;
    }
}

/// 32x32 orange 12-ray star, drawn procedurally (no .ico resource needed).
fn make_icon(inst: HANDLE) -> HANDLE {
    const S: usize = 32;
    let mut xor = vec![0u8; S * S * 4];
    let mut and = vec![0u8; S * S / 8];
    let c = (S as f32 - 1.0) / 2.0;
    for y in 0..S {
        for x in 0..S {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let r = (dx * dx + dy * dy).sqrt();
            let th = dy.atan2(dx);
            let edge = 5.0 + 10.5 * (6.0 * th).cos().abs().powf(2.2);
            let inside = r <= edge;
            let i = (y * S + x) * 4;
            if inside {
                xor[i..i + 4].copy_from_slice(&[0x57, 0x77, 0xD9, 0xFF]); // BGRA #D97757
            } else {
                and[y * 4 + x / 8] |= 0x80 >> (x % 8); // transparent
            }
        }
    }
    unsafe { CreateIcon(inst, S as i32, S as i32, 1, 32, and.as_ptr(), xor.as_ptr()) }
}

fn nid(hwnd: HANDLE, icon: HANDLE) -> NOTIFYICONDATAW {
    let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = 1;
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    d.uCallbackMessage = WM_APP_TRAY;
    d.hIcon = icon;
    copy_wide(&mut d.szTip, "Claude RPC");
    d
}

static mut ICON: HANDLE = std::ptr::null_mut();

unsafe fn add_icon(hwnd: HANDLE, balloon: bool) {
    let mut d = nid(hwnd, ICON);
    if balloon {
        d.uFlags |= NIF_INFO;
        d.dwInfoFlags = NIIF_INFO;
        copy_wide(&mut d.szInfoTitle, "Claude RPC çalışıyor");
        copy_wide(&mut d.szInfo, "Discord durumun arka planda güncelleniyor. Menü için sağ alttaki simgeye sağ tıkla.");
    }
    Shell_NotifyIconW(NIM_ADD, &d);
}

fn status_line() -> String {
    if PAUSED.load(Ordering::Relaxed) {
        return "Duraklatıldı (presence gizli)".into();
    }
    let t = std::fs::read_to_string(config::data_dir().join("status.json")).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&t).unwrap_or(serde_json::Value::Null);
    if v["connected"].as_bool() != Some(true) {
        return "Discord'a bağlı değil (Discord açık mı?)".into();
    }
    let d = v["details"].as_str().unwrap_or("");
    if d.is_empty() {
        "Discord'a bağlı · gösterilecek oturum yok".into()
    } else {
        format!("Bağlı · {} / {}", d, v["state"].as_str().unwrap_or(""))
    }
}

fn open(file: &str, params: Option<&str>) {
    unsafe {
        let p = params.map(wide);
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(file).as_ptr(),
            p.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            std::ptr::null(),
            1,
        );
    }
}

fn open_config() {
    let dir = config::data_dir();
    let path = dir.join("config.toml");
    if !path.exists() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(&path, include_str!("../config.example.toml"));
    }
    open("notepad.exe", Some(&format!("\"{}\"", path.display())));
}

unsafe fn show_menu(hwnd: HANDLE) {
    let menu = CreatePopupMenu();
    let status = wide(&status_line());
    AppendMenuW(menu, MF_GRAYED, 1, status.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    let paused = PAUSED.load(Ordering::Relaxed);
    AppendMenuW(menu, 0, ID_PAUSE, wide(if paused { "Presence'ı devam ettir" } else { "Presence'ı duraklat" }).as_ptr());
    AppendMenuW(menu, 0, ID_CONFIG, wide("Ayar dosyasını aç (config.toml)").as_ptr());
    AppendMenuW(menu, 0, ID_FOLDER, wide("Veri klasörünü aç").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(menu, 0, ID_QUIT, wide("Çıkış (Claude RPC'yi kapat)").as_ptr());
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd); // required so the menu closes when clicking elsewhere
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, std::ptr::null()) as usize;
    PostMessageW(hwnd, 0, 0, 0);
    DestroyMenu(menu);
    match cmd {
        ID_PAUSE => {
            PAUSED.fetch_xor(true, Ordering::Relaxed);
        }
        ID_CONFIG => open_config(),
        ID_FOLDER => open(&config::data_dir().to_string_lossy(), None),
        ID_QUIT => {
            Shell_NotifyIconW(NIM_DELETE, &nid(hwnd, ICON));
            PostQuitMessage(0);
        }
        _ => {}
    }
}

unsafe extern "system" fn wnd_proc(h: HANDLE, m: u32, w: usize, l: isize) -> isize {
    match m {
        WM_APP_TRAY if l as u32 == WM_RBUTTONUP || l as u32 == WM_LBUTTONUP => {
            show_menu(h);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            add_icon(h, false); // Explorer restarted: put the icon back
            0
        }
        _ => DefWindowProcW(h, m, w, l),
    }
}

/// Blocks until the user picks "Çıkış". Returns false if the tray could not be created.
pub fn run(notify_on_start: bool) -> bool {
    NOTIFY.store(notify_on_start, Ordering::Relaxed);
    unsafe {
        let inst = GetModuleHandleW(std::ptr::null());
        ICON = make_icon(inst);
        TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);
        let class = wide("ClaudeRpcTrayWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: wnd_proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: ICON,
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: ICON,
        };
        if RegisterClassExW(&wc) == 0 {
            return false;
        }
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide("Claude RPC").as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), inst, std::ptr::null_mut());
        if hwnd.is_null() {
            return false;
        }
        add_icon(hwnd, NOTIFY.load(Ordering::Relaxed));
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    let _ = win::pid_alive; // keep win linked for cfg combinations
    true
}

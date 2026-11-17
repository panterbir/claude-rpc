//! Minimal Win32 FFI (no windows-sys crate -> smaller binary, less RAM).
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::os::windows::io::IntoRawHandle;

pub type HANDLE = *mut c_void;
pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

#[repr(C)]
pub struct PROCESSENTRY32W {
    pub dwSize: u32,
    pub cntUsage: u32,
    pub th32ProcessID: u32,
    pub th32DefaultHeapID: usize,
    pub th32ModuleID: u32,
    pub cntThreads: u32,
    pub th32ParentProcessID: u32,
    pub pcPriClassBase: i32,
    pub dwFlags: u32,
    pub szExeFile: [u16; 260],
}

type WNDENUMPROC = unsafe extern "system" fn(HANDLE, isize) -> i32;

#[link(name = "kernel32")]
extern "system" {
    pub fn AttachConsole(pid: u32) -> i32;
    pub fn GetStdHandle(n: u32) -> HANDLE;
    pub fn SetStdHandle(n: u32, h: HANDLE) -> i32;
    pub fn CreateMutexW(attr: *const c_void, initial: i32, name: *const u16) -> HANDLE;
    pub fn GetLastError() -> u32;
    pub fn PeekNamedPipe(
        h: HANDLE,
        buf: *mut c_void,
        size: u32,
        read: *mut u32,
        avail: *mut u32,
        left: *mut u32,
    ) -> i32;
    pub fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> HANDLE;
    pub fn Process32FirstW(h: HANDLE, e: *mut PROCESSENTRY32W) -> i32;
    pub fn Process32NextW(h: HANDLE, e: *mut PROCESSENTRY32W) -> i32;
    pub fn CloseHandle(h: HANDLE) -> i32;
    pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> HANDLE;
    pub fn QueryFullProcessImageNameW(h: HANDLE, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    pub fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> i32;
    pub fn TerminateProcess(h: HANDLE, code: u32) -> i32;
    pub fn SetProcessWorkingSetSize(h: HANDLE, min: usize, max: usize) -> i32;
    pub fn GetCurrentProcess() -> HANDLE;
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(cb: WNDENUMPROC, lparam: isize) -> i32;
    fn IsWindowVisible(h: HANDLE) -> i32;
    fn GetWindowTextLengthW(h: HANDLE) -> i32;
    fn GetWindowThreadProcessId(h: HANDLE, pid: *mut u32) -> u32;
    fn GetForegroundWindow() -> HANDLE;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        hkey: isize,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        sam: u32,
        sa: *const c_void,
        result: *mut isize,
        disp: *mut u32,
    ) -> i32;
    fn RegOpenKeyExW(hkey: isize, sub: *const u16, opts: u32, sam: u32, result: *mut isize) -> i32;
    fn RegSetValueExW(hkey: isize, name: *const u16, reserved: u32, typ: u32, data: *const u8, len: u32) -> i32;
    fn RegDeleteValueW(hkey: isize, name: *const u16) -> i32;
    fn RegQueryValueExW(
        hkey: isize,
        name: *const u16,
        reserved: *mut u32,
        typ: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> i32;
    fn RegCloseKey(hkey: isize) -> i32;
}

const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const KEY_SET_VALUE: u32 = 0x0002;
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_WRITE: u32 = 0x20006;
const REG_SZ: u32 = 1;
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Console subcommands: the binary is a GUI-subsystem exe, so attach to the parent
/// console (if any) and wire stdout/stderr to it unless they are already redirected.
pub fn attach_console() {
    unsafe {
        if AttachConsole(0xFFFF_FFFF) == 0 {
            return; // no parent console (double click, redirected, ...)
        }
        for (std_id, _name) in [(0xFFFF_FFF5u32, "out"), (0xFFFF_FFF4u32, "err")] {
            // STD_OUTPUT_HANDLE = -11, STD_ERROR_HANDLE = -12
            let h = GetStdHandle(std_id);
            if h.is_null() || h == INVALID_HANDLE_VALUE {
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
                    SetStdHandle(std_id, f.into_raw_handle() as HANDLE);
                }
            }
        }
    }
}

/// Returns true if this call created the mutex (i.e. we are the only instance).
pub fn single_instance(name: &str) -> bool {
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, wide(name).as_ptr());
        if h.is_null() {
            return true; // cannot tell; do not block startup
        }
        GetLastError() != 183 // ERROR_ALREADY_EXISTS (handle intentionally leaked)
    }
}

pub fn pipe_available(h: HANDLE) -> Result<u32, ()> {
    let mut avail = 0u32;
    let ok = unsafe { PeekNamedPipe(h, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut()) };
    if ok == 0 {
        Err(())
    } else {
        Ok(avail)
    }
}

pub fn pid_alive(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(0x1000, 0, pid); // PROCESS_QUERY_LIMITED_INFORMATION
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == 259 // STILL_ACTIVE
    }
}

pub fn kill_pid(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(0x0001, 0, pid); // PROCESS_TERMINATE
        if h.is_null() {
            return false;
        }
        let ok = TerminateProcess(h, 0);
        CloseHandle(h);
        ok != 0
    }
}

/// Trim this process's working set (releases pages back to the OS).
pub fn trim_working_set() {
    unsafe {
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

fn image_path(pid: u32, buf: &mut [u16; 520]) -> Option<String> {
    unsafe {
        let h = OpenProcess(0x1000, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// PIDs of Claude *Desktop* processes (Claude.exe that is not the bundled Claude Code CLI).
pub fn claude_desktop_pids(out: &mut Vec<u32>) {
    out.clear();
    unsafe {
        let snap = CreateToolhelp32Snapshot(0x2, 0); // TH32CS_SNAPPROCESS
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut pbuf = [0u16; 520];
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
            let name = String::from_utf16_lossy(&e.szExeFile[..n]);
            if name.eq_ignore_ascii_case("claude.exe") {
                if let Some(p) = image_path(e.th32ProcessID, &mut pbuf) {
                    let l = p.to_ascii_lowercase();
                    if !l.contains("claude-code") && !l.contains("\\.local\\bin") {
                        out.push(e.th32ProcessID);
                    }
                }
            }
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
}

struct EnumCtx<'a> {
    pids: &'a [u32],
    found: bool,
}

unsafe extern "system" fn enum_cb(h: HANDLE, lparam: isize) -> i32 {
    let ctx = &mut *(lparam as *mut EnumCtx);
    if IsWindowVisible(h) != 0 && GetWindowTextLengthW(h) > 0 {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if ctx.pids.contains(&pid) {
            ctx.found = true;
            return 0;
        }
    }
    1
}

/// True if any visible, titled top-level window belongs to one of `pids`.
pub fn has_visible_window(pids: &[u32]) -> bool {
    let mut ctx = EnumCtx { pids, found: false };
    unsafe {
        EnumWindows(enum_cb, &mut ctx as *mut EnumCtx as isize);
    }
    ctx.found
}

pub fn foreground_pid() -> u32 {
    unsafe {
        let h = GetForegroundWindow();
        if h.is_null() {
            return 0;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        pid
    }
}

/// HKCU\...\Run helpers.
pub fn run_key_set(name: &str, value: &str) -> bool {
    unsafe {
        let mut key = 0isize;
        let mut disp = 0u32;
        if RegCreateKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, std::ptr::null(), 0, KEY_WRITE, std::ptr::null(), &mut key, &mut disp) != 0 {
            return false;
        }
        let v = wide(value);
        let rc = RegSetValueExW(key, wide(name).as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32);
        RegCloseKey(key);
        rc == 0
    }
}

pub fn run_key_delete(name: &str) -> bool {
    unsafe {
        let mut key = 0isize;
        if RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, KEY_SET_VALUE, &mut key) != 0 {
            return false;
        }
        let rc = RegDeleteValueW(key, wide(name).as_ptr());
        RegCloseKey(key);
        rc == 0
    }
}

pub fn run_key_get(name: &str) -> Option<String> {
    unsafe {
        let mut key = 0isize;
        if RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, KEY_QUERY_VALUE, &mut key) != 0 {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let mut typ = 0u32;
        let rc = RegQueryValueExW(key, wide(name).as_ptr(), std::ptr::null_mut(), &mut typ, buf.as_mut_ptr() as *mut u8, &mut len);
        RegCloseKey(key);
        if rc != 0 {
            return None;
        }
        let n = (len as usize / 2).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..n]);
        Some(s.trim_end_matches('\0').to_string())
    }
}

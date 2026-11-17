//! Background daemon: single thread, poll loop, no async.

use crate::claude;
use crate::config::{self, Config};
use crate::discord::{Activity, Conn, Incoming};
use crate::git;
use crate::state::{self, Phase, State};
use crate::tray;
use crate::win;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

const TICK: Duration = Duration::from_secs(2);
const SLOW_EVERY: Duration = Duration::from_secs(10); // process scan / projects scan
const MIN_SEND_GAP: Duration = Duration::from_secs(15); // Discord rate limit
const BACKOFF_MIN: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
const HOOK_GRACE_MS: u64 = 30_000;

struct Daemon {
    cfg: Config,
    dir: PathBuf,
    state_path: PathBuf,
    state: State,
    state_mtime: Option<SystemTime>,
    text: String, // reusable file buffer
    // fallbacks (refreshed every SLOW_EVERY)
    last_slow: Option<Instant>,
    pids: Vec<u32>,
    chat_open: bool,
    chat_start: u64,
    chat_last_active: u64,
    proj_name: String, // newest projects/* folder
    proj_mtime: u64,
    proj_cwd: String,
    proj_cwd_for: String, // folder name proj_cwd was decoded from
    fb_key: String,
    fb_start: u64,
    // discord
    conn: Option<Conn>,
    backoff: Duration,
    next_attempt: Instant,
    sent: Option<Option<Activity>>, // None = nothing sent yet on this connection
    last_send: Option<Instant>,
    desired: Option<Activity>,
    branch: String,
    scratch: String,
    // status.json
    status_key: String,
    last_error: String,
}

/// `interactive` = started by double click (no arguments): tell the user if it is already running.
pub fn run(interactive: bool) -> i32 {
    let dir = config::data_dir();
    let _ = fs::create_dir_all(&dir);
    let cfg = config::load();
    if cfg.client_id.is_empty() {
        write_status(&dir, false, "", "", config::MISSING_ID_MSG);
        log(&dir, config::MISSING_ID_MSG);
        tray::message_box(config::MISSING_ID_MSG, "Claude RPC");
        return 1;
    }
    if !win::single_instance("Local\\claude-rpc-daemon") {
        if interactive {
            tray::message_box("Claude RPC zaten çalışıyor. Sağ alttaki tepsideki (^) turuncu yıldız simgesine sağ tıklayarak yönetebilirsin.", "Claude RPC");
        }
        return 0;
    }
    let notify = cfg.notify_on_start;
    let mut d = Daemon {
        state_path: state::state_path(&dir),
        dir,
        cfg,
        state: State::default(),
        state_mtime: None,
        text: String::with_capacity(4096),
        last_slow: None,
        pids: Vec::with_capacity(32),
        chat_open: false,
        chat_start: 0,
        chat_last_active: 0,
        proj_name: String::new(),
        proj_mtime: 0,
        proj_cwd: String::new(),
        proj_cwd_for: String::new(),
        fb_key: String::new(),
        fb_start: 0,
        conn: None,
        backoff: BACKOFF_MIN,
        next_attempt: Instant::now(),
        sent: None,
        last_send: None,
        desired: None,
        branch: String::with_capacity(64),
        scratch: String::with_capacity(256),
        status_key: String::new(),
        last_error: String::new(),
    };
    // Discord loop on a worker thread; the tray (message pump) owns the main thread.
    std::thread::spawn(move || d.main_loop());
    if !tray::run(notify) {
        log(&config::data_dir(), "tepsi simgesi olusturulamadi");
        loop {
            std::thread::sleep(Duration::from_secs(3600)); // keep serving without a tray
        }
    }
    0
}

impl Daemon {
    fn main_loop(&mut self) {
        let mut tick: u32 = 0;
        loop {
            let now = state::now_ms();
            self.reload_state();
            self.slow_refresh(now);
            self.compute(now);
            self.drive_discord();
            self.write_status_if_changed();
            tick = tick.wrapping_add(1);
            if tick % 30 == 1 {
                win::trim_working_set(); // every minute: hand freed pages back to the OS
            }
            std::thread::sleep(TICK);
        }
    }

    fn reload_state(&mut self) {
        let mtime = fs::metadata(&self.state_path).and_then(|m| m.modified()).ok();
        if mtime == self.state_mtime {
            return;
        }
        self.state_mtime = mtime;
        self.text.clear();
        match fs::File::open(&self.state_path).and_then(|mut f| std::io::Read::read_to_string(&mut f, &mut self.text)) {
            Ok(_) => self.state = State::from_json(&self.text),
            Err(_) => self.state = State::default(),
        }
    }

    fn slow_refresh(&mut self, now: u64) {
        if self.last_slow.is_some_and(|t| t.elapsed() < SLOW_EVERY) {
            return;
        }
        self.last_slow = Some(Instant::now());
        let hooks_active = self.state.current(now, self.cfg.idle_secs * 1000, self.cfg.stale_secs * 1000).is_some();
        if hooks_active {
            self.chat_open = false;
            return; // hook data wins, skip the expensive scans
        }
        // projects fallback
        if let Some((name, mtime)) = claude::latest_project(&claude::claude_dir().join("projects")) {
            if name != self.proj_cwd_for {
                self.proj_cwd = claude::decode_project_dir(&name)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| claude::naive_decode(&name));
                self.proj_cwd_for = name.clone();
            }
            self.proj_name = name;
            self.proj_mtime = mtime;
        } else {
            self.proj_name.clear();
        }
        // Claude Desktop chat
        win::claude_desktop_pids(&mut self.pids);
        let open = !self.pids.is_empty() && win::has_visible_window(&self.pids);
        if open && !self.chat_open {
            self.chat_start = now;
            self.chat_last_active = now;
        }
        self.chat_open = open;
    }

    /// Fill `self.desired` from the best available source.
    fn compute(&mut self, now: u64) {
        let idle_ms = self.cfg.idle_secs * 1000;
        let stale_ms = self.cfg.stale_secs * 1000;
        let mut a = self.desired.take().unwrap_or_default();
        a.details.clear();
        a.state.clear();
        a.small_image.clear();
        a.small_text.clear();
        a.large_text.clear();
        a.large_image.clear();
        a.start_secs = None;

        // 1) hooks
        let mut source: Option<(String, Option<u64>, Phase)> = None; // (cwd, start, phase)
        if let Some((s, ph)) = self.state.current(now, idle_ms, stale_ms) {
            source = Some((s.cwd.clone(), Some(s.start), ph));
        } else if !self.proj_name.is_empty()
            && self.proj_mtime > self.state.last_event + HOOK_GRACE_MS
            && now.saturating_sub(self.proj_mtime) < stale_ms
        {
            // 2) ~/.claude/projects mtime fallback
            if self.fb_key != self.proj_cwd {
                self.fb_key.clone_from(&self.proj_cwd);
                self.fb_start = now;
            }
            let ph = if now.saturating_sub(self.proj_mtime) < idle_ms { Phase::Active } else { Phase::Idle };
            source = Some((self.proj_cwd.clone(), Some(self.fb_start), ph));
        }

        let have = if let Some((cwd, start, ph)) = source {
            a.details.push_str(state::project_name(&cwd));
            let text = if ph == Phase::Active { &self.cfg.active_text } else { &self.cfg.idle_text };
            if self.cfg.show_branch && git::current_branch(&cwd, &mut self.branch, &mut self.scratch) {
                a.state.push_str(&self.branch);
                a.state.push_str(" · ");
            }
            a.state.push_str(text);
            a.start_secs = start.map(|s| s / 1000);
            self.fill_assets(&mut a, ph);
            true
        } else if self.chat_open {
            // 3) Claude Desktop chat (no project info available)
            if win::foreground_pid() != 0 && self.pids.contains(&win::foreground_pid()) {
                self.chat_last_active = now;
            }
            let ph = if now.saturating_sub(self.chat_last_active) < idle_ms { Phase::Active } else { Phase::Idle };
            a.details.push_str(&self.cfg.chat_details);
            a.state.push_str(if ph == Phase::Active { &self.cfg.active_text } else { &self.cfg.idle_text });
            a.start_secs = Some(self.chat_start / 1000);
            self.fill_assets(&mut a, ph);
            true
        } else {
            false
        };
        self.desired = if have && !tray::PAUSED.load(std::sync::atomic::Ordering::Relaxed) { Some(a) } else { None };
    }

    fn fill_assets(&self, a: &mut Activity, ph: Phase) {
        a.large_image.push_str(&self.cfg.large_image);
        a.large_text.push_str(&self.cfg.large_text);
        let (img, txt) = if ph == Phase::Active {
            (&self.cfg.small_active, &self.cfg.active_text)
        } else {
            (&self.cfg.small_idle, &self.cfg.idle_text)
        };
        a.small_image.push_str(img);
        a.small_text.push_str(txt);
    }

    fn drive_discord(&mut self) {
        if self.conn.is_none() && Instant::now() >= self.next_attempt {
            match Conn::connect(&self.cfg.client_id) {
                Ok(c) => {
                    log(&self.dir, &format!("Discord baglandi: {}", c.pipe));
                    self.conn = Some(c);
                    self.backoff = BACKOFF_MIN;
                    self.sent = None;
                    self.last_error.clear();
                }
                Err(e) => {
                    self.last_error = e.to_string();
                    self.next_attempt = Instant::now() + self.backoff;
                    self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
                }
            }
        }
        let Some(conn) = self.conn.as_mut() else { return };

        match conn.poll() {
            Ok(Incoming::Closed) | Err(_) => {
                log(&self.dir, "Discord baglantisi kapandi, yeniden denenecek");
                self.drop_conn();
                return;
            }
            Ok(Incoming::Error(m)) => self.last_error = m,
            Ok(Incoming::Nothing) => {}
        }

        let due = self.last_send.is_none_or(|t| t.elapsed() >= MIN_SEND_GAP);
        // Nothing sent yet and nothing to show: no need to talk to Discord at all.
        if self.sent.is_none() && self.desired.is_none() {
            self.sent = Some(None);
            return;
        }
        let changed = self.sent.as_ref() != Some(&self.desired);
        if changed && due {
            let res = conn.set_activity(self.desired.as_ref());
            match res {
                Ok(()) => {
                    self.last_send = Some(Instant::now());
                    self.sent = Some(self.desired.clone());
                }
                Err(e) => {
                    self.last_error = e.to_string();
                    self.drop_conn();
                }
            }
        }
    }

    fn drop_conn(&mut self) {
        self.conn = None;
        self.sent = None;
        self.next_attempt = Instant::now() + self.backoff;
        self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
    }

    fn write_status_if_changed(&mut self) {
        self.scratch.clear();
        let connected = self.conn.is_some();
        let (d, s) = match &self.sent {
            Some(Some(a)) => (a.details.as_str(), a.state.as_str()),
            _ => ("", ""),
        };
        let key = format!("{connected}|{d}|{s}|{}", self.last_error);
        if key != self.status_key {
            write_status(&self.dir, connected, d, s, &self.last_error);
            self.status_key = key;
        }
    }
}

fn write_status(dir: &std::path::Path, connected: bool, details: &str, st: &str, err: &str) {
    let v = json!({
        "pid": std::process::id(),
        "connected": connected,
        "details": details,
        "state": st,
        "error": err,
        "updated": state::now_ms(),
    });
    let _ = state::write_atomic(&dir.join("status.json"), v.to_string().as_bytes());
}

fn log(dir: &std::path::Path, msg: &str) {
    let p = dir.join("daemon.log");
    if fs::metadata(&p).map(|m| m.len() > 64 * 1024).unwrap_or(false) {
        let _ = fs::remove_file(&p);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "[{}] {}", state::now_ms() / 1000, msg);
    }
}

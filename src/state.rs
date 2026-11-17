//! Shared state between `hook` (writer) and `daemon` (reader) + the session state machine.

use serde_json::{json, Value};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub start: u64, // unix ms
    pub last: u64,  // unix ms of last event
    pub working: bool,
}

#[derive(Default, Debug)]
pub struct State {
    pub last_event: u64,
    pub sessions: Vec<Session>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Active,
    Idle,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Last path component of cwd ("C:\a\b\" -> "b").
pub fn project_name(cwd: &str) -> &str {
    let t = cwd.trim_end_matches(['\\', '/']);
    t.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()).unwrap_or(cwd)
}

fn norm_event(ev: &str) -> String {
    ev.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

impl State {
    pub fn from_json(s: &str) -> State {
        let v: Value = serde_json::from_str(s.trim_start_matches('\u{feff}')).unwrap_or(Value::Null);
        let mut st = State { last_event: v["last_event"].as_u64().unwrap_or(0), sessions: Vec::new() };
        if let Some(arr) = v["sessions"].as_array() {
            for o in arr {
                st.sessions.push(Session {
                    id: o["id"].as_str().unwrap_or("").to_string(),
                    cwd: o["cwd"].as_str().unwrap_or("").to_string(),
                    start: o["start"].as_u64().unwrap_or(0),
                    last: o["last"].as_u64().unwrap_or(0),
                    working: o["working"].as_bool().unwrap_or(false),
                });
            }
        }
        st
    }

    pub fn to_json(&self) -> String {
        let sessions: Vec<Value> = self
            .sessions
            .iter()
            .map(|s| json!({"id": s.id, "cwd": s.cwd, "start": s.start, "last": s.last, "working": s.working}))
            .collect();
        json!({"last_event": self.last_event, "sessions": sessions}).to_string()
    }

    /// Apply one hook event. Event name may be `SessionStart` or `session-start` style.
    pub fn apply(&mut self, event: &str, id: &str, cwd: &str, now: u64) {
        let ev = norm_event(event);
        let id = if id.is_empty() { "unknown" } else { id };
        self.last_event = now;
        // drop sessions untouched for 24h so the file never grows
        self.sessions.retain(|s| now.saturating_sub(s.last) < 24 * 3600 * 1000);

        if ev == "sessionend" {
            self.sessions.retain(|s| s.id != id);
            return;
        }
        let idx = match self.sessions.iter().position(|s| s.id == id) {
            Some(i) => i,
            None => {
                self.sessions.push(Session { id: id.to_string(), cwd: cwd.to_string(), start: now, last: now, working: false });
                self.sessions.len() - 1
            }
        };
        let s = &mut self.sessions[idx];
        if !cwd.is_empty() && s.cwd != cwd {
            s.cwd = cwd.to_string();
            s.start = now; // project changed -> reset timer
        }
        s.last = now;
        match ev.as_str() {
            "userpromptsubmit" | "pretooluse" => s.working = true,
            "stop" | "sessionstart" => s.working = false,
            _ => {}
        }
    }

    /// Most recently active non-stale session and its phase.
    pub fn current(&self, now: u64, idle_ms: u64, stale_ms: u64) -> Option<(&Session, Phase)> {
        self.sessions
            .iter()
            .filter(|s| now.saturating_sub(s.last) < stale_ms)
            .max_by_key(|s| s.last)
            .map(|s| (s, phase(s, now, idle_ms)))
    }
}

pub fn phase(s: &Session, now: u64, idle_ms: u64) -> Phase {
    if s.working && now.saturating_sub(s.last) < idle_ms {
        Phase::Active
    } else {
        Phase::Idle
    }
}

/// Atomic write: temp file in the same dir + rename.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Cross-process lock for the read-modify-write in `hook` (best effort, never blocks long).
struct Lock(Option<std::path::PathBuf>);
impl Lock {
    fn acquire(path: &Path) -> Lock {
        for attempt in 0..2 {
            for _ in 0..100 {
                if fs::OpenOptions::new().write(true).create_new(true).open(path).is_ok() {
                    return Lock(Some(path.to_path_buf()));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            if attempt == 0 {
                // stale lock (crashed hook)? remove if older than 2s
                let old = fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .map(|e| e > Duration::from_secs(2))
                    .unwrap_or(false);
                if old {
                    let _ = fs::remove_file(path);
                }
            }
        }
        Lock(None) // proceed unlocked; a lost update is better than blocking Claude Code
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = fs::remove_file(p);
        }
    }
}

pub fn state_path(dir: &Path) -> std::path::PathBuf {
    dir.join("state.json")
}

pub fn hook_update(dir: &Path, event: &str, id: &str, cwd: &str, now: u64) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let _lock = Lock::acquire(&dir.join("state.lock"));
    let path = state_path(dir);
    let mut st = fs::read_to_string(&path).map(|s| State::from_json(&s)).unwrap_or_default();
    st.apply(event, id, cwd, now);
    write_atomic(&path, st.to_json().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn active_idle_stale_cycle() {
        let mut st = State::default();
        st.apply("SessionStart", "a", "C:\\p\\proj", 1_000);
        let (s, ph) = st.current(1_000, 5 * MIN, 60 * MIN).unwrap();
        assert_eq!(ph, Phase::Idle);
        assert_eq!(s.start, 1_000);

        st.apply("UserPromptSubmit", "a", "C:\\p\\proj", 2_000);
        assert_eq!(st.current(2_000, 5 * MIN, 60 * MIN).unwrap().1, Phase::Active);
        st.apply("PreToolUse", "a", "C:\\p\\proj", 3_000);
        assert_eq!(st.current(3_000, 5 * MIN, 60 * MIN).unwrap().1, Phase::Active);

        // auto idle after 5 minutes without events
        assert_eq!(st.current(3_000 + 5 * MIN, 5 * MIN, 60 * MIN).unwrap().1, Phase::Idle);
        // stale after 60 minutes -> nothing
        assert!(st.current(3_000 + 60 * MIN, 5 * MIN, 60 * MIN).is_none());

        // Stop -> idle immediately
        st.apply("Stop", "a", "C:\\p\\proj", 4_000);
        assert_eq!(st.current(4_000, 5 * MIN, 60 * MIN).unwrap().1, Phase::Idle);
        // start time survives
        assert_eq!(st.sessions[0].start, 1_000);
    }

    #[test]
    fn session_end_removes_and_latest_wins() {
        let mut st = State::default();
        st.apply("SessionStart", "a", "C:\\one", 1_000);
        st.apply("UserPromptSubmit", "b", "C:\\two", 2_000);
        assert_eq!(st.current(2_500, 5 * MIN, 60 * MIN).unwrap().0.id, "b");
        st.apply("session-end", "b", "C:\\two", 3_000);
        assert_eq!(st.current(3_500, 5 * MIN, 60 * MIN).unwrap().0.id, "a");
        st.apply("SessionEnd", "a", "C:\\one", 4_000);
        assert!(st.current(4_500, 5 * MIN, 60 * MIN).is_none());
    }

    #[test]
    fn project_change_resets_start() {
        let mut st = State::default();
        st.apply("SessionStart", "a", "C:\\one", 1_000);
        st.apply("Stop", "a", "C:\\one", 2_000);
        assert_eq!(st.sessions[0].start, 1_000);
        st.apply("UserPromptSubmit", "a", "C:\\two", 9_000);
        assert_eq!(st.sessions[0].start, 9_000);
    }

    #[test]
    fn json_roundtrip_and_garbage() {
        let mut st = State::default();
        st.apply("PreToolUse", "x", "C:\\a\\b", 5);
        let back = State::from_json(&st.to_json());
        assert_eq!(back.sessions, st.sessions);
        assert_eq!(State::from_json("not json").sessions.len(), 0);
    }

    #[test]
    fn project_names() {
        assert_eq!(project_name("C:\\a\\b"), "b");
        assert_eq!(project_name("C:\\a\\b\\"), "b");
        assert_eq!(project_name("/home/u/x"), "x");
    }

    #[test]
    fn hook_update_writes_atomically() {
        let dir = std::env::temp_dir().join(format!("claude-rpc-test-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        hook_update(&dir, "SessionStart", "s1", "C:\\x\\y", 10).unwrap();
        hook_update(&dir, "Stop", "s1", "C:\\x\\y", 20).unwrap();
        let st = State::from_json(&fs::read_to_string(state_path(&dir)).unwrap());
        assert_eq!(st.sessions.len(), 1);
        assert_eq!(st.last_event, 20);
        assert!(!dir.join("state.lock").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}

//! Discord IPC over the Windows named pipe, no external library.
//! Frame = u32 LE opcode + u32 LE length + JSON payload.

use crate::win;
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::time::{Duration, Instant};

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;
const MAX_PAYLOAD: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Activity {
    pub details: String,
    pub state: String,
    pub start_secs: Option<u64>,
    pub large_image: String,
    pub large_text: String,
    pub small_image: String,
    pub small_text: String,
}

#[derive(Debug, PartialEq)]
pub enum Incoming {
    Nothing,
    Closed,
    Error(String),
}

pub fn encode_frame(op: u32, payload: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
}

#[cfg(test)]
/// Decode one frame from the front of `buf`: (opcode, payload, bytes consumed).
pub fn decode_frame(buf: &[u8]) -> Option<(u32, &[u8], usize)> {
    if buf.len() < 8 {
        return None;
    }
    let op = u32::from_le_bytes(buf[0..4].try_into().ok()?);
    let len = u32::from_le_bytes(buf[4..8].try_into().ok()?) as usize;
    if buf.len() < 8 + len {
        return None;
    }
    Some((op, &buf[8..8 + len], 8 + len))
}

fn clip(s: &str) -> &str {
    // Discord limits strings to 128 chars
    match s.char_indices().nth(128) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Discord rejects details/state shorter than 2 characters (e.g. a 1-letter project name).
fn min2(s: &str) -> String {
    let c = clip(s);
    if c.chars().count() < 2 {
        format!("{c}\u{200B}")
    } else {
        c.to_string()
    }
}

pub fn activity_json(a: &Activity) -> Value {
    let mut act = json!({ "details": min2(&a.details) });
    if !a.state.is_empty() {
        act["state"] = json!(min2(&a.state));
    }
    if let Some(s) = a.start_secs {
        act["timestamps"] = json!({ "start": s });
    }
    let mut assets = serde_json::Map::new();
    for (k, v) in [
        ("large_image", &a.large_image),
        ("large_text", &a.large_text),
        ("small_image", &a.small_image),
        ("small_text", &a.small_text),
    ] {
        if !v.is_empty() {
            assets.insert(k.into(), json!(clip(v)));
        }
    }
    if !assets.is_empty() {
        act["assets"] = Value::Object(assets);
    }
    act
}

pub struct Conn {
    f: File,
    nonce: u64,
    buf: Vec<u8>,
    pub pipe: String,
}

impl Conn {
    /// Try discord-ipc-0..9.
    pub fn connect(client_id: &str) -> io::Result<Conn> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "discord-ipc pipe yok (Discord kapali?)");
        for i in 0..10 {
            let path = format!(r"\\.\pipe\discord-ipc-{i}");
            match Conn::connect_path(&path, client_id) {
                Ok(c) => return Ok(c),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    pub fn connect_path(path: &str, client_id: &str) -> io::Result<Conn> {
        let f = OpenOptions::new().read(true).write(true).open(path)?;
        let mut c = Conn { f, nonce: 0, buf: Vec::with_capacity(512), pipe: path.to_string() };
        let hs = json!({"v": 1, "client_id": client_id}).to_string();
        c.write_frame(OP_HANDSHAKE, hs.as_bytes())?;
        // wait for READY
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if Instant::now() > deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "READY gelmedi"));
            }
            if c.available()? == 0 {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            let (op, payload) = c.read_frame()?;
            match op {
                OP_FRAME => {
                    let v: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
                    if v["evt"] == "READY" {
                        return Ok(c);
                    }
                    if v["evt"] == "ERROR" {
                        return Err(io::Error::other(v["data"]["message"].as_str().unwrap_or("handshake hatasi").to_string()));
                    }
                }
                OP_CLOSE => {
                    let v: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
                    return Err(io::Error::other(
                        v["message"].as_str().unwrap_or("Discord baglantiyi kapatti (client_id gecerli mi?)").to_string(),
                    ));
                }
                OP_PING => c.write_frame(OP_PONG, &payload)?,
                _ => {}
            }
        }
    }

    fn available(&self) -> io::Result<u32> {
        win::pipe_available(self.f.as_raw_handle() as win::HANDLE)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "pipe koptu"))
    }

    fn write_frame(&mut self, op: u32, payload: &[u8]) -> io::Result<()> {
        encode_frame(op, payload, &mut self.buf);
        self.f.write_all(&self.buf)?;
        self.f.flush()
    }

    fn read_frame(&mut self) -> io::Result<(u32, Vec<u8>)> {
        let mut hdr = [0u8; 8];
        self.f.read_exact(&mut hdr)?;
        let op = u32::from_le_bytes(hdr[0..4].try_into().unwrap_or([0; 4]));
        let len = u32::from_le_bytes(hdr[4..8].try_into().unwrap_or([0; 4])) as usize;
        if len > MAX_PAYLOAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "frame cok buyuk"));
        }
        let mut payload = vec![0u8; len];
        self.f.read_exact(&mut payload)?;
        Ok((op, payload))
    }

    /// SET_ACTIVITY (None clears the activity).
    pub fn set_activity(&mut self, a: Option<&Activity>) -> io::Result<()> {
        self.nonce += 1;
        let activity = a.map(activity_json).unwrap_or(Value::Null);
        let msg = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity },
            "nonce": self.nonce.to_string(),
        })
        .to_string();
        self.write_frame(OP_FRAME, msg.as_bytes())
    }

    /// Drain pending frames without blocking: answer pings, report close/errors.
    pub fn poll(&mut self) -> io::Result<Incoming> {
        let mut result = Incoming::Nothing;
        while self.available()? >= 8 {
            let (op, payload) = self.read_frame()?;
            match op {
                OP_PING => self.write_frame(OP_PONG, &payload)?,
                OP_CLOSE => return Ok(Incoming::Closed),
                OP_FRAME => {
                    let v: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
                    if v["evt"] == "ERROR" {
                        result = Incoming::Error(v["data"]["message"].as_str().unwrap_or("bilinmeyen hata").to_string());
                    }
                }
                _ => {}
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::io::FromRawHandle;

    extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max: u32,
            out_size: u32,
            in_size: u32,
            timeout: u32,
            sa: *const c_void,
        ) -> win::HANDLE;
        fn ConnectNamedPipe(h: win::HANDLE, ov: *mut c_void) -> i32;
    }

    #[test]
    fn frame_roundtrip() {
        let mut out = Vec::new();
        encode_frame(OP_FRAME, b"{\"a\":1}", &mut out);
        assert_eq!(&out[0..4], &1u32.to_le_bytes());
        assert_eq!(&out[4..8], &7u32.to_le_bytes());
        let (op, p, used) = decode_frame(&out).unwrap();
        assert_eq!((op, p, used), (1, &b"{\"a\":1}"[..], 15));
        assert!(decode_frame(&out[..10]).is_none()); // truncated
        assert!(decode_frame(&out[..3]).is_none());
    }

    #[test]
    fn short_strings_are_padded_to_two_chars() {
        let a = Activity { details: "x".into(), state: "y".into(), ..Default::default() };
        let v = activity_json(&a);
        assert!(v["details"].as_str().unwrap().chars().count() >= 2);
        assert!(v["state"].as_str().unwrap().chars().count() >= 2);
        let long = Activity { details: "é".repeat(300), ..Default::default() };
        assert_eq!(activity_json(&long)["details"].as_str().unwrap().chars().count(), 128);
    }

    #[test]
    fn activity_json_shape() {
        let a = Activity {
            details: "proj".into(),
            state: "main · Aktif".into(),
            start_secs: Some(1700000000),
            large_image: "claude".into(),
            small_image: "active".into(),
            ..Default::default()
        };
        let v = activity_json(&a);
        assert_eq!(v["details"], "proj");
        assert_eq!(v["timestamps"]["start"], 1700000000u64);
        assert_eq!(v["assets"]["small_image"], "active");
        assert!(v["assets"].get("large_text").is_none());
    }

    #[test]
    fn mock_pipe_handshake_set_activity_ping_close() {
        let path = format!(r"\\.\pipe\claude-rpc-test-{}", std::process::id());
        let wpath = win::wide(&path);
        // duplex, byte mode, wait
        let h = unsafe { CreateNamedPipeW(wpath.as_ptr(), 3, 0, 1, 4096, 4096, 0, std::ptr::null()) };
        assert!(h != win::INVALID_HANDLE_VALUE && !h.is_null(), "CreateNamedPipeW failed");
        let hv = h as usize;

        let server = std::thread::spawn(move || {
            let h = hv as win::HANDLE;
            unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) };
            let mut f = unsafe { File::from_raw_handle(h as _) };
            let mut seen: Vec<(u32, Value)> = Vec::new();
            let read = |f: &mut File| -> (u32, Value) {
                let mut hdr = [0u8; 8];
                f.read_exact(&mut hdr).unwrap();
                let op = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
                let len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
                let mut p = vec![0u8; len];
                f.read_exact(&mut p).unwrap();
                (op, serde_json::from_slice(&p).unwrap())
            };
            let send = |f: &mut File, op: u32, v: Value| {
                let mut out = Vec::new();
                encode_frame(op, v.to_string().as_bytes(), &mut out);
                f.write_all(&out).unwrap();
            };
            seen.push(read(&mut f)); // handshake
            send(&mut f, OP_FRAME, json!({"cmd":"DISPATCH","evt":"READY","data":{"v":1}}));
            seen.push(read(&mut f)); // SET_ACTIVITY #1
            send(&mut f, OP_PING, json!({"nonce":"p"}));
            seen.push(read(&mut f)); // PONG
            seen.push(read(&mut f)); // SET_ACTIVITY #2 (clear)
            send(&mut f, OP_CLOSE, json!({"code":1000,"message":"bye"}));
            seen
        });

        let mut c = Conn::connect_path(&path, "12345").expect("handshake");
        let act = Activity { details: "proj".into(), state: "main · Aktif".into(), start_secs: Some(42), large_image: "claude".into(), small_image: "active".into(), ..Default::default() };
        c.set_activity(Some(&act)).unwrap();

        // ping must be answered by poll()
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(Instant::now() < deadline, "no ping seen");
            if c.available().unwrap() >= 8 {
                assert_eq!(c.poll().unwrap(), Incoming::Nothing);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        c.set_activity(None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(Instant::now() < deadline, "no close seen");
            match c.poll() {
                Ok(Incoming::Closed) => break,
                Ok(_) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }

        let seen = server.join().unwrap();
        assert_eq!(seen[0].0, OP_HANDSHAKE);
        assert_eq!(seen[0].1["v"], 1);
        assert_eq!(seen[0].1["client_id"], "12345");
        assert_eq!(seen[1].0, OP_FRAME);
        assert_eq!(seen[1].1["cmd"], "SET_ACTIVITY");
        assert_eq!(seen[1].1["args"]["pid"], std::process::id());
        assert_eq!(seen[1].1["args"]["activity"]["details"], "proj");
        assert_eq!(seen[1].1["args"]["activity"]["state"], "main · Aktif");
        assert_eq!(seen[1].1["args"]["activity"]["timestamps"]["start"], 42);
        assert!(seen[1].1["nonce"].is_string());
        assert_eq!(seen[2].0, OP_PONG);
        assert_eq!(seen[3].1["args"]["activity"], Value::Null);
    }
}

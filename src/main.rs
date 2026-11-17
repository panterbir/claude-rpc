#![windows_subsystem = "windows"]

mod claude;
mod config;
mod daemon;
mod discord;
mod git;
mod install;
mod state;
mod tray;
mod win;

use serde_json::Value;
use std::io::Read;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("hook") {
        config::trace(&format!("basladi v{} args={:?}", env!("CARGO_PKG_VERSION"), args));
    }
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "daemon" => daemon::run(false),
        "hook" => {
            hook(args.get(1).map(String::as_str));
            0 // never block Claude Code
        }
        other => {
            let console = win::attach_console();
            if args.is_empty() {
                // double click or plain `claude-rpc.exe` in a terminal: start the tray app
                return daemon::run(true);
            }
            let _ = console;
            cli(other, &args[1..])
        }
    }
}

/// `claude-rpc hook <event>`: read the hook JSON from stdin, update state.json, exit fast.
fn hook(arg: Option<&str>) {
    let mut buf = Vec::with_capacity(4096);
    let _ = std::io::stdin().lock().take(4 * 1024 * 1024).read_to_end(&mut buf);
    let body = buf.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&buf); // tolerate a UTF-8 BOM
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let event = v["hook_event_name"].as_str().or(arg).unwrap_or("");
    let id = v["session_id"].as_str().unwrap_or("");
    let cwd = v["cwd"]
        .as_str()
        .map(String::from)
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let _ = state::hook_update(&config::data_dir(), event, id, &cwd, state::now_ms());
}

fn cli(cmd: &str, rest: &[String]) -> i32 {
    match cmd {
        "install" => install::install(rest),
        "uninstall" => install::uninstall(rest),
        "status" => status(),
        "start" => {
            let cfg = config::load();
            if cfg.client_id.is_empty() {
                eprintln!("{}", config::MISSING_ID_MSG);
                return 1;
            }
            match std::env::current_exe().and_then(|e| install::start_daemon(&e)) {
                Ok(()) => {
                    println!("Daemon baslatildi");
                    0
                }
                Err(e) => {
                    eprintln!("Baslatilamadi: {e}");
                    1
                }
            }
        }
        "stop" => {
            if install::stop_daemon(&config::data_dir()) {
                println!("Daemon durduruldu");
            } else {
                println!("Daemon calismiyor");
            }
            0
        }
        _ => {
            println!(
                "claude-rpc - Claude icin Discord Rich Presence\n\n\
                 Kullanim:\n  \
                 claude-rpc install [--settings <yol>] [--no-registry] [--no-start] [--in-place]\n  \
                 claude-rpc uninstall [--settings <yol>] [--no-registry] [--no-start]\n  \
                 claude-rpc status | start | stop\n  \
                 claude-rpc               (argumansiz: tepsi uygulamasini baslatir)\n  \
                 claude-rpc daemon        (arka plan sureci)\n  \
                 claude-rpc hook <olay>   (Claude Code hook'lari cagirir)"
            );
            0
        }
    }
}

fn status() -> i32 {
    let dir = config::data_dir();
    let cfg = config::load();
    println!("Veri klasoru : {}", dir.display());
    println!("Client ID    : {}", if cfg.client_id.is_empty() { "YOK (config.toml / CLAUDE_RPC_CLIENT_ID)" } else { "ayarli" });
    println!("Otomatik baslat: {}", if win::run_key_get(install::RUN_VALUE).is_some() { "evet" } else { "hayir" });
    let st = std::fs::read_to_string(dir.join("status.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let pid = st.as_ref().and_then(|v| v["pid"].as_u64()).map(|p| p as u32);
    let alive = pid.is_some_and(win::pid_alive);
    println!("Daemon       : {}", if alive { format!("calisiyor (pid {})", pid.unwrap_or(0)) } else { "calismiyor".into() });
    if let (true, Some(v)) = (alive, &st) {
        println!("Discord      : {}", if v["connected"].as_bool() == Some(true) { "bagli" } else { "bagli degil" });
        let d = v["details"].as_str().unwrap_or("");
        if !d.is_empty() {
            println!("Presence     : {} / {}", d, v["state"].as_str().unwrap_or(""));
        } else {
            println!("Presence     : (yok)");
        }
        if let Some(e) = v["error"].as_str().filter(|e| !e.is_empty()) {
            println!("Son hata     : {e}");
        }
    } else if let Some(e) = st.as_ref().and_then(|v| v["error"].as_str()).filter(|e| !e.is_empty()) {
        println!("Son hata     : {e}");
    }
    let s = std::fs::read_to_string(state::state_path(&dir)).map(|t| state::State::from_json(&t)).unwrap_or_default();
    let now = state::now_ms();
    println!("Hook oturumlari: {}", s.sessions.len());
    for x in &s.sessions {
        println!(
            "  - {} [{}] {}s once",
            state::project_name(&x.cwd),
            if state::phase(x, now, cfg.idle_secs * 1000) == state::Phase::Active { "aktif" } else { "bosta" },
            now.saturating_sub(x.last) / 1000
        );
    }
    let mut pids = Vec::new();
    win::claude_desktop_pids(&mut pids);
    println!(
        "Claude Desktop: {} ({} surec, gorunur pencere: {})",
        if pids.is_empty() { "kapali" } else { "acik" },
        pids.len(),
        if win::has_visible_window(&pids) { "evet" } else { "hayir" }
    );
    0
}

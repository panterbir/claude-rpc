//! config.toml (tiny hand parser, no `toml` crate) + paths.

use std::path::PathBuf;

pub struct Config {
    pub client_id: String,
    pub idle_secs: u64,
    pub stale_secs: u64,
    pub active_text: String,
    pub idle_text: String,
    pub chat_details: String,
    pub large_image: String,
    pub small_active: String,
    pub small_idle: String,
    pub large_text: String,
    pub show_branch: bool,
    pub notify_on_start: bool,
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            client_id: String::new(),
            idle_secs: 5 * 60,
            stale_secs: 60 * 60,
            active_text: "Aktif".into(),
            idle_text: "Boşta".into(),
            chat_details: "Claude ile sohbet ediyor".into(),
            large_image: "https://host-gif.vercel.app/claude-spin.gif".into(),
            small_active: "active".into(),
            small_idle: "idle".into(),
            large_text: "Claude".into(),
            show_branch: true,
            notify_on_start: true,
            check_updates: true,
        }
    }
}

/// Default Discord Application ID ("AI Coding", assets: claude / active / idle).
/// Application IDs are public identifiers, not secrets. Override with `client_id` in
/// config.toml or the CLAUDE_RPC_CLIENT_ID environment variable to use your own app.
pub const DEFAULT_CLIENT_ID: &str = "1572334563114352660";

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("claude-rpc")
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if let Some(rest) = v.strip_prefix('"') {
        // up to the closing quote (ignores trailing comment)
        return rest.split('"').next().unwrap_or("").to_string();
    }
    // bare value: strip inline comment
    v.split('#').next().unwrap_or("").trim().to_string()
}

pub fn parse(text: &str, cfg: &mut Config) {
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = unquote(v);
        match k.trim() {
            "client_id" => cfg.client_id = v,
            "idle_minutes" => {
                if let Ok(n) = v.parse::<f64>() {
                    cfg.idle_secs = (n * 60.0) as u64;
                }
            }
            "stale_minutes" => {
                if let Ok(n) = v.parse::<f64>() {
                    cfg.stale_secs = (n * 60.0) as u64;
                }
            }
            "active_text" => cfg.active_text = v,
            "idle_text" => cfg.idle_text = v,
            "chat_details" => cfg.chat_details = v,
            "large_image" => cfg.large_image = v,
            "small_image_active" => cfg.small_active = v,
            "small_image_idle" => cfg.small_idle = v,
            "large_text" => cfg.large_text = v,
            "show_branch" => cfg.show_branch = !matches!(v.as_str(), "false" | "0" | "no"),
            "check_updates" => cfg.check_updates = !matches!(v.as_str(), "false" | "0" | "no"),
            "notify_on_start" => cfg.notify_on_start = !matches!(v.as_str(), "false" | "0" | "no"),
            _ => {}
        }
    }
}

/// Append one line to startup.log (diagnostics for "nothing happens" reports).
pub fn trace(msg: &str) {
    use std::io::Write;
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join("startup.log");
    if std::fs::metadata(&p).map(|m| m.len() > 32 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&p);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "[{}] {}", crate::state::now_ms() / 1000, msg);
    }
}

pub fn config_paths() -> Vec<PathBuf> {
    let mut v = vec![data_dir().join("config.toml")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            v.push(d.join("config.toml"));
        }
    }
    v
}

/// Env var CLAUDE_RPC_CLIENT_ID overrides config file.
pub fn load() -> Config {
    let mut cfg = Config::default();
    for p in config_paths() {
        if let Ok(t) = std::fs::read_to_string(&p) {
            parse(&t, &mut cfg);
            break;
        }
    }
    if let Ok(id) = std::env::var("CLAUDE_RPC_CLIENT_ID") {
        if !id.trim().is_empty() {
            cfg.client_id = id.trim().to_string();
        }
    }
    if cfg.client_id.is_empty() {
        cfg.client_id = DEFAULT_CLIENT_ID.to_string(); // works out of the box
    }
    cfg
}

pub const MISSING_ID_MSG: &str = "Discord Application ID bulunamadi. CLAUDE_RPC_CLIENT_ID ortam degiskenini ya da \
%LOCALAPPDATA%\\claude-rpc\\config.toml icinde client_id = \"...\" degerini ayarla \
(Discord Developer Portal -> uygulaman -> General Information -> Application ID).";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_values_and_comments() {
        let mut c = Config::default();
        parse("# x\nclient_id = \"123\" # id\nidle_minutes = 2\nshow_branch = false\nactive_text='x'\n[sec]\n", &mut c);
        assert_eq!(c.client_id, "123");
        assert_eq!(c.idle_secs, 120);
        assert!(!c.show_branch);
        assert_eq!(c.stale_secs, 3600);
    }
}

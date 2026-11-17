//! Self-update from GitHub Releases. HTTPS is delegated to the `curl.exe` that ships with
//! Windows 10/11 (no TLS stack in this binary). A running exe cannot be overwritten but it
//! can be renamed, so the swap is: download -> verify -> rename running exe to `.old` ->
//! rename new into place -> start the new exe -> exit.

use crate::tray;
use crate::win;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const REPO: &str = "panterbir/claude-rpc";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    pub url: String,
    pub sha256: Option<String>,
}

pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next().unwrap_or("0").parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    Some((a, b, c))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

pub fn parse_release(json: &str) -> Option<Release> {
    let v: Value = serde_json::from_str(json).ok()?;
    let tag = v["tag_name"].as_str()?.to_string();
    let a = v["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some("claude-rpc.exe"))?;
    let url = a["browser_download_url"].as_str()?.to_string();
    let sha256 = a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|s| s.to_ascii_lowercase());
    Some(Release { tag, url, sha256 })
}

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("curl.exe")
        .args(args)
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|e| format!("curl.exe çalıştırılamadı: {e}"))?;
    if !out.status.success() {
        return Err(format!("bağlantı/indirme başarısız (curl kodu {:?})", out.status.code()));
    }
    Ok(out.stdout)
}

pub fn fetch_latest() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = curl(&["-sSfL", "--max-time", "20", "-H", "User-Agent: claude-rpc", "-H", "Accept: application/vnd.github+json", &url])?;
    parse_release(&String::from_utf8_lossy(&body)).ok_or_else(|| "sürüm bilgisi okunamadı".to_string())
}

fn sha256_file(p: &Path) -> Option<String> {
    let out = Command::new("certutil.exe").arg("-hashfile").arg(p).arg("SHA256").creation_flags(NO_WINDOW).output().ok()?;
    let t = String::from_utf8_lossy(&out.stdout);
    t.lines().nth(1).map(|l| l.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase())
}

/// Download and swap the executable. Returns the path of the (new) exe.
pub fn apply(rel: &Release) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("new");
    let old = exe.with_extension("old");
    let _ = fs::remove_file(&new);
    curl(&["-sSfL", "--max-time", "180", "-o", new.to_str().ok_or("geçersiz yol")?, &rel.url])?;
    let len = fs::metadata(&new).map(|m| m.len()).unwrap_or(0);
    let mut head = [0u8; 2];
    let ok_head = fs::File::open(&new).and_then(|mut f| f.read_exact(&mut head)).is_ok() && &head == b"MZ";
    if len < 100_000 || !ok_head {
        let _ = fs::remove_file(&new);
        return Err("indirilen dosya geçersiz".into());
    }
    if let Some(want) = &rel.sha256 {
        if sha256_file(&new).as_deref() != Some(want.as_str()) {
            let _ = fs::remove_file(&new);
            return Err("indirilen dosyanın SHA-256 özeti GitHub'dakiyle uyuşmuyor".into());
        }
    }
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(|e| format!("mevcut exe yeniden adlandırılamadı: {e}"))?;
    if let Err(e) = fs::rename(&new, &exe) {
        let _ = fs::rename(&old, &exe);
        return Err(format!("yeni exe yerine konamadı: {e}"));
    }
    Ok(exe)
}

/// Start the new exe (it waits for this process to end) and exit.
fn restart(exe: &Path) -> ! {
    let _ = Command::new(exe)
        .arg("daemon")
        .env("CLAUDE_RPC_WAIT_PID", std::process::id().to_string())
        .creation_flags(0x0000_0008 | NO_WINDOW | 0x0000_0200)
        .spawn();
    tray::remove_icon();
    std::process::exit(0)
}

/// Remove leftovers of a previous update.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(exe.with_extension("old"));
        let _ = fs::remove_file(exe.with_extension("new"));
    }
}

/// If we were started by an update, wait for the previous process to release the single-instance mutex.
pub fn wait_for_previous() {
    if let Some(pid) = std::env::var("CLAUDE_RPC_WAIT_PID").ok().and_then(|p| p.parse::<u32>().ok()) {
        for _ in 0..100 {
            if !win::pid_alive(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// `interactive` = user clicked "Güncellemeleri denetle" (always answer); otherwise only speak up when an update exists.
pub fn check(interactive: bool, declined: &mut String) {
    match fetch_latest() {
        Err(e) => {
            if interactive {
                tray::message_box(&format!("Güncelleme denetlenemedi: {e}"), "Claude RPC");
            }
        }
        Ok(rel) if !is_newer(&rel.tag, CURRENT) => {
            if interactive {
                tray::message_box(&format!("Zaten en güncel sürümdesin (v{CURRENT})."), "Claude RPC");
            }
        }
        Ok(rel) => {
            if !interactive && *declined == rel.tag {
                return;
            }
            let q = format!(
                "Claude RPC'nin yeni sürümü var: {} (şu an v{}).\n\nŞimdi güncellemek ister misin? Uygulama indirilip otomatik olarak yeniden başlatılır.",
                rel.tag, CURRENT
            );
            if tray::ask_yes_no(&q, "Claude RPC güncellemesi") {
                match apply(&rel) {
                    Ok(exe) => restart(&exe),
                    Err(e) => tray::message_box(&format!("Güncelleme başarısız: {e}\n\nElle indirmek için: https://github.com/{REPO}/releases/latest"), "Claude RPC"),
                }
            } else {
                *declined = rel.tag.clone();
            }
        }
    }
}

pub fn manual_check() {
    check(true, &mut String::new());
}

/// Background loop: first check shortly after start, then every 6 hours.
pub fn background_loop() {
    std::thread::sleep(Duration::from_secs(25));
    let mut declined = String::new();
    loop {
        check(false, &mut declined);
        std::thread::sleep(Duration::from_secs(6 * 3600));
    }
}

/// `claude-rpc update [--yes]` for terminals/scripts (no dialogs).
pub fn cli(yes: bool) -> i32 {
    println!("Mevcut sürüm: v{CURRENT}");
    match fetch_latest() {
        Err(e) => {
            eprintln!("Denetlenemedi: {e}");
            1
        }
        Ok(rel) if !is_newer(&rel.tag, CURRENT) => {
            println!("Zaten en güncel sürümdesin ({}).", rel.tag);
            0
        }
        Ok(rel) => {
            println!("Yeni sürüm var: {}", rel.tag);
            if !yes {
                println!("Güncellemek için: claude-rpc update --yes");
                return 0;
            }
            match apply(&rel) {
                Ok(p) => {
                    println!("Güncellendi: {}. Çalışan bir daemon varsa tepsiden Çıkış yapıp yeniden başlat.", p.display());
                    0
                }
                Err(e) => {
                    eprintln!("Güncelleme başarısız: {e}");
                    1
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.2.10"), Some((0, 2, 10)));
        assert_eq!(parse_version("1.0"), Some((1, 0, 0)));
        assert_eq!(parse_version("v1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(parse_version("abc"), None);
        assert!(is_newer("v0.2.10", "0.2.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.2.2", "0.2.2"));
        assert!(!is_newer("v0.2.1", "0.2.2"));
        assert!(!is_newer("garbage", "0.2.2"));
    }

    #[test]
    fn parses_github_release_json() {
        let j = r#"{"tag_name":"v0.3.0","assets":[
          {"name":"other.zip","browser_download_url":"https://x/other.zip"},
          {"name":"claude-rpc.exe","browser_download_url":"https://github.com/o/r/releases/download/v0.3.0/claude-rpc.exe","digest":"sha256:ABCDEF"}]}"#;
        let r = parse_release(j).unwrap();
        assert_eq!(r.tag, "v0.3.0");
        assert!(r.url.ends_with("/claude-rpc.exe"));
        assert_eq!(r.sha256.as_deref(), Some("abcdef"));
        assert!(parse_release("{\"tag_name\":\"v1\",\"assets\":[]}").is_none());
        assert!(parse_release("nope").is_none());
    }
}

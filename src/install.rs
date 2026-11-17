//! install / uninstall: settings.json hook merge, HKCU Run entry, daemon start/stop.

use crate::{claude, config, state, win};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const RUN_VALUE: &str = "ClaudeRPC";

/// (settings.json event name, `hook` argument)
pub const EVENTS: [(&str, &str); 5] = [
    ("SessionStart", "session-start"),
    ("UserPromptSubmit", "user-prompt-submit"),
    ("PreToolUse", "pre-tool-use"),
    ("Stop", "stop"),
    ("SessionEnd", "session-end"),
];

fn is_ours(cmd: &str) -> bool {
    let l = cmd.to_ascii_lowercase();
    l.contains("claude-rpc") && l.contains(" hook ")
}

/// Remove our hook commands from every event. Returns true if anything was removed.
pub fn remove_hooks(settings: &mut Value) -> Result<bool, String> {
    let Some(root) = settings.as_object_mut() else { return Err("settings.json kok nesnesi degil".into()) };
    let Some(hooks) = root.get_mut("hooks") else { return Ok(false) };
    let Some(hooks_obj) = hooks.as_object_mut() else { return Err("\"hooks\" bir nesne degil".into()) };
    let mut changed = false;
    let mut empty_events = Vec::new();
    for (ev, groups) in hooks_obj.iter_mut() {
        let Some(arr) = groups.as_array_mut() else { continue };
        let mut touched = false;
        for g in arr.iter_mut() {
            if let Some(inner) = g.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                let n = inner.len();
                inner.retain(|h| !h["command"].as_str().is_some_and(is_ours));
                if inner.len() != n {
                    touched = true;
                }
            }
        }
        if touched {
            changed = true;
            // drop groups we emptied
            arr.retain(|g| g.get("hooks").and_then(|h| h.as_array()).is_none_or(|h| !h.is_empty()));
            if arr.is_empty() {
                empty_events.push(ev.clone());
            }
        }
    }
    for ev in empty_events {
        hooks_obj.shift_remove(&ev);
    }
    if changed && hooks_obj.is_empty() {
        root.shift_remove("hooks");
    }
    Ok(changed)
}

/// Add our hooks (idempotent: existing claude-rpc entries are replaced, nothing else touched).
/// `cmd_for(arg)` builds the command line. Returns true if the document changed.
pub fn merge_hooks(settings: &mut Value, cmd_for: &dyn Fn(&str) -> String) -> Result<bool, String> {
    let before = settings.clone();
    if !settings.is_object() {
        return Err("settings.json kok nesnesi degil".into());
    }
    remove_hooks(settings)?;
    let root = settings.as_object_mut().ok_or("settings.json kok nesnesi degil")?;
    let hooks = root.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().ok_or("\"hooks\" bir nesne degil")?;
    for (ev, arg) in EVENTS {
        let entry = hooks.entry(ev).or_insert_with(|| Value::Array(vec![]));
        let arr = entry.as_array_mut().ok_or_else(|| format!("hooks.{ev} bir dizi degil"))?;
        let mut group = json!({ "hooks": [ { "type": "command", "command": cmd_for(arg), "timeout": 5 } ] });
        if ev == "PreToolUse" {
            group = json!({ "matcher": "*", "hooks": [ { "type": "command", "command": cmd_for(arg), "timeout": 5 } ] });
        }
        arr.push(group);
    }
    Ok(*settings != before)
}

pub fn hook_command(exe: &Path, arg: &str) -> String {
    let p = exe.to_string_lossy().replace('\\', "/");
    let p = p.strip_prefix("//?/").unwrap_or(&p).to_string();
    if p.contains(' ') {
        format!("\"{p}\" hook {arg}")
    } else {
        format!("{p} hook {arg}")
    }
}

fn read_settings(path: &Path) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(t) => {
            let t = t.trim_start_matches('\u{feff}');
            if t.trim().is_empty() {
                return Ok(json!({}));
            }
            serde_json::from_str(t).map_err(|e| format!("{} gecerli JSON degil ({e}); dokunulmadi", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("{} okunamadi: {e}", path.display())),
    }
}

fn write_settings(path: &Path, v: &Value) -> Result<(), String> {
    if let Some(p) = path.parent() {
        let _ = fs::create_dir_all(p);
    }
    let mut text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    text.push('\n');
    state::write_atomic(path, text.as_bytes()).map_err(|e| format!("yazilamadi: {e}"))
}

fn backup(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let b = path.with_file_name(format!(
        "{}.bak-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("settings.json"),
        state::now_ms()
    ));
    fs::copy(path, &b).map_err(|e| format!("yedek alinamadi: {e}"))?;
    Ok(Some(b))
}

pub fn install_settings(path: &Path, exe: &Path) -> Result<String, String> {
    let mut v = read_settings(path)?;
    let changed = merge_hooks(&mut v, &|arg| hook_command(exe, arg))?;
    if !changed {
        return Ok(format!("{}: hook'lar zaten ekli, degisiklik yok", path.display()));
    }
    let b = backup(path)?;
    write_settings(path, &v)?;
    Ok(match b {
        Some(b) => format!("{}: hook'lar eklendi (yedek: {})", path.display(), b.display()),
        None => format!("{}: olusturuldu, hook'lar eklendi", path.display()),
    })
}

pub fn uninstall_settings(path: &Path) -> Result<String, String> {
    if !path.exists() {
        return Ok(format!("{}: yok, atlandi", path.display()));
    }
    let mut v = read_settings(path)?;
    if !remove_hooks(&mut v)? {
        return Ok(format!("{}: claude-rpc hook'u yok, degisiklik yok", path.display()));
    }
    let b = backup(path)?;
    write_settings(path, &v)?;
    Ok(format!("{}: hook'lar kaldirildi{}", path.display(), b.map(|b| format!(" (yedek: {})", b.display())).unwrap_or_default()))
}

pub fn read_daemon_pid(dir: &Path) -> Option<u32> {
    let t = fs::read_to_string(dir.join("status.json")).ok()?;
    let v: Value = serde_json::from_str(&t).ok()?;
    v["pid"].as_u64().map(|p| p as u32)
}

pub fn stop_daemon(dir: &Path) -> bool {
    match read_daemon_pid(dir) {
        // pid_alive alone is not enough: Windows reuses PIDs, so confirm it is really claude-rpc
        Some(pid)
            if win::pid_alive(pid)
                && win::process_image(pid).is_some_and(|p| p.to_ascii_lowercase().contains("claude-rpc")) =>
        {
            let ok = win::kill_pid(pid);
            std::thread::sleep(std::time::Duration::from_millis(300));
            ok
        }
        _ => false,
    }
}

pub fn start_daemon(exe: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    // DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
    std::process::Command::new(exe).arg("daemon").creation_flags(0x0000_0008 | 0x0800_0000 | 0x0000_0200).spawn().map(|_| ())
}

struct Opts {
    settings: PathBuf,
    no_registry: bool,
    no_start: bool,
    in_place: bool,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { settings: claude::claude_dir().join("settings.json"), no_registry: false, no_start: false, in_place: false };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--settings" => {
                i += 1;
                o.settings = PathBuf::from(args.get(i).ok_or("--settings icin yol gerekli")?);
            }
            "--no-registry" => o.no_registry = true,
            "--no-start" => o.no_start = true,
            "--in-place" => o.in_place = true,
            x => return Err(format!("bilinmeyen secenek: {x}")),
        }
        i += 1;
    }
    Ok(o)
}

pub fn install(args: &[String]) -> i32 {
    let o = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let dir = config::data_dir();
    let _ = fs::create_dir_all(&dir);
    let cur = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("exe yolu alinamadi: {e}");
            return 1;
        }
    };
    // Copy to a stable location so rebuilding the project never hits a locked exe.
    let exe = if o.in_place {
        cur.clone()
    } else {
        stop_daemon(&dir);
        let target = dir.join("claude-rpc.exe");
        if target != cur {
            if let Err(e) = fs::copy(&cur, &target) {
                eprintln!("exe kopyalanamadi ({}): {e}", target.display());
                return 1;
            }
        }
        target
    };
    println!("exe: {}", exe.display());

    match install_settings(&o.settings, &exe) {
        Ok(m) => println!("{m}"),
        Err(e) => {
            eprintln!("HATA: {e}");
            return 1;
        }
    }
    if !o.no_registry {
        let val = format!("\"{}\" daemon", exe.display());
        if win::run_key_set(RUN_VALUE, &val) {
            println!("Baslangic kaydi (HKCU Run\\{RUN_VALUE}) eklendi");
        } else {
            eprintln!("UYARI: Run kaydi yazilamadi");
        }
    }
    let cfg = config::load();
    if cfg.client_id.is_empty() {
        eprintln!("UYARI: {}", config::MISSING_ID_MSG);
        eprintln!("Daemon baslatilmadi. Ayari yaptiktan sonra: claude-rpc start");
    } else if !o.no_start {
        match start_daemon(&exe) {
            Ok(()) => println!("Daemon baslatildi"),
            Err(e) => eprintln!("Daemon baslatilamadi: {e}"),
        }
    }
    0
}

pub fn uninstall(args: &[String]) -> i32 {
    let o = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let dir = config::data_dir();
    let mut rc = 0;
    match uninstall_settings(&o.settings) {
        Ok(m) => println!("{m}"),
        Err(e) => {
            eprintln!("HATA: {e}");
            rc = 1;
        }
    }
    if !o.no_registry {
        if win::run_key_delete(RUN_VALUE) {
            println!("Baslangic kaydi kaldirildi");
        } else {
            println!("Baslangic kaydi zaten yok");
        }
    }
    if !o.no_start {
        if stop_daemon(&dir) {
            println!("Daemon durduruldu");
        }
    }
    rc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(arg: &str) -> String {
        format!("C:/x/claude-rpc.exe hook {arg}")
    }

    fn sample() -> Value {
        serde_json::from_str(
            r#"{
  "model": "opus",
  "permissions": {"allow": ["Bash(ls)"]},
  "hooks": {
    "PreToolUse": [
      {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo mine"}]}
    ],
    "PostToolUse": [
      {"hooks": [{"type": "command", "command": "echo post"}]}
    ]
  },
  "env": {"A": "1"}
}"#,
        )
        .unwrap()
    }

    #[test]
    fn merge_preserves_existing_and_is_idempotent() {
        let mut v = sample();
        assert!(merge_hooks(&mut v, &cmd).unwrap());
        // existing settings intact
        assert_eq!(v["model"], "opus");
        assert_eq!(v["permissions"]["allow"][0], "Bash(ls)");
        assert_eq!(v["env"]["A"], "1");
        assert_eq!(v["hooks"]["PostToolUse"][0]["hooks"][0]["command"], "echo post");
        // user's PreToolUse group still first, ours appended
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "echo mine");
        assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        for (ev, _) in EVENTS {
            assert!(v["hooks"][ev].is_array(), "{ev}");
        }
        // run again: no duplicates, no change
        let once = v.clone();
        assert!(!merge_hooks(&mut v, &cmd).unwrap());
        assert_eq!(v, once);
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        // key order preserved: model first
        assert_eq!(v.as_object().unwrap().keys().next().unwrap(), "model");
    }

    #[test]
    fn uninstall_restores_original() {
        let orig = sample();
        let mut v = orig.clone();
        merge_hooks(&mut v, &cmd).unwrap();
        assert!(remove_hooks(&mut v).unwrap());
        assert_eq!(v, orig);
        // second removal is a no-op
        assert!(!remove_hooks(&mut v).unwrap());
    }

    #[test]
    fn uninstall_on_empty_settings_removes_hooks_key() {
        let mut v = json!({"model": "x"});
        merge_hooks(&mut v, &cmd).unwrap();
        assert!(remove_hooks(&mut v).unwrap());
        assert_eq!(v, json!({"model": "x"}));
    }

    #[test]
    fn rejects_bad_shapes() {
        let mut v = json!({"hooks": []});
        assert!(merge_hooks(&mut v, &cmd).is_err());
        let mut v = json!({"hooks": {"Stop": {}}});
        assert!(merge_hooks(&mut v, &cmd).is_err());
        let mut v = json!([1]);
        assert!(merge_hooks(&mut v, &cmd).is_err());
    }

    #[test]
    fn file_roundtrip_with_backup() {
        let dir = std::env::temp_dir().join(format!("claude-rpc-inst-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");
        let original = "{\n  \"model\": \"opus\",\n  \"hooks\": {\"Stop\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"echo hi\"}]}]}\n}\n";
        fs::write(&p, original).unwrap();
        let exe = Path::new("C:\\Users\\x\\claude-rpc.exe");
        install_settings(&p, exe).unwrap();
        let backups: Vec<_> = fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains(".bak-")).collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), original);
        let v: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(v["hooks"]["SessionEnd"][0]["hooks"][0]["command"].as_str().unwrap().starts_with("C:/Users/x/claude-rpc.exe hook "));
        uninstall_settings(&p).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v, serde_json::from_str::<Value>(original).unwrap());
        // invalid JSON is never overwritten
        fs::write(&p, "{ broken").unwrap();
        assert!(install_settings(&p, exe).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "{ broken");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hook_command_quotes_spaces() {
        assert_eq!(hook_command(Path::new("C:\\a b\\c.exe"), "stop"), "\"C:/a b/c.exe\" hook stop");
    }
}

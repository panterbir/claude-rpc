//! Fallback sources when hooks give nothing: ~/.claude/projects mtime + Claude Desktop detection.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub fn claude_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join(".claude")
}

/// Claude Code encodes a cwd as a folder name by replacing every non-alphanumeric
/// character (`:` `\` `/` `.` space `_` `-` ...) with `-`.
pub fn encode_path(p: &str) -> String {
    p.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Reverse of `encode_path`. The mapping is lossy (`-` may have been `\`, `.`, ` ` ...), so
/// the real directory tree is walked and each entry's encoded name is matched against the rest.
pub fn decode_project_dir(name: &str) -> Option<PathBuf> {
    let b = name.as_bytes();
    if name.len() < 3 || !b[0].is_ascii_alphabetic() || &name[1..3] != "--" {
        return None;
    }
    let root = PathBuf::from(format!("{}:\\", &name[..1]));
    walk(&root, &name[3..])
}

fn walk(dir: &Path, rest: &str) -> Option<PathBuf> {
    if rest.is_empty() {
        return Some(dir.to_path_buf());
    }
    let entries = fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let fname = e.file_name();
        let enc = encode_path(&fname.to_string_lossy());
        if enc.is_empty() {
            continue;
        }
        if rest == enc {
            return Some(dir.join(&fname));
        }
        if let Some(r) = rest.strip_prefix(enc.as_str()).and_then(|r| r.strip_prefix('-')) {
            let p = e.path();
            if p.is_dir() {
                if let Some(found) = walk(&p, r) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Last-resort guess when the folder no longer exists: treat every `-` as a separator.
pub fn naive_decode(name: &str) -> String {
    match name.split_once("--") {
        Some((d, rest)) if d.len() == 1 => format!("{}:\\{}", d, rest.replace('-', "\\")),
        _ => name.replace('-', "\\"),
    }
}

/// (project folder name, mtime ms) of the most recently written session file.
pub fn latest_project(projects: &Path) -> Option<(String, u64)> {
    let mut best: Option<(String, u64)> = None;
    for d in fs::read_dir(projects).ok()?.flatten() {
        let Ok(files) = fs::read_dir(d.path()) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(m) = f.metadata().and_then(|m| m.modified()) else { continue };
            let ms = m.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
            if best.as_ref().is_none_or(|(_, b)| ms > *b) {
                best = Some((d.file_name().to_string_lossy().into_owned(), ms));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_matches_observed_names() {
        assert_eq!(
            encode_path(r"C:\Users\user\Desktop\claude\clauderichpresenceproject"),
            "C--Users-user-Desktop-claude-clauderichpresenceproject"
        );
        assert_eq!(encode_path(r"C:\Users\user\AppData\Local\Claude-Data"), "C--Users-user-AppData-Local-Claude-Data");
    }

    #[test]
    fn decode_handles_hyphens_dots_spaces() {
        let root = std::env::temp_dir().join(format!("claude-rpc-dec-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let deep = root.join("my-proj").join(".hidden dir").join("sub_x");
        fs::create_dir_all(&deep).unwrap();
        // decoy that shares a prefix
        fs::create_dir_all(root.join("my")).unwrap();
        let enc = encode_path(deep.to_str().unwrap());
        let got = decode_project_dir(&enc).expect("decoded");
        assert_eq!(got, deep);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn decode_missing_returns_none_and_naive_works() {
        assert!(decode_project_dir("C--definitely-not-here-xyz").is_none());
        assert_eq!(naive_decode("C--a-b"), "C:\\a\\b");
        assert!(decode_project_dir("garbage").is_none());
    }

    #[test]
    fn latest_project_picks_newest() {
        let root = std::env::temp_dir().join(format!("claude-rpc-lp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("A")).unwrap();
        fs::create_dir_all(root.join("B")).unwrap();
        fs::write(root.join("A").join("s.jsonl"), "x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        fs::write(root.join("B").join("s.jsonl"), "x").unwrap();
        fs::write(root.join("A").join("ignore.txt"), "x").unwrap();
        assert_eq!(latest_project(&root).unwrap().0, "B");
        let _ = fs::remove_dir_all(&root);
    }
}

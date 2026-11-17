//! Branch lookup by reading .git/HEAD directly (no `git` process).

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub enum Head {
    Branch(String),
    Detached(String), // short sha
}

pub fn parse_head(content: &str) -> Option<Head> {
    let c = content.trim();
    if let Some(r) = c.strip_prefix("ref:") {
        let r = r.trim();
        let name = r.strip_prefix("refs/heads/").or_else(|| r.strip_prefix("refs/")).unwrap_or(r);
        return Some(Head::Branch(name.to_string()));
    }
    if c.len() >= 7 && c.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(Head::Detached(c[..7].to_string()));
    }
    None
}

/// Walk upwards looking for `.git`. A `.git` *file* (worktree / submodule) is resolved
/// through its `gitdir:` line. Returns the directory that contains HEAD.
pub fn find_git_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let g = d.join(".git");
        if let Ok(md) = fs::metadata(&g) {
            if md.is_dir() {
                return Some(g);
            }
            if md.is_file() {
                let txt = fs::read_to_string(&g).ok()?;
                let line = txt.lines().find_map(|l| l.trim().strip_prefix("gitdir:"))?;
                let p = PathBuf::from(line.trim());
                return Some(if p.is_absolute() { p } else { d.join(p) });
            }
        }
        dir = d.parent();
    }
    None
}

/// Writes the branch (or short SHA) of the repo containing `cwd` into `out`.
/// Returns false when not a git repo. `buf` is a reusable scratch string.
pub fn current_branch(cwd: &str, out: &mut String, buf: &mut String) -> bool {
    out.clear();
    let Some(gd) = find_git_dir(Path::new(cwd)) else { return false };
    buf.clear();
    let Ok(mut f) = fs::File::open(gd.join("HEAD")) else { return false };
    if std::io::Read::read_to_string(&mut f, buf).is_err() {
        return false;
    }
    match parse_head(buf) {
        Some(Head::Branch(b)) | Some(Head::Detached(b)) => {
            out.push_str(&b);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("claude-rpc-git-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parse_variants() {
        assert_eq!(parse_head("ref: refs/heads/main\n"), Some(Head::Branch("main".into())));
        assert_eq!(parse_head("ref: refs/heads/feat/x-y\n"), Some(Head::Branch("feat/x-y".into())));
        assert_eq!(
            parse_head("0123456789abcdef0123456789abcdef01234567\n"),
            Some(Head::Detached("0123456".into()))
        );
        assert_eq!(parse_head("garbage"), None);
    }

    #[test]
    fn normal_repo_walks_up() {
        let r = tmp("normal");
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::write(r.join(".git").join("HEAD"), "ref: refs/heads/dev\n").unwrap();
        let sub = r.join("a").join("b");
        fs::create_dir_all(&sub).unwrap();
        let (mut o, mut b) = (String::new(), String::new());
        assert!(current_branch(sub.to_str().unwrap(), &mut o, &mut b));
        assert_eq!(o, "dev");
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn detached_head() {
        let r = tmp("det");
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::write(r.join(".git").join("HEAD"), "abcdef0123456789abcdef0123456789abcdef01\n").unwrap();
        let (mut o, mut b) = (String::new(), String::new());
        assert!(current_branch(r.to_str().unwrap(), &mut o, &mut b));
        assert_eq!(o, "abcdef0");
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn worktree_gitfile_relative_and_absolute() {
        let r = tmp("wt");
        // main repo with worktree metadata
        let wtmeta = r.join("main").join(".git").join("worktrees").join("w1");
        fs::create_dir_all(&wtmeta).unwrap();
        fs::write(wtmeta.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
        // worktree dir with .git file (absolute)
        let wt = r.join("w1");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", wtmeta.to_string_lossy().replace('\\', "/"))).unwrap();
        let (mut o, mut b) = (String::new(), String::new());
        assert!(current_branch(wt.to_str().unwrap(), &mut o, &mut b));
        assert_eq!(o, "wt-branch");
        // submodule style: relative path
        let sm = r.join("main").join("sub");
        fs::create_dir_all(&sm).unwrap();
        let modules = r.join("main").join(".git").join("modules").join("sub");
        fs::create_dir_all(&modules).unwrap();
        fs::write(modules.join("HEAD"), "ref: refs/heads/sm\n").unwrap();
        fs::write(sm.join(".git"), "gitdir: ../.git/modules/sub\n").unwrap();
        assert!(current_branch(sm.to_str().unwrap(), &mut o, &mut b));
        assert_eq!(o, "sm");
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn no_repo() {
        let r = tmp("none");
        let (mut o, mut b) = (String::new(), String::new());
        // temp dir parents normally have no .git
        if find_git_dir(&r).is_none() {
            assert!(!current_branch(r.to_str().unwrap(), &mut o, &mut b));
        }
        let _ = fs::remove_dir_all(&r);
    }
}

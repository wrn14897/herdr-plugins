//! Background git branch/dirty lookups so first paint never waits on git.

use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

#[derive(Debug, Clone)]
pub struct GitInfo {
    pub branch: String,
    pub dirty: bool,
}

/// Spawns one lookup thread per distinct directory; results arrive on `tx`.
pub fn spawn_lookups<'a>(
    dirs: impl IntoIterator<Item = &'a str>,
    tx: &Sender<(String, Option<GitInfo>)>,
) {
    let mut seen = HashSet::new();
    for dir in dirs {
        if dir.is_empty() || !seen.insert(dir.to_owned()) {
            continue;
        }
        let dir = dir.to_owned();
        let tx = tx.clone();
        thread::spawn(move || {
            let info = lookup(&dir);
            let _ = tx.send((dir, info));
        });
    }
}

fn lookup(dir: &str) -> Option<GitInfo> {
    let branch = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = if branch == "HEAD" {
        git(dir, &["rev-parse", "--short", "HEAD"]).map_or(branch, |sha| format!("@{sha}"))
    } else {
        branch
    };
    let dirty =
        git(dir, &["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    Some(GitInfo { branch, dirty })
}

fn git(dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

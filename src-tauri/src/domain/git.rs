// Git status for a working directory. Mirrors ClaudePet/src/shared/git.js.
//
// Spawns `git` (no libgit2 dependency) to read branch, dirty/staged/untracked
// counts, and ahead/behind vs upstream. Returns a non-repo marker on failure.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitInfo {
    pub is_repo: bool,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub dirty: u32,
    #[serde(default)]
    pub staged: u32,
    #[serde(default)]
    pub untracked: u32,
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
}

pub fn get_git_info(cwd: &Path) -> GitInfo {
    let inside = match run_git(cwd, &["rev-parse", "--is-inside-work-tree"]) {
        Some(s) if s.trim() == "true" => true,
        _ => return GitInfo { is_repo: false, ..Default::default() },
    };

    let branch = run_git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let porcelain = run_git(cwd, &["status", "--porcelain=v1", "-b"]).unwrap_or_default();
    let mut dirty = 0u32;
    let mut staged = 0u32;
    let mut untracked = 0u32;
    let mut ahead = 0u32;
    let mut behind = 0u32;

    for line in porcelain.lines() {
        if line.starts_with("## ") {
            // ## branch...origin/branch [ahead 2, behind 1]
            if let Some(rest) = line.split('[').nth(1) {
                let rest = rest.trim_end_matches(']');
                for token in rest.split(',') {
                    let token = token.trim();
                    if let Some(n) = token.strip_prefix("ahead ") {
                        ahead = n.parse().unwrap_or(0);
                    } else if let Some(n) = token.strip_prefix("behind ") {
                        behind = n.parse().unwrap_or(0);
                    }
                }
            }
            continue;
        }
        if line.len() < 2 {
            continue;
        }
        let x = line.as_bytes()[0];
        let y = line.as_bytes()[1];
        if x == b'?' && y == b'?' {
            untracked += 1;
        } else {
            if x != b' ' && x != b'?' {
                staged += 1;
            }
            if y != b' ' && y != b'?' {
                dirty += 1;
            }
        }
    }

    GitInfo {
        is_repo: inside,
        branch,
        dirty,
        staged,
        untracked,
        ahead,
        behind,
    }
}

fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

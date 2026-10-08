//! Current git branch of a directory, read straight from `HEAD` (without
//! spawning `git`), so it's cheap to call after every command.

use std::path::{Path, PathBuf};

pub fn branch(cwd: &str) -> Option<String> {
    let mut dir = PathBuf::from(cwd);
    loop {
        let dotgit = dir.join(".git");
        if dotgit.is_dir() {
            return head_to_branch(&dotgit.join("HEAD"));
        }
        if dotgit.is_file() {
            // Worktree or submodule: the `.git` file contains "gitdir: <path>".
            let content = std::fs::read_to_string(&dotgit).ok()?;
            let gitdir = content.trim().strip_prefix("gitdir:")?.trim();
            let gitdir = if Path::new(gitdir).is_absolute() {
                PathBuf::from(gitdir)
            } else {
                dir.join(gitdir)
            };
            return head_to_branch(&gitdir.join("HEAD"));
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// `ref: refs/heads/main` → `main`; detached HEAD → short SHA.
fn head_to_branch(head_path: &Path) -> Option<String> {
    let head = std::fs::read_to_string(head_path).ok()?;
    let head = head.trim();
    if let Some(rest) = head.strip_prefix("ref:") {
        let r = rest.trim();
        return Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string());
    }
    if head.is_empty() {
        None
    } else {
        Some(head.chars().take(7).collect())
    }
}

/// Uncommitted changes against HEAD: files, lines added and removed.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct DiffStat {
    pub files: u32,
    pub insertions: u32,
    pub deletions: u32,
}

/// `git diff --shortstat HEAD` in `cwd`. Spawns `git`, so it is called from
/// a separate thread. `None` if not a repository or git is unavailable.
pub fn diff_stat(cwd: &str) -> Option<DiffStat> {
    let out = std::process::Command::new("git")
        .args(["-C", cwd, "diff", "--shortstat", "HEAD"])
        // No index locks: don't interfere with other running git processes.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_shortstat(&String::from_utf8_lossy(&out.stdout)))
}

/// " 3 files changed, 48 insertions(+), 12 deletions(-)" → DiffStat.
fn parse_shortstat(s: &str) -> DiffStat {
    let mut stat = DiffStat::default();
    for part in s.split(',') {
        let mut words = part.split_whitespace();
        let (Some(n), Some(kind)) = (words.next(), words.next()) else {
            continue;
        };
        let Ok(n) = n.parse() else { continue };
        if kind.starts_with("file") {
            stat.files = n;
        } else if kind.starts_with("insertion") {
            stat.insertions = n;
        } else if kind.starts_with("deletion") {
            stat.deletions = n;
        }
    }
    stat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortstat() {
        let s = parse_shortstat(" 3 files changed, 48 insertions(+), 12 deletions(-)\n");
        assert_eq!(
            s,
            DiffStat {
                files: 3,
                insertions: 48,
                deletions: 12
            }
        );
        let s = parse_shortstat(" 1 file changed, 6 insertions(+)\n");
        assert_eq!(
            s,
            DiffStat {
                files: 1,
                insertions: 6,
                deletions: 0
            }
        );
        assert_eq!(parse_shortstat(""), DiffStat::default());
    }
}

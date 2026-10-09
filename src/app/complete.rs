//! `Tab` completion for the Cairn prompt: paths (relative to the tab's
//! directory) and commands from the user's shell PATH.
//!
//! The prompt belongs to Cairn, not zle, so the shell never sees the `Tab`:
//! this module does the work. It knows no command-specific completions
//! (git branches, options…), only files and commands.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A completion option.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// What the list shows (`src/`, `Cargo.toml`, `cargo`).
    pub label: String,
    /// What replaces the word (already escaped; with a trailing `/` or space).
    pub insert: String,
    pub dir: bool,
}

/// Result of completing the word right before the cursor.
pub struct Completion {
    /// Position (in chars) where the completed word starts.
    pub start: usize,
    pub candidates: Vec<Candidate>,
}

/// Cap on computed options (huge directories, the whole PATH).
const MAX: usize = 400;

/// Shell builtins that aren't on the PATH.
const BUILTINS: &[&str] = &[
    "alias", "bg", "bind", "builtin", "cd", "command", "echo", "eval", "exec", "exit", "export",
    "fg", "history", "jobs", "kill", "popd", "printf", "pushd", "pwd", "read", "source", "test",
    "type", "ulimit", "umask", "unalias", "unset", "wait", "which",
];

/// Words followed by another command.
const PREFIX_COMMANDS: &[&str] = &["sudo", "time", "exec", "nohup", "command", "which", "env"];

/// Completes `before` (the prompt text up to the cursor) in directory `cwd`.
pub fn complete(before: &str, cwd: &str) -> Completion {
    let words = split_words(before);
    let (start, raw) = match words.last() {
        // The cursor follows a space: a new, empty word.
        Some(w) if w.end == before.chars().count() => (w.start, w.raw.clone()),
        _ => (before.chars().count(), String::new()),
    };
    let is_new_word = words.last().is_none_or(|w| w.end != before.chars().count());
    let prev: Vec<&Word> = if is_new_word {
        words.iter().collect()
    } else {
        words[..words.len() - 1].iter().collect()
    };
    let command_position = prev.last().is_none_or(|w| w.separator)
        || prev
            .last()
            .is_some_and(|w| PREFIX_COMMANDS.contains(&w.text.as_str()));
    // Command of the current pipeline stage (for `cd`: folders only).
    let current_cmd = prev
        .iter()
        .rev()
        .take_while(|w| !w.separator)
        .last()
        .map(|w| w.text.clone());

    let quote = raw.chars().next().filter(|c| *c == '\'' || *c == '"');
    let typed = unquote(&raw);
    let candidates = if command_position && !typed.contains('/') && !typed.starts_with('~') {
        commands(&typed)
    } else {
        let only_dirs = matches!(current_cmd.as_deref(), Some("cd" | "pushd" | "rmdir"));
        paths(&typed, cwd, only_dirs, quote)
    };
    Completion { start, candidates }
}

/// Longest common prefix of the inserts (in chars).
pub fn common_prefix(candidates: &[Candidate]) -> String {
    let Some(first) = candidates.first() else {
        return String::new();
    };
    let mut prefix: Vec<char> = first.insert.chars().collect();
    for c in &candidates[1..] {
        let n = prefix
            .iter()
            .zip(c.insert.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        prefix.truncate(n);
    }
    // Without the trailing space of a truncated single candidate.
    prefix.into_iter().collect()
}

// ── Words ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Word {
    /// Position in chars.
    start: usize,
    end: usize,
    /// Raw text (with quotes and escapes).
    raw: String,
    /// Text without quotes or escapes.
    text: String,
    /// Command separator (`|`, `;`, `&&`…).
    separator: bool,
}

/// Splits the line into words, honoring quotes and `\`. Command separators
/// are words of their own.
fn split_words(line: &str) -> Vec<Word> {
    let chars: Vec<char> = line.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if "|;&()".contains(c) {
            let start = i;
            while i < chars.len() && "|;&()".contains(chars[i]) {
                i += 1;
            }
            let raw: String = chars[start..i].iter().collect();
            words.push(Word {
                start,
                end: i,
                text: raw.clone(),
                raw,
                separator: true,
            });
            continue;
        }
        let start = i;
        let mut quote: Option<char> = None;
        while i < chars.len() {
            let c = chars[i];
            match quote {
                Some(q) if c == q => quote = None,
                Some('"') if c == '\\' => i += 1,
                Some(_) => {}
                None if c == '\\' => i += 1,
                None if c == '\'' || c == '"' => quote = Some(c),
                None if c.is_whitespace() || "|;&()".contains(c) => break,
                None => {}
            }
            i += 1;
        }
        let end = i.min(chars.len());
        let raw: String = chars[start..end].iter().collect();
        words.push(Word {
            start,
            end,
            text: unquote(&raw),
            raw,
            separator: false,
        });
    }
    words
}

/// Strips quotes and escapes from a word (even if a quote is left open).
fn unquote(raw: &str) -> String {
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            Some(_) => out.push(c),
            None if c == '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            None if c == '\'' || c == '"' => quote = Some(c),
            None => out.push(c),
        }
    }
    out
}

/// Escapes shell special characters with `\`.
fn escape(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_whitespace() || "'\"\\$`!&|;()<>*?[]#{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// ── Paths ────────────────────────────────────────────────────────────────────

fn paths(typed: &str, cwd: &str, only_dirs: bool, quote: Option<char>) -> Vec<Candidate> {
    // Directory part as typed, and the name prefix.
    let (dir_typed, prefix) = match typed.rfind('/') {
        Some(i) => (&typed[..=i], &typed[i + 1..]),
        None => ("", typed),
    };
    let home = dirs::home_dir();
    let expanded = if dir_typed == "~/" || dir_typed.starts_with("~/") {
        home.as_ref()
            .map(|h| h.join(&dir_typed[2..]))
            .unwrap_or_else(|| PathBuf::from(dir_typed))
    } else if typed == "~" {
        // Bare `~`: completes to `~/`.
        return vec![Candidate {
            label: "~/".into(),
            insert: "~/".into(),
            dir: true,
        }];
    } else {
        PathBuf::from(dir_typed)
    };
    let base = if expanded.is_absolute() {
        expanded
    } else {
        Path::new(cwd).join(expanded)
    };
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let show_hidden = prefix.starts_with('.');
    let mut names: Vec<(String, bool)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !show_hidden {
                return None;
            }
            // Follow symlinks to tell whether it's a folder.
            let dir = std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir());
            (!only_dirs || dir).then_some((name, dir))
        })
        .collect();
    let mut matching: Vec<(String, bool)> = names
        .iter()
        .filter(|(n, _)| n.starts_with(prefix))
        .cloned()
        .collect();
    if matching.is_empty() {
        // No exact matches: retry case-insensitively.
        let lower = prefix.to_lowercase();
        matching = names
            .drain(..)
            .filter(|(n, _)| n.to_lowercase().starts_with(&lower))
            .collect();
    }
    matching.sort_by_key(|(name, _)| name.to_lowercase());
    matching.truncate(MAX);
    matching
        .into_iter()
        .map(|(name, dir)| {
            let label = if dir {
                format!("{name}/")
            } else {
                name.clone()
            };
            let insert = match quote {
                // Inside quotes: a folder leaves the quote open to keep going.
                Some(q) if dir => format!("{q}{dir_typed}{name}/"),
                Some(q) => format!("{q}{dir_typed}{name}{q} "),
                None if dir => format!("{}{}/", escape(dir_typed), escape(&name)),
                None => format!("{}{} ", escape(dir_typed), escape(&name)),
            };
            Candidate { label, insert, dir }
        })
        .collect()
}

// ── Commands ─────────────────────────────────────────────────────────────────

fn commands(prefix: &str) -> Vec<Candidate> {
    let mut names: BTreeSet<String> = BUILTINS
        .iter()
        .filter(|b| b.starts_with(prefix))
        .map(|b| b.to_string())
        .collect();
    for dir in shell_path() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(prefix) && is_executable(&e.path()) {
                names.insert(name);
            }
        }
    }
    names
        .into_iter()
        .take(MAX)
        .map(|n| Candidate {
            insert: format!("{} ", escape(&n)),
            label: n,
            dir: false,
        })
        .collect()
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// PATH of the user's login shell (the app's may be Finder's minimal one).
/// Computed once; `warm_up` does it in the background at startup.
fn shell_path() -> &'static [PathBuf] {
    static PATH: OnceLock<Vec<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let shell = crate::term::shell::default_shell();
        let from_shell = std::process::Command::new(&shell)
            .args(["-l", "-c", "printf %s \"$PATH\""])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let own = std::env::var("PATH").unwrap_or_default();
        let mut seen = BTreeSet::new();
        from_shell
            .split(':')
            .chain(own.split(':'))
            .filter(|p| !p.is_empty() && seen.insert(p.to_string()))
            .map(PathBuf::from)
            .collect()
    })
}

/// Computes the shell PATH in the background so the first `Tab` doesn't wait.
pub fn warm_up() {
    std::thread::spawn(|| {
        let _ = shell_path();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cairn-complete-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("name_of_folder")).unwrap();
        std::fs::create_dir_all(d.join("name_of_other")).unwrap();
        std::fs::create_dir_all(d.join("my folder")).unwrap();
        std::fs::write(d.join("notes.md"), "").unwrap();
        std::fs::write(d.join(".hidden"), "").unwrap();
        d
    }

    fn inserts(c: &Completion) -> Vec<String> {
        c.candidates.iter().map(|c| c.insert.clone()).collect()
    }

    #[test]
    fn several_folders_and_common_prefix() {
        let d = tmp("several_folders_and_common_prefix");
        let c = complete("cd name_of", d.to_str().unwrap());
        assert_eq!(c.start, 3);
        assert_eq!(inserts(&c), ["name_of_folder/", "name_of_other/"]);
        assert_eq!(common_prefix(&c.candidates), "name_of_");
    }

    #[test]
    fn single_match_and_spaces() {
        let d = tmp("single_match_and_spaces");
        let c = complete("ls my", d.to_str().unwrap());
        assert_eq!(inserts(&c), ["my\\ folder/"]);
        let c = complete("cat not", d.to_str().unwrap());
        assert_eq!(inserts(&c), ["notes.md "]);
        let c = complete("cat \"my", d.to_str().unwrap());
        assert_eq!(inserts(&c), ["\"my folder/"]);
    }

    #[test]
    fn cd_only_folders_and_hidden() {
        let d = tmp("cd_only_folders_and_hidden");
        let c = complete("cd n", d.to_str().unwrap());
        assert!(c.candidates.iter().all(|c| c.dir));
        let c = complete("cat .", d.to_str().unwrap());
        assert_eq!(inserts(&c), [".hidden "]);
    }

    #[test]
    fn command_position() {
        let words = split_words("ls -la | gr");
        assert!(words[2].separator);
        let c = complete("ech", "/");
        assert!(c.candidates.iter().any(|c| c.label == "echo"));
        let c = complete("sudo ech", "/");
        assert!(c.candidates.iter().any(|c| c.label == "echo"));
    }
}

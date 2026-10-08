//! File-based persistence, with the same XDG layout on macOS and Linux:
//!
//! - `~/.config/cairn/config.toml`: startup profiles and snippets. The app
//!   writes it, but it is meant to be edited by hand and kept in your
//!   dotfiles. The profile name and the snippet label are their keys: adding
//!   one with the same key replaces the previous one.
//! - `~/.local/share/cairn/history.jsonl`: history, one JSON line per
//!   command. Loaded in full at startup, trimmed to `HISTORY_MAX`, and each
//!   new command is appended to the end of the file.
//!
//! The SQLite database from earlier versions is imported the first time
//! (`legacy`).

#[cfg(feature = "legacy-db")]
mod legacy;
mod paths;

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::path::Path;

use anyhow::Context;
pub use paths::Paths;
use serde::{Deserialize, Serialize};

/// History entries kept.
const HISTORY_MAX: usize = 10_000;

/// Header of the `config.toml` the app writes. The file is regenerated in
/// full every time, so user comments do not survive.
const CONFIG_HEADER: &str = "\
# Cairn startup profiles and snippets.
# The app rewrites this file when adding or deleting: comments are not kept.
#
# update_check = false   # do not ask GitHub for new releases at startup
#
# [[profiles]]
# name = \"work\"
# shell = \"/bin/zsh\"
# cwd = \"~/Offing\"
#
# [[snippets]]
# label = \"logs\"
# command = \"make logs\"

";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub shell: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub command: String,
    /// Milliseconds since the Unix epoch.
    pub ts: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    /// Ask GitHub once a day, at startup, whether there is a newer release.
    /// First: TOML wants plain values before the tables.
    update_check: bool,
    profiles: Vec<Profile>,
    snippets: Vec<Snippet>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            update_check: true,
            profiles: Vec::new(),
            snippets: Vec::new(),
        }
    }
}

pub struct Store {
    paths: Paths,
    config: Config,
    /// Oldest to newest.
    history: Vec<HistoryEntry>,
}

impl Store {
    /// The user's data. The first time there are no files, it brings over
    /// whatever was in the SQLite database from earlier versions.
    pub fn open() -> anyhow::Result<Self> {
        let paths = Paths::from_env();
        paths.adopt_old_dirs();
        #[cfg(feature = "legacy-db")]
        if !paths.config_file().exists() && !paths.history_file().exists() {
            import_legacy(&paths.config_file(), &paths.history_file());
        }
        Self::open_at(paths)
    }

    /// Only what is in `paths`: it never looks elsewhere, so it is usable in
    /// tests without touching real data.
    pub fn open_at(paths: Paths) -> anyhow::Result<Self> {
        let config_file = paths.config_file();
        let history_file = paths.history_file();
        let config = read_config(&config_file)?;
        let mut history = read_history(&history_file)?;
        if history.len() > HISTORY_MAX {
            history.drain(..history.len() - HISTORY_MAX);
            write_history(&history_file, &history)?;
        }
        Ok(Self {
            paths,
            config,
            history,
        })
    }

    /// Where the files live, to tell the user.
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// Whether the startup check for a newer release is on.
    pub fn update_check(&self) -> bool {
        self.config.update_check
    }

    // ── Profiles ─────────────────────────────────────────────────────────────

    pub fn profiles(&self) -> &[Profile] {
        &self.config.profiles
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.config.profiles.iter().find(|p| p.name == name)
    }

    /// Adds a profile; if one with that name exists, replaces it.
    pub fn add_profile(&mut self, name: &str, shell: &str, cwd: &str) -> anyhow::Result<()> {
        self.config.profiles.retain(|p| p.name != name);
        self.config.profiles.push(Profile {
            name: name.into(),
            shell: shell.into(),
            cwd: cwd.into(),
        });
        self.save_config()
    }

    pub fn delete_profile(&mut self, name: &str) -> anyhow::Result<()> {
        self.config.profiles.retain(|p| p.name != name);
        self.save_config()
    }

    // ── Snippets ─────────────────────────────────────────────────────────────

    pub fn snippets(&self) -> &[Snippet] {
        &self.config.snippets
    }

    /// Adds a snippet; if one with that label exists, replaces it.
    pub fn add_snippet(&mut self, label: &str, command: &str) -> anyhow::Result<()> {
        self.config.snippets.retain(|s| s.label != label);
        self.config.snippets.push(Snippet {
            label: label.into(),
            command: command.into(),
        });
        self.save_config()
    }

    pub fn delete_snippet(&mut self, label: &str) -> anyhow::Result<()> {
        self.config.snippets.retain(|s| s.label != label);
        self.save_config()
    }

    // ── History ──────────────────────────────────────────────────────────────

    /// Newest to oldest.
    pub fn history(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.history.iter().rev()
    }

    pub fn add_history(
        &mut self,
        profile: Option<&str>,
        command: &str,
        cwd: Option<&str>,
    ) -> anyhow::Result<()> {
        let entry = HistoryEntry {
            command: command.into(),
            ts: chrono::Utc::now().timestamp_millis(),
            cwd: cwd.map(Into::into),
            profile: profile.map(Into::into),
        };
        append_history(&self.paths.history_file(), &entry)?;
        self.history.push(entry);
        Ok(())
    }

    pub fn clear_history(&mut self) -> anyhow::Result<()> {
        self.history.clear();
        write_history(&self.paths.history_file(), &[])
    }

    fn save_config(&self) -> anyhow::Result<()> {
        let body =
            toml::to_string_pretty(&self.config).context("cannot serialize the configuration")?;
        write_atomic(&self.paths.config_file(), &format!("{CONFIG_HEADER}{body}"))
    }
}

fn read_config(path: &Path) -> anyhow::Result<Config> {
    match fs::read_to_string(path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("{} is malformed", path.display()))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn read_history(path: &Path) -> anyhow::Result<Vec<HistoryEntry>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let mut out = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.with_context(|| format!("cannot read {}", path.display()))?;
        // A broken line (a half-written close) does not invalidate the rest.
        if let Ok(entry) = serde_json::from_str::<HistoryEntry>(&line) {
            out.push(entry);
        }
    }
    Ok(out)
}

fn append_history(path: &Path, entry: &HistoryEntry) -> anyhow::Result<()> {
    ensure_parent(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    file.write_all(line.as_bytes())
        .with_context(|| format!("cannot write to {}", path.display()))
}

fn write_history(path: &Path, entries: &[HistoryEntry]) -> anyhow::Result<()> {
    let mut text = String::new();
    for e in entries {
        text.push_str(&serde_json::to_string(e)?);
        text.push('\n');
    }
    write_atomic(path, &text)
}

/// Writes to a temp file and renames it: if something is cut off halfway,
/// the previous file remains.
fn write_atomic(path: &Path, text: &str) -> anyhow::Result<()> {
    ensure_parent(path)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))
}

fn ensure_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    Ok(())
}

/// Dumps the first existing SQLite database into the new files and moves it
/// aside. A failure here does not prevent startup: it warns and carries on
/// with empty data.
#[cfg(feature = "legacy-db")]
fn import_legacy(config_file: &Path, history_file: &Path) {
    let Some(old) = legacy::candidates().into_iter().find(|p| p.exists()) else {
        return;
    };
    if let Err(e) = import_from(&old, config_file, history_file) {
        eprintln!("cairn: could not import {}: {e:#}", old.display());
    }
}

#[cfg(feature = "legacy-db")]
fn import_from(old: &Path, config_file: &Path, history_file: &Path) -> anyhow::Result<()> {
    let data = legacy::read(old)?;
    let config = Config {
        profiles: data.profiles,
        snippets: data.snippets,
        ..Config::default()
    };
    let body = toml::to_string_pretty(&config)?;
    write_atomic(config_file, &format!("{CONFIG_HEADER}{body}"))?;
    write_history(history_file, &data.history)?;
    legacy::retire(old);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "legacy-db")]
    use std::path::PathBuf;

    /// Clean directory per test; no `tempfile`, which would be another dependency.
    fn scratch(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("cairn-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Paths {
            config_dir: dir.join("config"),
            data_dir: dir.join("data"),
            state_dir: dir.join("state"),
        }
    }

    fn open(paths: &Paths) -> Store {
        Store::open_at(paths.clone()).unwrap()
    }

    #[test]
    fn profiles_and_snippets_round_trip() {
        let paths = scratch("config");
        let mut s = open(&paths);
        assert!(s.profiles().is_empty());
        s.add_profile("work", "/bin/zsh", "~/Offing").unwrap();
        s.add_snippet("logs", "make logs").unwrap();
        // Same key: replaces, does not duplicate.
        s.add_snippet("logs", "make logs -f").unwrap();

        let s = open(&paths);
        assert_eq!(
            s.profile("work").map(|p| p.shell.as_str()),
            Some("/bin/zsh")
        );
        assert_eq!(s.snippets().len(), 1);
        assert_eq!(s.snippets()[0].command, "make logs -f");
        let text = fs::read_to_string(paths.config_file()).unwrap();
        assert!(text.contains("[[profiles]]") && text.contains("[[snippets]]"));

        let mut s = s;
        s.delete_profile("work").unwrap();
        s.delete_snippet("logs").unwrap();
        let s = open(&paths);
        assert!(s.profiles().is_empty() && s.snippets().is_empty());
    }

    #[test]
    fn hand_written_config_missing_a_table() {
        let paths = scratch("manual");
        fs::create_dir_all(&paths.config_dir).unwrap();
        fs::write(
            paths.config_file(),
            "[[snippets]]\nlabel = \"x\"\ncommand = \"echo x\"\n",
        )
        .unwrap();
        let s = open(&paths);
        assert!(s.profiles().is_empty());
        assert_eq!(s.snippets()[0].label, "x");
    }

    #[test]
    fn history_appends_reads_reversed_and_clears() {
        let paths = scratch("history");
        let mut s = open(&paths);
        s.add_history(None, "ls", Some("/tmp")).unwrap();
        s.add_history(Some("work"), "make check", None).unwrap();

        let s = open(&paths);
        let cmds: Vec<_> = s.history().map(|h| h.command.as_str()).collect();
        assert_eq!(cmds, ["make check", "ls"]);
        assert_eq!(s.history().next().unwrap().profile.as_deref(), Some("work"));

        let mut s = s;
        s.clear_history().unwrap();
        assert_eq!(open(&paths).history().count(), 0);
    }

    #[test]
    fn broken_line_does_not_break_history_and_trims_on_startup() {
        let paths = scratch("trim");
        fs::create_dir_all(&paths.data_dir).unwrap();
        let mut text = String::from("{this is not json\n");
        for i in 0..HISTORY_MAX + 5 {
            text.push_str(&format!("{{\"command\":\"c{i}\",\"ts\":{i}}}\n"));
        }
        fs::write(paths.history_file(), text).unwrap();

        let s = open(&paths);
        assert_eq!(s.history().count(), HISTORY_MAX);
        assert_eq!(
            s.history().next().unwrap().command,
            format!("c{}", HISTORY_MAX + 4)
        );
        // The file ends up trimmed and without the broken line.
        let lines = fs::read_to_string(paths.history_file())
            .unwrap()
            .lines()
            .count();
        assert_eq!(lines, HISTORY_MAX);
    }

    /// Database with the 0.3 schema (no `cwd` in snippets, `profile_id` in
    /// history) as the old `db.rs` left it.
    #[cfg(feature = "legacy-db")]
    #[test]
    fn imports_old_sqlite_db_and_moves_it_aside() {
        let paths = scratch("legacy");
        fs::create_dir_all(&paths.data_dir).unwrap();
        let old: PathBuf = paths.data_dir.join("prism.db");
        let conn = rusqlite::Connection::open(&old).unwrap();
        conn.execute_batch(
            "CREATE TABLE profiles (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, shell TEXT NOT NULL, cwd TEXT NOT NULL);
             CREATE TABLE history (id INTEGER PRIMARY KEY AUTOINCREMENT, profile_id INTEGER, command TEXT NOT NULL, ts INTEGER NOT NULL, cwd TEXT);
             CREATE TABLE snippets (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL, command TEXT NOT NULL);
             INSERT INTO profiles (name, shell, cwd) VALUES ('work', '/bin/zsh', '~/Offing');
             INSERT INTO snippets (label, command) VALUES ('logs', 'make logs');
             INSERT INTO history (profile_id, command, ts, cwd) VALUES (NULL, 'ls', 1, '/tmp');
             INSERT INTO history (profile_id, command, ts, cwd) VALUES (1, 'make check', 2, NULL);",
        )
        .unwrap();
        drop(conn);

        import_from(&old, &paths.config_file(), &paths.history_file()).unwrap();
        assert!(!old.exists() && paths.data_dir.join("prism.db.bak").exists());

        let s = open(&paths);
        assert_eq!(s.profile("work").map(|p| p.cwd.as_str()), Some("~/Offing"));
        assert_eq!(s.snippets()[0].command, "make logs");
        let h: Vec<_> = s.history().collect();
        assert_eq!(h[0].command, "make check");
        assert_eq!(h[0].profile.as_deref(), Some("work"));
        assert_eq!(h[1].cwd.as_deref(), Some("/tmp"));
    }
}

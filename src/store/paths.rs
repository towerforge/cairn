//! Config, data and state paths. XDG on macOS too: a terminal tool has no
//! business in `~/Library/Application Support`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

const APP: &str = "cairn";
/// The app's name before it became Cairn.
const OLD_APP: &str = "prism";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    /// Things worth keeping between runs but not worth backing up: the note
    /// of the last update check.
    pub state_dir: PathBuf,
}

impl Paths {
    /// Reads `$HOME` and the `XDG_*` variables from the environment.
    pub fn from_env() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::resolve(
            &home,
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("XDG_DATA_HOME"),
            std::env::var_os("XDG_STATE_HOME"),
        )
    }

    /// Resolves the paths from explicit values. An empty variable counts as
    /// unset.
    pub fn resolve(
        home: &Path,
        xdg_config: Option<OsString>,
        xdg_data: Option<OsString>,
        xdg_state: Option<OsString>,
    ) -> Self {
        fn pick(var: Option<OsString>, home: &Path, fallback: &str) -> PathBuf {
            match var {
                Some(v) if !v.is_empty() => PathBuf::from(v),
                _ => home.join(fallback),
            }
        }
        Self {
            config_dir: pick(xdg_config, home, ".config").join(APP),
            data_dir: pick(xdg_data, home, ".local/share").join(APP),
            state_dir: pick(xdg_state, home, ".local/state").join(APP),
        }
    }

    /// Startup profiles and snippets.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Command history, one JSON line per entry.
    pub fn history_file(&self) -> PathBuf {
        self.data_dir.join("history.jsonl")
    }

    /// Moves the directories left by the app under its old name (`prism`) to
    /// the new ones, unless those already exist.
    pub fn adopt_old_dirs(&self) {
        for dir in [&self.config_dir, &self.data_dir, &self.state_dir] {
            let old = dir.with_file_name(OLD_APP);
            if !dir.exists() && old.is_dir() {
                let _ = std::fs::rename(&old, dir);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_under_home() {
        let p = Paths::resolve(Path::new("/home/j"), None, None, None);
        assert_eq!(
            p.config_file(),
            PathBuf::from("/home/j/.config/cairn/config.toml")
        );
        assert_eq!(
            p.history_file(),
            PathBuf::from("/home/j/.local/share/cairn/history.jsonl")
        );
        assert_eq!(p.state_dir, PathBuf::from("/home/j/.local/state/cairn"));
    }

    #[test]
    fn honours_xdg_and_ignores_empty() {
        let p = Paths::resolve(
            Path::new("/home/j"),
            Some("/etc/x".into()),
            Some("".into()),
            Some("/var/s".into()),
        );
        assert_eq!(p.config_dir, PathBuf::from("/etc/x/cairn"));
        assert_eq!(p.data_dir, PathBuf::from("/home/j/.local/share/cairn"));
        assert_eq!(p.state_dir, PathBuf::from("/var/s/cairn"));
    }

    #[test]
    fn adopts_the_old_dirs_but_never_over_new_ones() {
        let home = std::env::temp_dir().join(format!("cairn-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let p = Paths::resolve(&home, None, None, None);
        let old_config = home.join(".config/prism");
        let old_data = home.join(".local/share/prism");
        std::fs::create_dir_all(&old_config).unwrap();
        std::fs::write(old_config.join("config.toml"), "x").unwrap();
        std::fs::create_dir_all(&old_data).unwrap();
        std::fs::create_dir_all(&p.data_dir).unwrap();

        p.adopt_old_dirs();
        assert!(p.config_file().exists() && !old_config.exists());
        assert!(old_data.exists(), "an existing new dir is left alone");
        let _ = std::fs::remove_dir_all(&home);
    }
}

//! Imports the SQLite database from earlier versions (up to 0.3) the first
//! time the app starts with file-based data (back when it was called Prism). Built with the `legacy-db`
//! feature; once nobody is coming from an old version any more, this module
//! goes away along with `rusqlite`.

use std::path::{Path, PathBuf};

use anyhow::Context;
use rusqlite::{Connection, OpenFlags};

use super::{HistoryEntry, Profile, Snippet};

pub struct Legacy {
    pub profiles: Vec<Profile>,
    pub snippets: Vec<Snippet>,
    /// Oldest to newest.
    pub history: Vec<HistoryEntry>,
}

/// Where earlier versions kept the database: the `dirs` data directory
/// (`~/Library/Application Support/prism` on macOS) and, before that, the web
/// version at `~/.web-terminal.db`.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = dirs::data_dir() {
        out.push(d.join("prism").join("prism.db"));
    }
    if let Some(h) = dirs::home_dir() {
        out.push(h.join(".web-terminal.db"));
    }
    out
}

pub fn read(path: &Path) -> anyhow::Result<Legacy> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("cannot open {}", path.display()))?;

    let profiles = conn
        .prepare("SELECT name, shell, cwd FROM profiles ORDER BY id")?
        .query_map([], |r| {
            Ok(Profile {
                name: r.get(0)?,
                shell: r.get(1)?,
                cwd: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let snippets = conn
        .prepare("SELECT label, command FROM snippets ORDER BY id")?
        .query_map([], |r| {
            Ok(Snippet {
                label: r.get(0)?,
                command: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Very old databases lack the cwd column: entries then have no directory.
    let has_cwd = conn.prepare("SELECT cwd FROM history LIMIT 0").is_ok();
    let cwd_col = if has_cwd { "h.cwd" } else { "NULL" };
    let history = conn
        .prepare(&format!(
            "SELECT h.command, h.ts, {cwd_col}, p.name
             FROM history h LEFT JOIN profiles p ON p.id = h.profile_id
             ORDER BY h.id"
        ))?
        .query_map([], |r| {
            Ok(HistoryEntry {
                command: r.get(0)?,
                ts: r.get(1)?,
                cwd: r.get(2)?,
                profile: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(Legacy {
        profiles,
        snippets,
        history,
    })
}

/// Moves the imported database aside (`prism.db` → `prism.db.bak`). The web
/// version's one is left alone: it was never ours.
pub fn retire(path: &Path) {
    if path.file_name().is_some_and(|n| n == "prism.db") {
        let _ = std::fs::rename(path, path.with_extension("db.bak"));
    }
}

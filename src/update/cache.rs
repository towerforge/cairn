//! A note on disk with what the last check found, so the startup check asks
//! GitHub once a day and not on every launch.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Client, Version};

const FILE: &str = "update-check.json";
/// How long a check is good for. A release is not news by the minute.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckCache {
    pub checked_at: DateTime<Utc>,
    /// Newest version GitHub had at that moment.
    pub latest: String,
}

impl CheckCache {
    pub fn new(latest: &Version) -> Self {
        Self {
            checked_at: Utc::now(),
            latest: latest.to_string(),
        }
    }

    fn path(state_dir: &Path) -> PathBuf {
        state_dir.join(FILE)
    }

    /// Reads the note. Anything wrong with it (missing, unreadable, an older
    /// format) simply means there is no note.
    pub fn load(state_dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(state_dir)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, state_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(state_dir)?;
        let text = serde_json::to_string(self).unwrap_or_default();
        std::fs::write(Self::path(state_dir), text)
    }

    /// Forgets the check. Done right after installing, so the notice does not
    /// survive the update that answered it.
    pub fn forget(state_dir: &Path) {
        let _ = std::fs::remove_file(Self::path(state_dir));
    }

    pub fn fresh_at(&self, now: DateTime<Utc>, ttl: Duration) -> bool {
        match (now - self.checked_at).to_std() {
            Ok(age) => age < ttl,
            // Checked in the future: the clock moved, do not trust it.
            Err(_) => false,
        }
    }

    pub fn version(&self) -> Option<Version> {
        self.latest.parse().ok()
    }
}

/// The published version that beats `current`, if any: from the note while it
/// is fresh, from GitHub when it is not. Every failure is a `None`; nobody is
/// told that the check could not be made.
pub fn newer_than(current: &Version, state_dir: &Path) -> Option<Version> {
    if let Some(c) = CheckCache::load(state_dir)
        && c.fresh_at(Utc::now(), TTL)
    {
        return c.version().filter(|v| v > current);
    }
    let latest = Client::new().ok()?.latest().ok()?.version;
    let _ = CheckCache::new(&latest).save(state_dir);
    (latest > *current).then_some(latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cairn-update-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_note_round_trips_and_is_forgotten() {
        let dir = scratch("note");
        assert!(CheckCache::load(&dir).is_none());
        CheckCache::new(&"0.4.0".parse().unwrap())
            .save(&dir)
            .unwrap();
        let back = CheckCache::load(&dir).unwrap();
        assert_eq!(back.version(), Some("0.4.0".parse().unwrap()));
        assert!(back.fresh_at(Utc::now(), TTL));
        CheckCache::forget(&dir);
        assert!(CheckCache::load(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_or_future_notes_are_not_fresh() {
        let at = |h: i64| CheckCache {
            checked_at: Utc::now() + chrono::Duration::hours(h),
            latest: "0.4.0".into(),
        };
        assert!(!at(-25).fresh_at(Utc::now(), TTL));
        assert!(!at(2).fresh_at(Utc::now(), TTL));
        assert!(at(-1).fresh_at(Utc::now(), TTL));
    }

    #[test]
    fn a_fresh_note_answers_without_the_network() {
        let dir = scratch("offline");
        CheckCache::new(&"9.0.0".parse().unwrap())
            .save(&dir)
            .unwrap();
        let current: Version = "0.3.0".parse().unwrap();
        assert_eq!(newer_than(&current, &dir), Some("9.0.0".parse().unwrap()));
        let newest: Version = "9.0.0".parse().unwrap();
        assert_eq!(newer_than(&newest, &dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

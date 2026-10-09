//! Updating Cairn in place from its GitHub releases: what was published,
//! whether it is newer than what is running, and the download that replaces
//! it. The same steps serve `cairn update` (`cli`) and `/update` inside the
//! app, which runs them on a thread and reports through `AppEvent::Update`.

mod archive;
pub mod cache;
pub mod cli;
pub mod install;
pub mod target;
pub mod version;

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;

use crate::event::{AppEvent, EventSink};
pub use install::{InstallKind, Layout};
pub use target::Target;
pub use version::Version;

/// Where the releases are published. The same repository `install.sh` reads.
pub const REPO: &str = "towerforge/cairn";
const API: &str = "https://api.github.com/repos";
const CHECKSUMS: &str = "checksums.txt";
const UA: &str = concat!("cairn/", env!("CARGO_PKG_VERSION"));
/// Ceiling for a downloaded archive; a release is a few megabytes.
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

/// What the update threads tell the app.
pub enum Notice {
    /// The startup check found a newer release.
    Available(Version),
    /// `/update` progress, for the message line.
    Progress(String),
    /// `/update` finished: what to tell the user, or why it failed.
    Done(Result<String, String>),
}

/// A published release.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    pub notes: String,
    pub url: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// The first lines of the release notes, as bullets. GitHub writes
    /// `* title by @who in <url>`.
    pub fn highlights(&self, max: usize) -> Vec<String> {
        self.notes
            .lines()
            .map(str::trim)
            .filter_map(|l| l.strip_prefix("* ").or_else(|| l.strip_prefix("- ")))
            .map(|l| match l.find(" by @") {
                Some(i) => l[..i].trim(),
                None => l,
            })
            .filter(|l| !l.is_empty() && !l.starts_with("**Full Changelog**"))
            .map(str::to_string)
            .take(max)
            .collect()
    }
}

#[derive(Deserialize)]
struct WireRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    published_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    assets: Vec<WireAsset>,
}

#[derive(Deserialize)]
struct WireAsset {
    name: String,
    browser_download_url: String,
}

/// Talks to the GitHub releases API. Anonymous: 60 calls an hour per address,
/// which the daily check keeps well clear of.
pub struct Client {
    agent: ureq::Agent,
}

impl Client {
    pub fn new() -> anyhow::Result<Self> {
        let agent = ureq::Agent::config_builder()
            .user_agent(UA)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_recv_response(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self { agent })
    }

    /// The latest published release.
    pub fn latest(&self) -> anyhow::Result<Release> {
        self.release_at(&format!("{API}/{REPO}/releases/latest"))
            .map_err(|e| match e.downcast_ref::<NotFound>() {
                Some(_) => anyhow::anyhow!(
                    "no release published yet at https://github.com/{REPO}/releases"
                ),
                None => e,
            })
    }

    /// A specific version, by its tag.
    pub fn release(&self, v: &Version) -> anyhow::Result<Release> {
        self.release_at(&format!("{API}/{REPO}/releases/tags/{}", v.tag()))
            .map_err(|e| match e.downcast_ref::<NotFound>() {
                Some(_) => anyhow::anyhow!("there is no release {} to install", v.tag()),
                None => e,
            })
    }

    fn release_at(&self, url: &str) -> anyhow::Result<Release> {
        let mut resp = self
            .agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(http_error)?;
        match resp.status().as_u16() {
            200..=299 => {}
            404 => return Err(NotFound.into()),
            403 | 429 => anyhow::bail!("github: rate limit reached, try again in a while"),
            s => anyhow::bail!("github: HTTP {s}"),
        }
        let w: WireRelease = serde_json::from_reader(resp.body_mut().as_reader())
            .context("github: unexpected answer")?;
        Ok(Release {
            version: w.tag_name.parse()?,
            notes: w.body.unwrap_or_default(),
            url: w.html_url,
            published_at: w.published_at,
            assets: w
                .assets
                .into_iter()
                .map(|a| Asset {
                    name: a.name,
                    url: a.browser_download_url,
                })
                .collect(),
        })
    }

    /// Text of an asset, for `checksums.txt`.
    pub fn text(&self, url: &str) -> anyhow::Result<String> {
        let mut resp = self.agent.get(url).call().map_err(http_error)?;
        ensure_ok(resp.status().as_u16(), url)?;
        Ok(resp.body_mut().read_to_string()?)
    }

    /// Downloads an asset into memory, reporting bytes so far and the total.
    pub fn download(
        &self,
        url: &str,
        mut on_progress: impl FnMut(u64, Option<u64>),
    ) -> anyhow::Result<Vec<u8>> {
        let mut resp = self.agent.get(url).call().map_err(http_error)?;
        ensure_ok(resp.status().as_u16(), url)?;
        let total = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok()?.parse().ok());
        let mut reader = resp.body_mut().with_config().limit(MAX_DOWNLOAD).reader();
        let mut out = Vec::with_capacity(total.unwrap_or(1 << 20) as usize);
        let mut buf = [0u8; 64 * 1024];
        on_progress(0, total);
        loop {
            let n = reader.read(&mut buf).context("download interrupted")?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            on_progress(out.len() as u64, total);
        }
        Ok(out)
    }
}

#[derive(Debug)]
struct NotFound;

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not found")
    }
}

impl std::error::Error for NotFound {}

fn http_error(e: ureq::Error) -> anyhow::Error {
    anyhow::anyhow!("github: {e} (no connection?)")
}

fn ensure_ok(status: u16, url: &str) -> anyhow::Result<()> {
    anyhow::ensure!((200..300).contains(&status), "HTTP {status} for {url}");
    Ok(())
}

/// Downloads the release for `target`, checks it against `checksums.txt` and
/// puts it in place of `layout`. Returns whether the checksum was verified.
pub fn apply(
    client: &Client,
    release: &Release,
    target: &Target,
    layout: &Layout,
    on_progress: impl FnMut(u64, Option<u64>),
) -> anyhow::Result<bool> {
    let name = target.asset_name();
    let asset = release
        .asset(&name)
        .with_context(|| format!("release v{} has no {name}", release.version))?;
    let bytes = client.download(&asset.url, on_progress)?;
    let verified = match release.asset(CHECKSUMS) {
        Some(sums) => archive::verify_sha256(&bytes, &client.text(&sums.url)?, &name)?,
        None => false,
    };
    layout.install(&bytes, target)?;
    Ok(verified)
}

/// At startup, on a thread: is there a newer release? Only for a release
/// install (a local build would be told forever) and at most once a day.
pub fn spawn_startup_check(sink: EventSink, state_dir: PathBuf) {
    let Ok(layout) = Layout::current() else {
        return;
    };
    if layout.kind() != InstallKind::Release {
        return;
    }
    std::thread::spawn(move || {
        if let Some(v) = cache::newer_than(&Version::current(), &state_dir) {
            sink(AppEvent::Update(Notice::Available(v)));
        }
    });
}

/// `/update`: the latest release, installed from a thread. The running app
/// keeps going; the new version is there from the next launch.
pub fn spawn_update(sink: EventSink, state_dir: PathBuf) {
    std::thread::spawn(move || {
        let result = update_to_latest(&sink).map_err(|e| format!("update failed: {e:#}"));
        if result.is_ok() {
            cache::CheckCache::forget(&state_dir);
        }
        sink(AppEvent::Update(Notice::Done(result)));
    });
}

fn update_to_latest(sink: &EventSink) -> anyhow::Result<String> {
    let layout = Layout::current()?;
    if let Some(why) = layout.kind().refusal() {
        anyhow::bail!(why);
    }
    let current = Version::current();
    let client = Client::new()?;
    let release = client.latest()?;
    if release.version <= current {
        return Ok(format!("Cairn is up to date (v{current})"));
    }
    layout.check_writable()?;
    let target = Target::current()?;
    let mut shown = None;
    let v = release.version.clone();
    apply(&client, &release, &target, &layout, |done, total| {
        let pct = total.filter(|t| *t > 0).map(|t| done * 100 / t);
        // One message per ten percent: enough to see it move.
        let step = pct.map(|p| p / 10);
        if step != shown {
            shown = step;
            let text = match pct {
                Some(p) => format!("downloading Cairn v{v} · {p}%"),
                None => format!("downloading Cairn v{v}…"),
            };
            sink(AppEvent::Update(Notice::Progress(text)));
        }
    })?;
    Ok(format!(
        "Cairn v{} installed · restart Cairn to use it",
        release.version
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notes_are_summarized_into_bullets() {
        let release = |notes: &str| Release {
            version: "0.4.0".parse().unwrap(),
            notes: notes.into(),
            url: String::new(),
            published_at: None,
            assets: Vec::new(),
        };
        let notes = "## What's Changed\n\
             * feat: self-update by @towerforge in https://github.com/x/pull/3\n\
             - fix: narrow windows\n\
             \n\
             **Full Changelog**: https://github.com/x/compare/v0.3.0...v0.4.0\n";
        assert_eq!(
            release(notes).highlights(5),
            vec!["feat: self-update", "fix: narrow windows"]
        );
        assert_eq!(release(notes).highlights(1).len(), 1);
        assert!(release("").highlights(5).is_empty());
    }
}

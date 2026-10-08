//! The platform the running binary was built for, and the release asset that
//! matches it. The names are the ones `make package` writes and `install.sh`
//! downloads: `cairn-<os>-<arch>.tar.gz`.
//!
//! What is inside differs: on macOS the whole `Cairn.app`, on Linux the bare
//! binary next to its launcher and icon.

use std::fmt;

pub const APP: &str = "cairn";
/// The bundle inside the macOS archives.
pub const BUNDLE: &str = "Cairn.app";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub os: &'static str,
    pub arch: &'static str,
}

impl Target {
    pub const fn new(os: &'static str, arch: &'static str) -> Self {
        Self { os, arch }
    }

    /// What this binary was compiled for.
    pub fn current() -> anyhow::Result<Self> {
        let os = match std::env::consts::OS {
            "macos" => "macos",
            "linux" => "linux",
            other => anyhow::bail!("no prebuilt Cairn for {other}"),
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            other => anyhow::bail!("no prebuilt Cairn for {os} {other}"),
        };
        Ok(Self::new(os, arch))
    }

    pub fn is_macos(&self) -> bool {
        self.os == "macos"
    }

    /// Name of the release asset for this platform.
    pub fn asset_name(&self) -> String {
        format!("{APP}-{}-{}.tar.gz", self.os, self.arch)
    }

    /// Where the binary sits inside that asset.
    pub fn binary_path(&self) -> String {
        if self.is_macos() {
            format!("{BUNDLE}/Contents/MacOS/{APP}")
        } else {
            APP.to_string()
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.os, self.arch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_release_assets() {
        let mac = Target::new("macos", "aarch64");
        assert_eq!(mac.asset_name(), "cairn-macos-aarch64.tar.gz");
        assert_eq!(mac.binary_path(), "Cairn.app/Contents/MacOS/cairn");
        let linux = Target::new("linux", "x86_64");
        assert_eq!(linux.asset_name(), "cairn-linux-x86_64.tar.gz");
        assert_eq!(linux.binary_path(), "cairn");
    }

    #[test]
    fn this_platform_is_supported() {
        let t = Target::current().expect("Cairn builds for this platform");
        assert!(t.asset_name().starts_with("cairn-"));
    }
}

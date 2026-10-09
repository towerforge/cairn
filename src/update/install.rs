//! Putting the new release in the place of the one that is running. Nothing
//! is overwritten in place: the new copy is written next to the old one and a
//! rename swaps it in, so a failure half way leaves the old one untouched.
//!
//! On macOS the unit is the whole `Cairn.app`: replacing only the binary
//! inside would break the bundle's signature.

use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context;

use super::Target;
use super::archive;

/// Where the running binary came from. Only a release install is ours to
/// replace: the other two belong to cargo or to the build directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Installed by `install.sh`, from a release archive or by hand.
    Release,
    /// `cargo install`: cargo owns it and `cargo install` updates it.
    Cargo,
    /// Under `target/` (`make run`, `make app`): a build, not an installation.
    Dev,
}

impl InstallKind {
    /// Why the update stops because of where Cairn lives, and the command
    /// that updates it instead.
    pub fn refusal(self) -> Option<String> {
        match self {
            InstallKind::Release => None,
            InstallKind::Cargo => Some(
                "this Cairn came from cargo: update it with \
                 `cargo install --git https://github.com/towerforge/cairn`"
                    .into(),
            ),
            InstallKind::Dev => Some("this Cairn is a local build: rebuild it with `make`".into()),
        }
    }
}

/// What gets replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    /// macOS: the `.app` the binary lives in.
    Bundle(PathBuf),
    /// A bare binary (Linux, or a macOS binary copied out of its bundle).
    Binary(PathBuf),
}

impl Layout {
    /// For the binary that is running, with symlinks resolved so the update
    /// lands on the real file: `~/.local/bin/cairn` → `/Applications/Cairn.app`.
    pub fn current() -> anyhow::Result<Self> {
        let exe = std::env::current_exe().context("cannot locate the running binary")?;
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        Ok(Self::of(&exe))
    }

    pub fn of(exe: &Path) -> Self {
        // …/Cairn.app/Contents/MacOS/cairn
        let bundle = exe
            .parent()
            .filter(|p| p.ends_with("Contents/MacOS"))
            .and_then(|p| p.parent()?.parent())
            .filter(|p| p.extension().is_some_and(|e| e == "app"));
        match bundle {
            Some(app) => Layout::Bundle(app.to_path_buf()),
            None => Layout::Binary(exe.to_path_buf()),
        }
    }

    /// What the user knows it by.
    pub fn path(&self) -> &Path {
        match self {
            Layout::Bundle(p) | Layout::Binary(p) => p,
        }
    }

    /// Where the binary is: for `classify`.
    fn exe(&self) -> PathBuf {
        match self {
            Layout::Bundle(app) => app.join("Contents/MacOS").join(super::target::APP),
            Layout::Binary(p) => p.clone(),
        }
    }

    pub fn kind(&self) -> InstallKind {
        classify(&self.exe())
    }

    /// Whether it can be replaced, asked before anything is downloaded: the
    /// swap writes in the directory, not into the file.
    pub fn check_writable(&self) -> anyhow::Result<()> {
        let dir = parent(self.path());
        let probe = dir.join(format!(".cairn-update-probe-{}", std::process::id()));
        match std::fs::write(&probe, b"") {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => anyhow::bail!(
                "cannot write to {}: run `sudo cairn update`, or reinstall with install.sh",
                dir.display()
            ),
            Err(e) => Err(e).with_context(|| format!("cannot write to {}", dir.display())),
        }
    }

    /// Replaces it with what the release archive carries.
    pub fn install(&self, archive_bytes: &[u8], target: &Target) -> anyhow::Result<()> {
        match self {
            Layout::Binary(dest) => {
                let bin = archive::extract_file(archive_bytes, &target.binary_path())?;
                install_binary(&bin, dest)
            }
            Layout::Bundle(app) => install_bundle(archive_bytes, app),
        }
    }
}

pub fn classify(exe: &Path) -> InstallKind {
    let parts: Vec<_> = exe.iter().map(|p| p.to_string_lossy()).collect();
    let under_target = parts
        .windows(2)
        .any(|w| w[0] == "target" && (w[1] == "debug" || w[1] == "release"))
        || parts.windows(3).any(|w| {
            // target/<triple>/release
            w[0] == "target" && (w[2] == "debug" || w[2] == "release")
        });
    // `make app` builds the bundle in the checkout's dist/.
    let in_dist = parts
        .windows(2)
        .any(|w| w[0] == "dist" && w[1].ends_with(".app"));
    if under_target || in_dist {
        return InstallKind::Dev;
    }
    let cargo_bin = parts
        .windows(2)
        .any(|w| (w[0] == ".cargo" || w[0] == "cargo") && w[1] == "bin");
    if cargo_bin {
        return InstallKind::Cargo;
    }
    InstallKind::Release
}

fn parent(p: &Path) -> &Path {
    p.parent().unwrap_or(Path::new("."))
}

/// Writes `bytes` over `dest` through a temporary file in the same directory.
fn install_binary(bytes: &[u8], dest: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cairn".into());
    let tmp = parent(dest).join(format!(".{name}.new-{}", std::process::id()));
    let write = std::fs::write(&tmp, bytes)
        .and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)))
        .and_then(|_| std::fs::rename(&tmp, dest));
    if write.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write.with_context(|| format!("cannot write {}", dest.display()))
}

/// Unpacks the archive next to `app` and swaps the bundles. The running
/// process keeps its open files, so it goes on working until it is restarted.
fn install_bundle(archive_bytes: &[u8], app: &Path) -> anyhow::Result<()> {
    let dir = parent(app);
    let pid = std::process::id();
    let staging = dir.join(format!(".cairn-update-{pid}"));
    let old = dir.join(format!(".cairn-old-{pid}.app"));
    let _ = std::fs::remove_dir_all(&staging);

    let result = (|| {
        archive::unpack(archive_bytes, &staging)?;
        let new_app = staging.join(super::target::BUNDLE);
        anyhow::ensure!(
            new_app
                .join("Contents/MacOS")
                .join(super::target::APP)
                .is_file(),
            "the archive does not contain {}",
            super::target::BUNDLE
        );
        std::fs::rename(app, &old).with_context(|| format!("cannot move {}", app.display()))?;
        if let Err(e) = std::fs::rename(&new_app, app) {
            let _ = std::fs::rename(&old, app);
            return Err(e).with_context(|| format!("cannot write {}", app.display()));
        }
        Ok(())
    })();

    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&old);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::archive::tests::tar_gz;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cairn-install-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn leftovers(dir: &Path, keep: &str) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != keep)
            .collect()
    }

    #[test]
    fn tells_where_cairn_came_from() {
        let kind = |p: &str| classify(Path::new(p));
        assert_eq!(kind("/usr/local/bin/cairn"), InstallKind::Release);
        assert_eq!(kind("/home/j/.local/bin/cairn"), InstallKind::Release);
        assert_eq!(
            kind("/Applications/Cairn.app/Contents/MacOS/cairn"),
            InstallKind::Release
        );
        assert_eq!(kind("/home/j/.cargo/bin/cairn"), InstallKind::Cargo);
        assert_eq!(kind("/src/cairn/target/release/cairn"), InstallKind::Dev);
        assert_eq!(
            kind("/src/cairn/target/aarch64-apple-darwin/release/cairn"),
            InstallKind::Dev
        );
        assert_eq!(
            kind("/src/cairn/dist/Cairn.app/Contents/MacOS/cairn"),
            InstallKind::Dev
        );
        // A directory called target that is not a build directory.
        assert_eq!(kind("/opt/target/bin/cairn"), InstallKind::Release);
    }

    #[test]
    fn finds_the_bundle_around_the_binary() {
        assert_eq!(
            Layout::of(Path::new("/Applications/Cairn.app/Contents/MacOS/cairn")),
            Layout::Bundle("/Applications/Cairn.app".into())
        );
        assert_eq!(
            Layout::of(Path::new("/home/j/.local/bin/cairn")),
            Layout::Binary("/home/j/.local/bin/cairn".into())
        );
    }

    #[test]
    fn swaps_a_bare_binary_and_keeps_it_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("binary");
        let dest = dir.join("cairn");
        std::fs::write(&dest, b"old").unwrap();
        let layout = Layout::Binary(dest.clone());
        layout.check_writable().unwrap();
        let archive = tar_gz(&[("cairn", b"new")]);
        layout
            .install(&archive, &Target::new("linux", "x86_64"))
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "has to stay executable");
        assert!(leftovers(&dir, "cairn").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swaps_the_whole_bundle() {
        let dir = scratch("bundle");
        let app = dir.join("Cairn.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/cairn"), b"old").unwrap();
        std::fs::write(app.join("Contents/stale"), b"gone").unwrap();
        let archive = tar_gz(&[
            ("Cairn.app/Contents/MacOS/cairn", b"new"),
            ("Cairn.app/Contents/Info.plist", b"<plist/>"),
        ]);
        Layout::Bundle(app.clone())
            .install(&archive, &Target::new("macos", "aarch64"))
            .unwrap();
        assert_eq!(
            std::fs::read(app.join("Contents/MacOS/cairn")).unwrap(),
            b"new"
        );
        assert!(app.join("Contents/Info.plist").exists());
        assert!(
            !app.join("Contents/stale").exists(),
            "the old bundle goes whole"
        );
        assert!(leftovers(&dir, "Cairn.app").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_archive_leaves_the_bundle_as_it_was() {
        let dir = scratch("bad");
        let app = dir.join("Cairn.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/cairn"), b"old").unwrap();
        let archive = tar_gz(&[("README", b"no bundle here")]);
        assert!(
            Layout::Bundle(app.clone())
                .install(&archive, &Target::new("macos", "aarch64"))
                .is_err()
        );
        assert_eq!(
            std::fs::read(app.join("Contents/MacOS/cairn")).unwrap(),
            b"old"
        );
        assert!(leftovers(&dir, "Cairn.app").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

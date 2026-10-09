//! The `.tar.gz` a release ships and the `checksums.txt` next to it.

use std::io::Read;
use std::path::{Component, Path};

use anyhow::Context;
use sha2::{Digest, Sha256};

/// SHA-256 of the archive against its line in `checksums.txt`. `Ok(false)`
/// when the file has no line for it: `install.sh` warns and goes on, and so
/// does this.
pub fn verify_sha256(bytes: &[u8], checksums: &str, asset: &str) -> anyhow::Result<bool> {
    let Some(expected) = checksums
        .lines()
        .filter_map(|l| l.split_once("  ").or_else(|| l.split_once(' ')))
        .find(|(_, name)| name.trim().trim_start_matches('*') == asset)
        .map(|(hash, _)| hash.trim().to_lowercase())
    else {
        return Ok(false);
    };
    let got = format!("{:x}", Sha256::digest(bytes));
    anyhow::ensure!(
        got == expected,
        "checksum mismatch for {asset}: expected {expected}, got {got}"
    );
    Ok(true)
}

/// One file of the archive, by its path inside it (`./` ignored).
pub fn extract_file(bytes: &[u8], wanted: &str) -> anyhow::Result<Vec<u8>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for entry in archive.entries().context("unreadable archive")? {
        let mut entry = entry.context("unreadable archive")?;
        let path = entry.path().context("unreadable archive")?.into_owned();
        if normalize(&path) == wanted {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).context("unreadable archive")?;
            anyhow::ensure!(!buf.is_empty(), "{wanted} is empty in the archive");
            return Ok(buf);
        }
    }
    anyhow::bail!("the archive does not contain {wanted}")
}

/// The whole archive into `dest`. The tar crate refuses entries that would
/// land outside it.
pub fn unpack(bytes: &[u8], dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("cannot create {}", dest.display()))?;
    tar::Archive::new(flate2::read::GzDecoder::new(bytes))
        .unpack(dest)
        .with_context(|| format!("cannot unpack into {}", dest.display()))
}

fn normalize(path: &Path) -> String {
    path.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `.tar.gz` with these files, as `make package` would write it.
    pub(crate) fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (path, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, path, *data).unwrap();
        }
        let raw = tar.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &raw).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn the_checksum_matches_or_nothing_installs() {
        let data = b"cairn";
        let hash = format!("{:x}", Sha256::digest(data));
        let file =
            format!("{hash}  cairn-macos-aarch64.tar.gz\nother  cairn-linux-x86_64.tar.gz\n");
        assert!(verify_sha256(data, &file, "cairn-macos-aarch64.tar.gz").unwrap());
        // No line for this asset: nothing to check against, not an error.
        assert!(!verify_sha256(data, &file, "cairn-linux-aarch64.tar.gz").unwrap());
        let wrong = format!("{}  cairn-macos-aarch64.tar.gz\n", "0".repeat(64));
        assert!(verify_sha256(data, &wrong, "cairn-macos-aarch64.tar.gz").is_err());
    }

    #[test]
    fn pulls_one_file_out() {
        let archive = tar_gz(&[
            ("./cairn", b"ELF!"),
            ("share/applications/cairn.desktop", b"[Desktop Entry]"),
        ]);
        assert_eq!(extract_file(&archive, "cairn").unwrap(), b"ELF!");
        assert!(extract_file(&archive, "missing").is_err());
    }
}

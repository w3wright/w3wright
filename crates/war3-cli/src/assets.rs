//! Asset access backed by the game's own MPQ archives.
//!
//! The core crate cannot do this itself: it must not depend on the MPQ reader,
//! so wrapping an archive in an asset source is the caller's job. This is that
//! caller.
//!
//! # Why it is needed
//!
//! Most of the game's metadata and trigger definitions — `UI\TriggerData.txt`,
//! `Units\UnitMetaData.slk`, `UI\WorldEditStrings.txt` and so on — are **packed
//! inside `war3.mpq`, `War3x.mpq` and friends** rather than present as files on
//! disk. Scanning a directory tree alone therefore reports them missing on almost
//! every installation, which reads as "the game is not installed" when the real
//! situation is "its data has not been unpacked".
//!
//! Archives are queried in a fixed order and the first hit wins.

use std::path::{Path, PathBuf};

use war3_archive::Archive;
use war3_core::assets::normalize_asset_path;
use war3_core::AssetSource;

/// Archives to try, in order.
///
/// The ordering follows the game's own override precedence: the base archive
/// first, then the expansion and its localisation, then patches.
const ARCHIVE_NAMES: &[&str] = &[
    "war3.mpq",
    "War3x.mpq",
    "War3xLocal.mpq",
    "War3Patch.mpq",
    "war3local.mpq",
];

/// A chain of MPQ archives queried in order.
#[derive(Debug)]
pub struct MpqAssetSource {
    archives: Vec<Archive>,
    /// Archives that could not be opened, with the reason.
    failed: Vec<(PathBuf, String)>,
}

impl MpqAssetSource {
    /// Opens every archive it can find under `root`.
    ///
    /// An archive that will not open is recorded and skipped: one damaged
    /// localisation pack should not take the whole chain down.
    #[must_use]
    pub fn open(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        let mut archives = Vec::new();
        let mut failed = Vec::new();

        for name in ARCHIVE_NAMES {
            let path = root.join(name);
            if !path.is_file() {
                continue;
            }
            match Archive::open(&path) {
                Ok(a) => archives.push(a),
                Err(e) => failed.push((path, e.to_string())),
            }
        }

        Self { archives, failed }
    }

    /// How many archives opened.
    #[must_use]
    pub fn archive_count(&self) -> usize {
        self.archives.len()
    }

    /// The archives that failed, with reasons.
    #[must_use]
    pub fn failures(&self) -> &[(PathBuf, String)] {
        &self.failed
    }

    /// Per archive: its path, how many members it has names for, and how many
    /// block table entries are in use.
    ///
    /// The two counts differing is normal and informative: the format does not
    /// store names, so members that no enumeration source mentions stay
    /// anonymous.
    #[must_use]
    pub fn name_counts(&self) -> Vec<(PathBuf, usize, usize)> {
        self.archives
            .iter()
            .map(|a| {
                (
                    a.path().unwrap_or(Path::new("<memory>")).to_path_buf(),
                    a.file_count(),
                    a.used_block_count(),
                )
            })
            .collect()
    }
}

impl AssetSource for MpqAssetSource {
    fn get(&self, path: &str) -> Option<Vec<u8>> {
        // Normalising first is faster when the name is already in an index, and
        // harmless when it is not: the archive hashes the name itself.
        let wanted = normalize_asset_path(path);
        for archive in &self.archives {
            if let Ok(bytes) = archive.read_file(&wanted) {
                return Some(bytes);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_directory_yields_an_empty_but_usable_source() {
        let src = MpqAssetSource::open("Z:\\definitely-not-here");
        assert_eq!(src.archive_count(), 0);
        assert!(src.failures().is_empty());
        assert!(src.get("UI\\TriggerData.txt").is_none());
    }

    #[test]
    fn archive_names_follow_the_override_order() {
        assert_eq!(ARCHIVE_NAMES[0], "war3.mpq");
        assert!(ARCHIVE_NAMES.contains(&"War3xLocal.mpq"));
    }
}

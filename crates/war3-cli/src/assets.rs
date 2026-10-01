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
use war3_core::{AssetSource, FileAssetSource};

/// Archives in the order the game opens them: lowest priority first.
///
/// **The last archive opened wins.** That is not a detail — every metadata file
/// this tool reads exists in more than one of these archives with *different*
/// contents, so the order decides what the tool sees:
///
/// | File | `war3.mpq` | `War3x.mpq` | `War3Patch.mpq` |
/// | --- | --- | --- | --- |
/// | `Units\UnitMetaData.slk` | 117 rows | 248 rows | 267 rows |
/// | `Units\AbilityMetaData.slk` | absent | 630 rows | 748 rows |
/// | `Doodads\DoodadMetaData.slk` | absent | 62 rows | 35 rows |
/// | `UI\UnitEditorData.txt` | 12 sections | 31 sections | 36 sections |
///
/// The three `UnitMetaData.slk` sets are nested, and the doodad table is not
/// smaller by accident: the patch version replaces ten explicit level rows
/// (`dvr1`…`dvr0`, fields `vertR01`…`vertB10`) with three rows carrying
/// `repeat=10`. Reading the base archive instead therefore misses every
/// expansion field and every level beyond the first.
const ARCHIVE_LOAD_ORDER: &[&str] = &[
    "war3.mpq",
    "War3x.mpq",
    "War3xLocal.mpq",
    "War3Patch.mpq",
    "war3local.mpq",
];

/// Loose files over archives, or the other way round.
///
/// A dedicated type so that the ordering is defined in exactly one place:
/// reversing it would not fail, it would quietly read a different copy. The
/// archives come first because a patch archive overrides the loose tree.
#[derive(Debug)]
pub struct LayeredSource<'a> {
    first: &'a MpqAssetSource,
    second: &'a FileAssetSource,
}

impl<'a> LayeredSource<'a> {
    /// Layers `first` over `second`.
    #[must_use]
    pub const fn new(first: &'a MpqAssetSource, second: &'a FileAssetSource) -> Self {
        Self { first, second }
    }
}

impl AssetSource for LayeredSource<'_> {
    fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.first.get(path).or_else(|| self.second.get(path))
    }
}

/// A chain of MPQ archives queried in priority order.
#[derive(Debug)]
pub struct MpqAssetSource {
    /// Highest priority first, so a lookup is a forward scan with first hit
    /// winning.
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

        for name in ARCHIVE_LOAD_ORDER {
            let path = root.join(name);
            if !path.is_file() {
                continue;
            }
            match Archive::open(&path) {
                Ok(a) => archives.push(a),
                Err(e) => failed.push((path, e.to_string())),
            }
        }
        // Load order is lowest priority first; queries want the opposite.
        archives.reverse();

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
        // Highest priority first: the first archive holding the name answers.
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
    use war3_archive::ArchiveBuilder;

    #[test]
    fn a_missing_directory_yields_an_empty_but_usable_source() {
        let src = MpqAssetSource::open("Z:\\definitely-not-here");
        assert_eq!(src.archive_count(), 0);
        assert!(src.failures().is_empty());
        assert!(src.get("UI\\TriggerData.txt").is_none());
    }

    #[test]
    fn the_load_order_puts_patches_last_and_the_base_first() {
        // The constant is the game's load order, so the base archive comes
        // first and everything opened later overrides it. Reversing this list
        // would silently serve 2003-era metadata.
        assert_eq!(ARCHIVE_LOAD_ORDER[0], "war3.mpq");
        let patch = ARCHIVE_LOAD_ORDER
            .iter()
            .position(|n| *n == "War3Patch.mpq")
            .expect("the patch archive is part of the chain");
        let expansion = ARCHIVE_LOAD_ORDER
            .iter()
            .position(|n| *n == "War3x.mpq")
            .expect("the expansion archive is part of the chain");
        assert!(
            patch > expansion,
            "the patch archive must be opened after the expansion so it wins"
        );
    }

    /// A directory of two archives holding the same name, removed on drop.
    ///
    /// Deliberately dependency-free: the workspace has no temporary-directory
    /// crate, and one small guard is cheaper than adding one.
    struct TempTree(PathBuf);

    impl TempTree {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let dir =
                std::env::temp_dir().join(format!("w3wright-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("the temporary directory is creatable");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn archive(&self, name: &str, member: &str, contents: &[u8]) {
            let mut builder = ArchiveBuilder::new();
            builder.add_stored(member, contents.to_vec());
            builder
                .write(self.0.join(name))
                .expect("the archive is writable");
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_later_archive_overrides_an_earlier_one() {
        let tree = TempTree::new("precedence");
        tree.archive("war3.mpq", "Units\\UnitMetaData.slk", b"base-era");
        tree.archive("War3Patch.mpq", "Units\\UnitMetaData.slk", b"patched");

        let src = MpqAssetSource::open(tree.path());
        assert_eq!(src.archive_count(), 2);
        assert_eq!(
            src.get("Units\\UnitMetaData.slk").unwrap(),
            b"patched",
            "the last archive opened must answer, not the first"
        );
    }

    #[test]
    fn a_name_only_the_low_priority_archive_has_is_still_found() {
        let tree = TempTree::new("fallthrough");
        tree.archive("war3.mpq", "only\\in\\Base.slk", b"base only");
        tree.archive("War3Patch.mpq", "only\\in\\Patch.slk", b"patched only");

        let src = MpqAssetSource::open(tree.path());
        assert_eq!(src.get("only\\in\\Base.slk").unwrap(), b"base only");
        assert_eq!(src.get("only\\in\\Patch.slk").unwrap(), b"patched only");
        assert!(src.get("only\\in\\Neither.slk").is_none());
    }

    #[test]
    fn archives_are_named_case_insensitively_as_the_format_requires() {
        let tree = TempTree::new("case");
        tree.archive("war3.mpq", "UI\\UnitEditorData.txt", b"[attackBits]");

        let src = MpqAssetSource::open(tree.path());
        assert_eq!(src.get("ui/uniteditordata.txt").unwrap(), b"[attackBits]");
    }
}

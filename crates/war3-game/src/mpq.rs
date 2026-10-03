//! The game's own MPQ archives as an [`AssetSource`].
//!
//! # Why this lives in the core rather than in the command line
//!
//! It was written in `crates/war3-cli/src/assets.rs`, and grew into something two callers need:
//! the command line and the editor both have to answer "what is `hfoo` called", and the answer
//! depends entirely on which archive wins. Two copies of that order is exactly the kind of
//! divergence this workspace exists to prevent — the load order decides whether the reader sees
//! RoC-era metadata or the patched one, and the difference is 117 rows against 267.
//!
//! # The order is the game's, and it is load-bearing
//!
//! | File | `war3.mpq` | `War3x.mpq` | `War3Patch.mpq` |
//! | --- | --- | --- | --- |
//! | `Units\UnitMetaData.slk` | 117 rows | 248 rows | **267 rows** |
//! | `Units\AbilityMetaData.slk` | absent | 630 rows | **748 rows** |
//! | `UI\UnitEditorData.txt` | 12 sections | 31 sections | **36 sections** |
//!
//! Reversing it does not fail. It quietly serves 2003-era data, and the symptom is a field or an
//! object that "cannot be found" on a map that has it.

use std::path::{Path, PathBuf};

use war3_archive::Archive;
use war3_core::assets::normalize_asset_path;
use war3_core::{AssetSource, FileAssetSource};

/// Archives in the order the game opens them: lowest priority first.
///
/// **The last archive opened wins.** `War3xLocal.mpq` sits between the expansion and the patch
/// because it holds the localised text — the Chinese unit names on this machine — and the patch
/// must still be able to override it.
pub const ARCHIVE_LOAD_ORDER: &[&str] = &[
    "war3.mpq",
    "War3x.mpq",
    "War3xLocal.mpq",
    "War3Patch.mpq",
    "war3local.mpq",
];

/// Loose files under archives, or the other way round.
///
/// A dedicated type so the ordering is defined in exactly one place: reversing it would not fail,
/// it would quietly read a different copy. The archives win because a patch archive overrides an
/// unpacked tree.
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
    /// Highest priority first, so a lookup is a forward scan with first hit winning.
    archives: Vec<Archive>,
    /// Archives that could not be opened, with the reason.
    failed: Vec<(PathBuf, String)>,
}

impl MpqAssetSource {
    /// Opens every archive it can find under `root`.
    ///
    /// An archive that will not open is recorded and skipped: one damaged localisation pack should
    /// not take the whole chain down.
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

    /// Per archive: its path, how many members it has names for, and how many block table entries
    /// are in use.
    ///
    /// The two counts differing is normal and informative: the format does not store names, so
    /// members that no enumeration source mentions stay anonymous.
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
        // Normalising first is faster when the name is already in an index, and harmless when it
        // is not: the archive hashes the name itself.
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

/// The whole game installation as one asset source: archives over loose files.
///
/// # Why a type and not a function
///
/// The three pieces have different lifetimes and the layered source borrows two of them, so a
/// caller that wants the chain has to hold all three. Handing back one owning value keeps that
/// from being something every caller has to get right.
///
/// # ⚠️ Opening this is the expensive part
///
/// `MpqAssetSource::open` reads each archive's hash and block table. On this machine that is four
/// archives and 17,660 enumerable names, and it is why a caller that resolves names repeatedly
/// should build one of these **once** and keep it, rather than per lookup.
#[derive(Debug)]
pub struct GameAssets {
    /// The archives, highest priority first.
    archives: MpqAssetSource,
    /// The unpacked tree, which the archives override.
    loose: FileAssetSource,
}

impl GameAssets {
    /// Opens a game installation directory.
    ///
    /// A directory that is not one — no archives, no loose files — is not an error here. It
    /// produces a source whose every lookup misses, so a caller with no game installed gets IDs
    /// rather than a failure, which is the same degradation the rest of the core follows.
    #[must_use]
    pub fn open(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            archives: MpqAssetSource::open(root),
            loose: FileAssetSource::new(root),
        }
    }

    /// The archives, for a report that wants to name them.
    #[must_use]
    pub const fn archives(&self) -> &MpqAssetSource {
        &self.archives
    }

    /// The unpacked tree.
    #[must_use]
    pub const fn loose(&self) -> &FileAssetSource {
        &self.loose
    }

    /// A view implementing [`AssetSource`].
    #[must_use]
    pub const fn layered(&self) -> LayeredSource<'_> {
        LayeredSource::new(&self.archives, &self.loose)
    }
}

impl AssetSource for GameAssets {
    fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.archives.get(path).or_else(|| self.loose.get(path))
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
        // The constant is the game's load order, so the base archive comes first and everything
        // opened later overrides it. Reversing this list would silently serve 2003-era metadata.
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
        // And the localisation must sit between them: it carries the translated text, and the
        // patch still has to be able to override it.
        let local = ARCHIVE_LOAD_ORDER
            .iter()
            .position(|n| *n == "War3xLocal.mpq")
            .expect("the localisation archive is part of the chain");
        assert!(
            expansion < local && local < patch,
            "localisation sits in the middle"
        );
    }

    /// A directory of archives, removed on drop.
    ///
    /// Deliberately dependency-free: the workspace has no temporary-directory crate, and one small
    /// guard is cheaper than adding one.
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

    /// The owning source resolves through the same chain, so a caller that wants one value does
    /// not have to know about the layering.
    #[test]
    fn the_owning_source_reaches_both_layers() {
        let tree = TempTree::new("owning");
        tree.archive("war3.mpq", "from\\archive.txt", b"archive");
        std::fs::write(tree.path().join("from-loose.txt"), b"loose").unwrap();

        let assets = GameAssets::open(tree.path());
        assert_eq!(assets.get("from\\archive.txt").unwrap(), b"archive");
        assert_eq!(assets.get("from-loose.txt").unwrap(), b"loose");
        assert_eq!(assets.archives().archive_count(), 1);
    }
}

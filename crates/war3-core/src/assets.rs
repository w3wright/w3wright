//! Injectable source for Blizzard game data.
//!
//! Models, textures, metadata tables and trigger definitions are read from the
//! user's local game installation at runtime. None of it is bundled, because it
//! is Blizzard's content and cannot be redistributed. That is why this crate
//! defines a trait here and never a hard-coded path.
//!
//! When a lookup fails the caller degrades: a missing texture becomes a flat
//! colour, a missing display name becomes a key name or a raw identifier. Both
//! paths must work without ever panicking, so that a user without the game
//! installed can still parse, edit and build maps.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Abstracts "where game data comes from".
///
/// Desktop reads a directory tree, the browser reads user-selected handles,
/// tests read memory.
///
/// # Path convention
///
/// `path` uses MPQ-style `\` separators and is matched case-insensitively,
/// because the game's own file names are inconsistently cased on disk and map
/// authors write whatever casing they like.
pub trait AssetSource: std::fmt::Debug {
    /// Returns a file's bytes, or `None` when absent. Must not panic.
    fn get(&self, path: &str) -> Option<Vec<u8>>;

    /// Whether a file exists. Defaults to a lookup.
    fn has(&self, path: &str) -> bool {
        self.get(path).is_some()
    }

    /// Returns a file decoded as UTF-8, or `None` when absent or not UTF-8.
    fn get_text(&self, path: &str) -> Option<String> {
        let bytes = self.get(path)?;
        let bytes = strip_utf8_bom(&bytes);
        String::from_utf8(bytes.to_vec()).ok()
    }
}

/// Strips a UTF-8 BOM.
///
/// Many of the game's `.txt` and `.slk` files carry one. Leaving it in place
/// makes the *first* key name start with three invisible bytes, which shows up
/// as "the first field will not parse".
#[must_use]
pub fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

/// Normalises a path into a case-insensitive comparison key.
///
/// `/` and `\` are treated as equivalent, runs of separators collapse, and a
/// leading separator is dropped.
#[must_use]
pub fn normalize_asset_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut last_was_sep = true; // swallows a leading separator
    for ch in path.chars() {
        if ch == '/' || ch == '\\' {
            if !last_was_sep {
                out.push('\\');
                last_was_sep = true;
            }
        } else {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            last_was_sep = false;
        }
    }
    out
}

/// An asset source that never returns anything.
///
/// This is a legitimate state, not an error: it is the path taken by users who
/// have not installed the game.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyAssetSource;

impl AssetSource for EmptyAssetSource {
    fn get(&self, _path: &str) -> Option<Vec<u8>> {
        None
    }
}

/// An in-memory asset source, for tests and prebuilt metadata.
///
/// Keys are normalised on insertion, so lookups are case-insensitive.
#[derive(Debug, Default, Clone)]
pub struct MemoryAssetSource {
    files: BTreeMap<String, Vec<u8>>,
}

impl MemoryAssetSource {
    /// Empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a file.
    pub fn insert(&mut self, path: impl AsRef<str>, bytes: impl Into<Vec<u8>>) {
        self.files.insert(normalize_asset_path(path.as_ref()), bytes.into());
    }

    /// Builds a collection in one expression.
    #[must_use]
    pub fn with(mut self, path: impl AsRef<str>, bytes: impl Into<Vec<u8>>) -> Self {
        self.insert(path, bytes);
        self
    }

    /// Number of files.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl AssetSource for MemoryAssetSource {
    fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(&normalize_asset_path(path)).cloned()
    }
}

/// Reads assets from a local directory tree.
///
/// Point it at a game installation that has already been unpacked, or at any
/// directory laid out the same way.
///
/// # Why this does not read MPQ archives
///
/// Reading game archives requires the MPQ reader, which lives in a separate
/// crate that this one must not depend on. Wrapping an archive in an
/// `AssetSource` is the caller's job; see `war3-cli`'s asset layer for one.
#[derive(Debug, Clone)]
pub struct FileAssetSource {
    root: PathBuf,
    /// Built once at construction. The tree is large, but only file names are
    /// indexed, which is far cheaper than hitting the filesystem per lookup.
    index: BTreeMap<String, PathBuf>,
}

impl FileAssetSource {
    /// Indexes a directory tree rooted at `root`.
    ///
    /// Unreadable subdirectories are skipped: an incomplete environment is not
    /// data corruption.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let mut index = BTreeMap::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                match entry.file_type() {
                    Ok(ft) if ft.is_dir() => stack.push(path),
                    Ok(ft) if ft.is_file() => {
                        if let Ok(rel) = path.strip_prefix(&root) {
                            index.insert(normalize_asset_path(&rel.to_string_lossy()), path);
                        }
                    }
                    _ => {}
                }
            }
        }
        Self { root, index }
    }

    /// The root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// How many files were indexed.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.index.len()
    }
}

impl AssetSource for FileAssetSource {
    fn get(&self, path: &str) -> Option<Vec<u8>> {
        let full = self.index.get(&normalize_asset_path(path))?;
        std::fs::read(full).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_is_case_insensitive_and_separator_agnostic() {
        assert_eq!(normalize_asset_path("UI\\WorldEditStrings.txt"), "ui\\worldeditstrings.txt");
        assert_eq!(normalize_asset_path("UI/WorldEditStrings.txt"), "ui\\worldeditstrings.txt");
        assert_eq!(normalize_asset_path("\\UI\\\\TriggerData.txt"), "ui\\triggerdata.txt");
    }

    #[test]
    fn memory_source_lookup_is_case_insensitive() {
        let src = MemoryAssetSource::new().with("UI\\TriggerData.txt", b"hello".to_vec());
        assert!(src.get("ui/triggerdata.txt").is_some());
        assert!(src.get("UI\\TRIGGERDATA.TXT").is_some());
        assert!(src.get("UI\\Nope.txt").is_none());
    }

    #[test]
    fn empty_source_never_panics_and_returns_none() {
        let src = EmptyAssetSource;
        assert!(src.get("anything").is_none());
        assert!(!src.has("anything"));
        assert!(src.get_text("anything").is_none());
    }

    #[test]
    fn bom_is_stripped_from_text() {
        let src = MemoryAssetSource::new()
            .with("a.txt", [&[0xEF, 0xBB, 0xBF][..], b"Key=Value"].concat());
        assert_eq!(src.get_text("a.txt").as_deref(), Some("Key=Value"));
    }

    #[test]
    fn non_utf8_text_returns_none_instead_of_panicking() {
        let src = MemoryAssetSource::new().with("gbk.txt", vec![0xB0, 0xA1, 0xFF]);
        assert!(src.get_text("gbk.txt").is_none());
    }
}

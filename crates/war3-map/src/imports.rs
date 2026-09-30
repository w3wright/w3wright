//! `war3map.imp`: the imported-asset list.
//!
//! This file has two roles. It records which assets a map imports, and — when an
//! archive carries no `(listfile)` — it is the last source of member names. The
//! parsing itself lives in the MPQ crate because name enumeration needs it; this
//! module wraps the result in the semantics the map layer cares about.

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};

/// The import type flags.
///
/// The meanings are not established. Values seen in practice are 0, 8, 10 and 13.
/// They are kept verbatim rather than interpreted, since guessing would lose
/// information on a round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImportFlags(pub u32);

impl ImportFlags {
    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Whether the path is custom rather than the default `war3mapImported\`.
    ///
    /// This is the only bit with any supporting evidence: tools that rebuild
    /// `.imp` files use it to decide whether to write a full path. It remains an
    /// inference, so it must not drive a rewrite of the file.
    #[must_use]
    pub const fn is_custom_path(self) -> bool {
        self.0 & 0x1 != 0
    }
}

/// One import record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// Path inside the archive, as stored. It may be a bare file name or an
    /// explicit `war3mapImported\...` path.
    pub path: String,
    /// Type flags, preserved verbatim.
    pub flags: ImportFlags,
}

impl Import {
    /// Builds a record.
    #[must_use]
    pub fn new(path: impl Into<String>, flags: u32) -> Self {
        Self {
            path: path.into(),
            flags: ImportFlags(flags),
        }
    }

    /// The paths to try for this record, in order.
    ///
    /// A path listed in `.imp` that is not present in the archive is retried with
    /// a `war3mapImported\` prefix.
    #[must_use]
    pub fn candidate_paths(&self) -> Vec<String> {
        let mut out = vec![self.path.clone()];
        let lower = self.path.to_ascii_lowercase();
        if !lower.starts_with("war3mapimported\\") && !lower.starts_with("war3mapimported/") {
            out.push(format!("war3mapImported\\{}", self.path));
        }
        out
    }
}

/// The full import list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportList {
    /// Version field; 1 in every file seen.
    pub version: u32,
    /// The records.
    pub entries: Vec<Import>,
    /// Diagnostics.
    pub diagnostics: Diagnostics,
}

impl ImportList {
    /// Builds a list from raw `(path, flags)` pairs.
    #[must_use]
    pub fn from_entries(version: u32, entries: Vec<(String, u32)>) -> Self {
        Self {
            version,
            entries: entries
                .into_iter()
                .map(|(path, flags)| Import::new(path, flags))
                .collect(),
            diagnostics: Diagnostics::new(),
        }
    }

    /// How many records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every candidate path across all records.
    #[must_use]
    pub fn all_candidate_paths(&self) -> Vec<String> {
        self.entries
            .iter()
            .flat_map(|e| e.candidate_paths())
            .collect()
    }

    /// Reports imports that could not be located in the archive.
    ///
    /// Reported rather than dropped: a missing import changes what the map looks
    /// like when it loads.
    pub fn report_unresolved(&mut self, names: &[String]) {
        for name in names {
            self.diagnostics.push(Diagnostic::warn(
                DiagnosticCode::AssetFallbackUsed,
                format!(
                    "{name:?} is listed in war3map.imp but absent from the archive; both candidate \
                     paths were tried"
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_paths_adds_the_imported_prefix() {
        let import = Import::new("Textures\\a.blp", 0);
        let candidates = import.candidate_paths();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0], "Textures\\a.blp");
        assert_eq!(candidates[1], "war3mapImported\\Textures\\a.blp");
    }

    #[test]
    fn candidate_paths_does_not_double_the_prefix() {
        let import = Import::new("war3mapImported\\a.blp", 0);
        assert_eq!(import.candidate_paths().len(), 1);
        // Different casing still counts as the prefix.
        let import = Import::new("WAR3MAPIMPORTED/a.blp", 0);
        assert_eq!(import.candidate_paths().len(), 1);
    }

    #[test]
    fn flags_are_preserved_verbatim() {
        let import = Import::new("a.blp", 13);
        assert_eq!(import.flags.raw(), 13);
        assert!(import.flags.is_custom_path());
        let plain = Import::new("a.blp", 8);
        assert!(!plain.flags.is_custom_path());
    }

    #[test]
    fn unresolved_imports_are_diagnosed() {
        let mut list = ImportList::from_entries(1, vec![("a.blp".into(), 0)]);
        list.report_unresolved(&["a.blp".to_string()]);
        assert!(list
            .diagnostics
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::AssetFallbackUsed));
    }

    #[test]
    fn all_candidate_paths_expands_every_entry() {
        let list = ImportList::from_entries(
            1,
            vec![("a.blp".into(), 0), ("war3mapImported\\b.blp".into(), 0)],
        );
        assert_eq!(list.len(), 2);
        assert_eq!(list.all_candidate_paths().len(), 3);
    }
}

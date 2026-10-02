//! Diagnostic collection.
//!
//! Parsers report anything that is worth knowing but does not stop parsing by
//! pushing a [`Diagnostic`] here instead of silently skipping it. The CLI
//! prints them at the end.
//!
//! Typical uses:
//!
//! - a version field disagrees with what the rest of the file implies;
//! - an unknown bit or reserved field was preserved verbatim;
//! - a display name degraded to a raw identifier because game data was missing.

use std::fmt;

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Purely informational. Not printed by default.
    Info,
    /// Suspicious, but handled by a documented rule.
    Warning,
    /// The data may already be wrong, though the pipeline can finish.
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Info => "info",
            Self::Warning => "warn",
            Self::Error => "error",
        })
    }
}

/// Stable diagnostic codes.
///
/// An enum rather than bare strings so callers can match on them, tests can
/// assert on specific codes, and renames are caught by the compiler.
///
/// The value returned by [`DiagnosticCode::as_str`] is part of the CLI's output
/// contract; treat renames as breaking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticCode {
    // ---- MPQ ----
    /// The MPQ header is not at offset 0.
    MpqHeaderOffset,
    /// The archive has no usable `(listfile)`.
    MpqNoListfile,
    /// `(listfile)` held bytes that are not UTF-8 and were replaced.
    ///
    /// A listfile written by a non-English World Editor is often in a local
    /// code page, so the names survive but not byte-for-byte.
    MpqListfileNotUtf8,
    /// A member uses an unsupported compression algorithm.
    MpqUnsupportedCompression,
    /// A member uses an unsupported encryption scheme.
    MpqUnsupportedEncryption,
    /// A hash slot's filename hash did not match; the slot was skipped.
    MpqHashCollisionSkipped,

    // ---- terrain (.w3e) ----
    /// The record size implied by the geometry disagrees with the version field.
    W3eRecordSizeMismatch,
    /// Bit 15 of the water word is set; its meaning is unknown.
    W3eUnknownWaterBit,
    /// Bits 2-7 of the v12 second flag byte are set; their meaning is unknown.
    W3eUnknownFlagBits,
    /// A ground texture index exceeds what the version can address.
    W3eTextureIndexOutOfRange,
    /// A water level exceeds the 14 addressable bits.
    W3eWaterLevelOutOfRange,

    // ---- map info (.w3i) ----
    /// The trailing section (players, forces, ...) is missing or truncated.
    W3iMissingTrailingData,
    /// Bytes remain after parsing; this build may be missing a version's field.
    W3iTrailingBytesLeft,
    /// The script language flag is not available in this file version.
    W3iScriptLanguageUnknown,

    // ---- string table (.wts) ----
    /// An entry's value block was never closed.
    WtsUnterminatedEntry,
    /// An entry had no opening brace, so its extent had to be inferred.
    WtsMissingOpenBrace,
    /// A `TRIGSTR_nnn` reference points at an index the table does not have.
    WtsMissingKey,

    // ---- placed units and doodads (.doo) ----
    /// A `.doo` record had a non-zero value in a field the format does not
    /// explain, so a round trip has to preserve it rather than normalise it.
    DooUnknownField,
    /// Bytes were left over after the last `.doo` record.
    DooTrailingBytes,

    // ---- object data (.w3u and friends) ----
    /// A modification's field id is not in the metadata table.
    ///
    /// YDWE drops these silently; here the value is kept and reported, because a
    /// dropped modification is indistinguishable from one that never existed.
    ObjectUnknownField,
    /// A modification's stored type disagrees with the metadata.
    ObjectTypeMismatch,
    /// The file carries more table blocks than the two the format describes,
    /// each of them empty.
    ObjectExtraTable,
    /// Bytes were left over after the custom object table.
    ObjectTrailingBytes,

    // ---- assets ----
    /// A display name degraded to a key name or raw identifier.
    AssetFallbackUsed,

    // ---- source project ----
    /// A member could not be decoded, so its stored block was kept instead.
    ///
    /// This is not a failure: it is the honest form for a member this workspace
    /// has no decompressor for, and the block survives a round trip untouched.
    ProjectMemberKeptRaw,
    /// A member name could not be used as a file name and was percent-encoded.
    ///
    /// The manifest keeps the mapping, so nothing is lost — but the file on disk
    /// is no longer named like the member, which is worth knowing.
    ProjectNameEncoded,
    /// A member has a text form, but it is stored as binary.
    ///
    /// Either producing it failed or it did not reproduce the file byte for byte.
    /// The member is still written correctly; it is just not reviewable as text.
    ProjectMemberKeptBinary,
}

impl DiagnosticCode {
    /// Stable short code for CLI output and CI matching.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MpqHeaderOffset => "mpq.header-offset",
            Self::MpqNoListfile => "mpq.no-listfile",
            Self::MpqListfileNotUtf8 => "mpq.listfile-not-utf8",
            Self::MpqUnsupportedCompression => "mpq.unsupported-compression",
            Self::MpqUnsupportedEncryption => "mpq.unsupported-encryption",
            Self::MpqHashCollisionSkipped => "mpq.hash-collision-skipped",
            Self::W3eRecordSizeMismatch => "w3e.record-size-mismatch",
            Self::W3eUnknownWaterBit => "w3e.unknown-water-bit",
            Self::W3eUnknownFlagBits => "w3e.unknown-flag-bits",
            Self::W3eTextureIndexOutOfRange => "w3e.texture-index-out-of-range",
            Self::W3eWaterLevelOutOfRange => "w3e.water-level-out-of-range",
            Self::W3iMissingTrailingData => "w3i.missing-trailing-data",
            Self::W3iTrailingBytesLeft => "w3i.trailing-bytes-left",
            Self::W3iScriptLanguageUnknown => "w3i.script-language-unknown",
            Self::WtsUnterminatedEntry => "wts.unterminated-entry",
            Self::WtsMissingOpenBrace => "wts.missing-open-brace",
            Self::WtsMissingKey => "wts.missing-key",
            Self::DooUnknownField => "doo.unknown-field",
            Self::DooTrailingBytes => "doo.trailing-bytes",
            Self::ObjectUnknownField => "object.unknown-field",
            Self::ObjectTypeMismatch => "object.type-mismatch",
            Self::ObjectExtraTable => "object.extra-table",
            Self::ObjectTrailingBytes => "object.trailing-bytes",
            Self::AssetFallbackUsed => "asset.fallback-used",
            Self::ProjectMemberKeptRaw => "project.member-kept-raw",
            Self::ProjectNameEncoded => "project.name-encoded",
            Self::ProjectMemberKeptBinary => "project.member-kept-binary",
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// Stable code.
    pub code: DiagnosticCode,
    /// Human-readable message, including the numbers involved.
    pub message: String,
}

impl Diagnostic {
    /// Builds a diagnostic.
    pub fn new(severity: Severity, code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
        }
    }

    /// `Info` level.
    pub fn info(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Info, code, message)
    }

    /// `Warning` level.
    pub fn warn(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    /// `Error` level: parsing can continue, but the data may be wrong.
    pub fn error(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.code, self.message)
    }
}

/// Everything one parse produced.
///
/// Append-only; callers can take ownership and pass it further down.
///
/// Implements `PartialEq` so that structs holding diagnostics (such as the
/// terrain model) can derive it, which round-trip assertions rely on.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one diagnostic.
    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }

    /// Appends several diagnostics.
    pub fn extend(&mut self, other: impl IntoIterator<Item = Diagnostic>) {
        self.items.extend(other);
    }

    /// Absorbs another collection.
    pub fn merge(&mut self, other: &Self) {
        self.items.extend(other.items.iter().cloned());
    }

    /// All diagnostics, in insertion order.
    #[must_use]
    pub fn items(&self) -> &[Diagnostic] {
        &self.items
    }

    /// Whether anything at `Warning` or above was reported.
    #[must_use]
    pub fn has_problems(&self) -> bool {
        self.items.iter().any(|d| d.severity >= Severity::Warning)
    }

    /// Number of `Error`-level diagnostics.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    /// Number of `Warning`-level diagnostics.
    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    /// Whether nothing was reported.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many were reported.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Filters by severity.
    pub fn at_least(&self, severity: Severity) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter().filter(move |d| d.severity >= severity)
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl FromIterator<Diagnostic> for Diagnostics {
    fn from_iter<T: IntoIterator<Item = Diagnostic>>(iter: T) -> Self {
        Self {
            items: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_by_severity() {
        let mut d = Diagnostics::new();
        d.push(Diagnostic::info(
            DiagnosticCode::W3eUnknownWaterBit,
            "bit 15 set",
        ));
        d.push(Diagnostic::warn(
            DiagnosticCode::W3eRecordSizeMismatch,
            "11 vs 8",
        ));
        d.push(Diagnostic::error(
            DiagnosticCode::MpqNoListfile,
            "no listfile",
        ));

        assert_eq!(d.len(), 3);
        assert_eq!(d.warning_count(), 1);
        assert_eq!(d.error_count(), 1);
        assert!(d.has_problems());
    }

    #[test]
    fn info_only_is_not_a_problem() {
        let mut d = Diagnostics::new();
        d.push(Diagnostic::info(
            DiagnosticCode::AssetFallbackUsed,
            "no game data",
        ));
        assert!(!d.has_problems());
    }

    #[test]
    fn codes_are_stable_strings() {
        assert_eq!(
            DiagnosticCode::W3eRecordSizeMismatch.as_str(),
            "w3e.record-size-mismatch"
        );
        assert_eq!(
            DiagnosticCode::MpqHeaderOffset.as_str(),
            "mpq.header-offset"
        );
    }

    #[test]
    fn display_includes_code_and_message() {
        let d = Diagnostic::warn(DiagnosticCode::WtsMissingKey, "TRIGSTR_003 not found");
        let s = d.to_string();
        assert!(s.contains("wts.missing-key"), "{s}");
        assert!(s.contains("TRIGSTR_003"), "{s}");
    }
}

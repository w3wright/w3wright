//! `war3map.wts`: the map's string table.
//!
//! # The format is lossy
//!
//! Three known problems:
//!
//! - a string containing both a comma and a double quote does not survive a write;
//! - a string containing a closing brace does not survive a write;
//! - strings above 256 bytes placed in a binary field crash the game.
//!
//! The usual workaround replaces `}` with `|`.
//!
//! The approach here is therefore:
//!
//! 1. parsing keeps every value byte for byte and applies no "repair";
//! 2. an unterminated value block is reported and taken as ending at EOF, not
//!    guessed at;
//! 3. writing performs only the format's own structure, leaving values alone.
//!
//! Indices start at 0, and references are written as `TRIGSTR_nnn`, three digits
//! zero-padded — though parsing accepts unpadded forms.

use std::collections::BTreeMap;
use std::fmt;

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};

/// A `war3map.wts` string table.
///
/// Entries are keyed by index, so writing is deterministic and suitable for
/// golden-file comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringTable {
    entries: BTreeMap<u32, String>,
    diagnostics: Diagnostics,
}

impl StringTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `war3map.wts` bytes.
    ///
    /// Bytes that are not valid UTF-8 are replaced rather than rejected. This
    /// differs from the strict handling of map info and terrain on purpose: this
    /// table holds text the author typed, older tools may have written it in
    /// another encoding, and one bad character should not make a map unreadable.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Self {
        let text = String::from_utf8_lossy(bytes);
        let mut entries = BTreeMap::new();
        let mut diagnostics = Diagnostics::new();

        // Two states: waiting for `STRING n`, and collecting a value block.
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            let trimmed = line.trim();
            let Some(rest) = trimmed.strip_prefix("STRING") else {
                continue;
            };
            let Ok(index) = rest.trim().parse::<u32>() else {
                continue;
            };

            let mut value_lines: Vec<&str> = Vec::new();
            let mut closed = false;
            for inner in lines.by_ref() {
                if inner.trim() == "}" {
                    closed = true;
                    break;
                }
                if inner.trim() == "{" {
                    continue;
                }
                value_lines.push(inner);
            }

            if !closed {
                diagnostics.push(Diagnostic::warn(
                    DiagnosticCode::WtsUnterminatedEntry,
                    format!(
                        "STRING {index} has no closing brace; the value is taken as running to the \
                         end of the file"
                    ),
                ));
            }
            entries.insert(index, value_lines.join("\n"));
        }

        Self {
            entries,
            diagnostics,
        }
    }

    /// A value by index.
    #[must_use]
    pub fn get(&self, index: u32) -> Option<&str> {
        self.entries.get(&index).map(String::as_str)
    }

    /// Inserts or replaces an entry.
    pub fn insert(&mut self, index: u32, value: impl Into<String>) {
        self.entries.insert(index, value.into());
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All entries, in ascending index order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &str)> {
        self.entries.iter().map(|(k, v)| (*k, v.as_str()))
    }

    /// Diagnostics from parsing.
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Resolves a field value: a `TRIGSTR_nnn` reference is looked up, anything
    /// else is returned unchanged.
    ///
    /// A missing key returns the original string and reports a diagnostic. It
    /// does not return an empty string and does not panic, so the display path
    /// degrades to showing `TRIGSTR_003`.
    #[must_use]
    pub fn resolve(&self, value: &str, diagnostics: &mut Diagnostics) -> String {
        match parse_trigstr(value) {
            Some(index) => match self.get(index) {
                Some(text) => text.to_string(),
                None => {
                    diagnostics.push(Diagnostic::warn(
                        DiagnosticCode::WtsMissingKey,
                        format!(
                            "value references TRIGSTR_{index:03}, which the .wts does not have"
                        ),
                    ));
                    value.to_owned()
                }
            },
            None => value.to_owned(),
        }
    }

    /// Serialises back to `war3map.wts`.
    ///
    /// Each entry is written as `STRING n\r\n{\r\n<value>\r\n}\r\n`, joined with
    /// `\r\n\r\n`.
    #[must_use]
    pub fn to_wts_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut first = true;
        for (index, value) in &self.entries {
            if !first {
                out.extend_from_slice(b"\r\n");
            }
            first = false;
            out.extend_from_slice(format!("STRING {index}\r\n{{\r\n{value}\r\n}}\r\n").as_bytes());
        }
        out
    }
}

impl fmt::Display for StringTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StringTable({} entries)", self.entries.len())
    }
}

/// Parses a `TRIGSTR_nnn` reference.
///
/// Returns `None` when the value is not a reference. Matching is
/// case-insensitive and does not require three digits.
#[must_use]
pub fn parse_trigstr(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    // Seven characters of prefix plus one underscore.
    if trimmed.len() < 8 {
        return None;
    }
    let (prefix, rest) = trimmed.split_at(7);
    if !prefix.eq_ignore_ascii_case("TRIGSTR") || !rest.starts_with('_') {
        return None;
    }
    rest[1..].trim_start().parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_table() {
        let raw = b"STRING 0\r\n{\r\nHello\r\n}\r\n\r\nSTRING 1\r\n{\r\nWorld\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.len(), 2);
        assert_eq!(table.get(0), Some("Hello"));
        assert_eq!(table.get(1), Some("World"));
        assert!(table.diagnostics().is_empty());
    }

    #[test]
    fn multi_line_values_are_joined_with_newlines() {
        let raw = b"STRING 3\r\n{\r\nline one\r\nline two\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.get(3), Some("line one\nline two"));
    }

    #[test]
    fn unterminated_entry_is_diagnosed() {
        let raw = b"STRING 0\r\n{\r\nno closing brace\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.get(0), Some("no closing brace"));
        assert!(table
            .diagnostics()
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::WtsUnterminatedEntry));
    }

    #[test]
    fn parse_trigstr_accepts_case_and_variable_padding() {
        assert_eq!(parse_trigstr("TRIGSTR_003"), Some(3));
        assert_eq!(parse_trigstr("trigstr_1000"), Some(1000));
        assert_eq!(parse_trigstr("TRIGSTR_0"), Some(0));
        assert_eq!(parse_trigstr("not a ref"), None);
        assert_eq!(parse_trigstr("TRIGSTRX_003"), None);
    }

    #[test]
    fn resolve_reports_missing_key_and_keeps_the_raw_value() {
        let table = StringTable::new();
        let mut diags = Diagnostics::new();
        let resolved = table.resolve("TRIGSTR_007", &mut diags);
        assert_eq!(resolved, "TRIGSTR_007");
        assert!(diags
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::WtsMissingKey));
    }

    #[test]
    fn resolve_passes_through_non_references() {
        let table = StringTable::new();
        let mut diags = Diagnostics::new();
        assert_eq!(table.resolve("Just a name", &mut diags), "Just a name");
        assert!(diags.is_empty());
    }

    #[test]
    fn resolve_returns_the_table_value() {
        let mut table = StringTable::new();
        table.insert(5, "Real Text");
        let mut diags = Diagnostics::new();
        assert_eq!(table.resolve("TRIGSTR_005", &mut diags), "Real Text");
        assert!(diags.is_empty());
    }

    #[test]
    fn non_utf8_bytes_degrade_instead_of_failing() {
        let raw = b"STRING 0\r\n{\r\n\xB0\xA1\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert!(table.get(0).is_some());
    }

    #[test]
    fn round_trip_preserves_values() {
        let mut table = StringTable::new();
        table.insert(0, "Alpha");
        table.insert(7, "Beta");
        let bytes = table.to_wts_bytes();
        let reparsed = StringTable::parse(&bytes);
        assert_eq!(reparsed.get(0), Some("Alpha"));
        assert_eq!(reparsed.get(7), Some("Beta"));
    }

    #[test]
    fn serialization_is_deterministic_by_index_order() {
        let mut table = StringTable::new();
        table.insert(9, "nine");
        table.insert(1, "one");
        let bytes = table.to_wts_bytes();
        let text = String::from_utf8(bytes).unwrap();
        let first = text.find("STRING 1").unwrap();
        let second = text.find("STRING 9").unwrap();
        assert!(
            first < second,
            "entries must be written in ascending index order:\n{text}"
        );
    }
}

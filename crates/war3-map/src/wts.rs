//! `war3map.wts`: the map's string table.
//!
//! # The format is lossy
//!
//! Four known problems:
//!
//! - a string containing both a comma and a double quote does not survive a write;
//! - a string containing a closing brace does not survive a write;
//! - strings above 256 bytes placed in a binary field crash the game;
//! - the label comment that generator tools put before each value block (see
//!   below) is not kept, so a rewrite loses it.
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
//! # The lines between the index and the brace are a label, not the value
//!
//! Generator tools annotate every entry with the object it belongs to and the
//! field it fills:
//!
//! ```text
//! STRING 7
//! // 技能: A000 (跳跃2), Name (名字)
//! {
//! 跳跃2
//! }
//! ```
//!
//! That comment sits **before** the opening brace. It reads like part of the
//! block, and folding it into the value is wrong: the value is `跳跃2`, and a
//! tooltip showing the whole comment is a visible defect. The corpus is
//! unambiguous — of 1858 entries across 137 maps, every one carried such a
//! prelude, none of them held anything but comments and blank lines, and not one
//! held a comment *inside* the braces.
//!
//! Lines after the brace are therefore taken verbatim, comments included.
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
            // A BOM before the label must not hide it — see [`trim_bom`]. Without
            // this the first entry of a BOM-prefixed file is silently dropped.
            let trimmed = trim_bom(line).trim();
            let Some(rest) = trimmed.strip_prefix("STRING") else {
                continue;
            };
            let Ok(index) = rest.trim().parse::<u32>() else {
                continue;
            };

            let mut value_lines: Vec<&str> = Vec::new();
            let mut closed = false;
            // Whether the opening brace has been seen, and whether collection has
            // begun. They differ for a brace-less entry, which is malformed but
            // whose text is still the author's.
            let mut saw_brace = false;
            let mut collecting = false;
            // Every comparison in here has to see through a BOM as well: a mark
            // before the brace would make `== "{"` fail and the brace line would be
            // collected as part of the value.
            let clean = |line: &str| trim_bom(line).trim().to_string();
            for inner in lines.by_ref() {
                if clean(inner) == "}" {
                    closed = true;
                    break;
                }
                if !collecting {
                    if clean(inner) == "{" {
                        saw_brace = true;
                        collecting = true;
                        continue;
                    }
                    // Every line before the brace is a label — the generator's
                    // `// 技能: …` annotation — and never part of the value.
                    if clean(inner).is_empty() || clean(inner).starts_with("//") {
                        continue;
                    }
                    collecting = true;
                    value_lines.push(inner);
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
            if !saw_brace {
                diagnostics.push(Diagnostic::warn(
                    DiagnosticCode::WtsMissingOpenBrace,
                    format!(
                        "STRING {index} has no opening brace; its value was taken as starting at \
                         the first line that is neither blank nor a comment"
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
///
/// # Why this does not slice by byte
///
/// The obvious form — `trimmed.split_at(7)` and compare — **panics on any map whose
/// value starts with a non-ASCII character**, because `split_at` counts bytes and 7
/// can land inside a multi-byte character. It aborted the whole command on
/// `澄海3C-AI版.w3x`, whose first bytes are the name `力...`: `end byte index 7 is
/// not a char boundary`.
///
/// `str::get(..7)` is the safe equivalent: it yields `None` instead of panicking when
/// the range is not on a boundary, and the ASCII check that follows is what the
/// comparison wanted anyway. A non-ASCII value is by definition not `TRIGSTR_`, so
/// rejecting it there is correct rather than merely safe.
#[must_use]
pub fn parse_trigstr(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    // Seven characters of prefix, then an underscore.
    let prefix: &str = trimmed.get(..7)?;
    if !prefix.eq_ignore_ascii_case("TRIGSTR") {
        return None;
    }
    let rest = trimmed.get(7..)?.strip_prefix('_')?;
    rest.trim_start().parse::<u32>().ok()
}

/// A line with any leading byte-order marks removed.
///
/// # Why this is not just one `strip_prefix` at the top
///
/// `.wts` files are written by the World Editor and by third-party generators, and a
/// BOM can appear before a `STRING` label anywhere in the file — not only at the
/// start. Any BOM left in place makes the label unparseable, and the entry then
/// **silently disappears** from the table.
///
/// That is not hypothetical: the map `侏罗纪公园1.5.w3x` begins with `EF BB BF`
/// followed by `STRING 1`, which is the entry holding the map's own name. The table
/// parsed 2224 of its 2225 entries and the one it lost was the name, so the map
/// reported `TRIGSTR_001` plus a "the .wts does not have this key" warning — a wrong
/// answer about a file that was fine.
///
/// The strip repeats so that several marks in a row are handled, which is what a
/// generator concatenating BOM-prefixed fragments produces.
fn trim_bom(line: &str) -> &str {
    let mut rest = line;
    while let Some(stripped) = rest.strip_prefix('\u{FEFF}') {
        rest = stripped;
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value that starts with a multi-byte character must not panic.
    ///
    /// `parse_trigstr` used to compare `trimmed.split_at(7)`, which counts bytes —
    /// so a value whose 7th byte fell inside a character panicked and aborted the
    /// command. This is the exact byte sequence that did it:
    /// `war3 map info 澄海3C-AI版.w3x` died with "end byte index 7 is not a char
    /// boundary; it is inside the character ...".
    #[test]
    fn a_non_ascii_value_is_rejected_without_panicking() {
        // The same escapes the fixtures above use, so the source stays ASCII.
        let chinese = "\u{529b}\u{91cf}\u{8f6c}\u{6362}"; // 4 characters, 12 bytes
        assert!(parse_trigstr(chinese).is_none());
        // The dangerous case: a 3-byte character straddling byte 7.
        let straddles = "\u{4e2d}\u{56fd}\u{529b}";
        assert!(parse_trigstr(straddles).is_none());
        // And the ordinary cases still work, including around the boundary.
        assert_eq!(parse_trigstr("TRIGSTR_010"), Some(10));
        assert_eq!(parse_trigstr("  trigstr_7  "), Some(7));
        assert!(parse_trigstr("TRIGSTR").is_none());
        assert!(parse_trigstr("STRIGSTR_1").is_none());
    }

    #[test]
    fn parses_a_simple_table() {
        let raw = b"STRING 0\r\n{\r\nHello\r\n}\r\n\r\nSTRING 1\r\n{\r\nWorld\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.len(), 2);
        assert_eq!(table.get(0), Some("Hello"));
        assert_eq!(table.get(1), Some("World"));
        assert!(table.diagnostics().is_empty());
    }

    /// A byte-order mark before a label must not hide that entry.
    ///
    /// This is the bug the map `侏罗纪公园1.5.w3x` exposed: the file begins with
    /// `EF BB BF` then `STRING 1`, and entry 1 is where the map's own name lives.
    /// The table parsed every entry except that one, so the map resolved to
    /// `TRIGSTR_001` and reported a missing key — for a table that had it.
    #[test]
    fn a_bom_before_a_label_does_not_hide_the_entry() {
        let raw = b"\xEF\xBB\xBFSTRING 1\r\n{\r\n\xE5\x90\x8D\xE5\xAD\x97\r\n}\r\n\r\nSTRING 2\r\n{\r\nsecond\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.len(), 2, "both entries must be seen");
        assert_eq!(table.get(1), Some("\u{540d}\u{5b57}"), "the BOM-prefixed first entry");
        assert_eq!(table.get(2), Some("second"));
        assert!(
            !table.diagnostics().has_problems(),
            "a BOM is normal in these files, not something to warn about"
        );
    }

    /// A mark before the brace must not make the brace part of the value.
    #[test]
    fn a_bom_before_a_brace_is_still_a_brace() {
        let raw = b"STRING 3\r\n\xEF\xBB\xBF{\r\nvalue\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.get(3), Some("value"));
    }

    #[test]
    fn multi_line_values_are_joined_with_newlines() {
        let raw = b"STRING 3\r\n{\r\nline one\r\nline two\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.get(3), Some("line one\nline two"));
    }

    #[test]
    fn a_label_comment_before_the_brace_is_not_part_of_the_value() {
        let raw = b"STRING 7\r\n// \xE6\x8A\x80\xE8\x83\xBD: A000 (\xE8\xB7\xB3\xE8\xB7\x832), Name (\xE5\x90\x8D\xE5\xAD\x97)\r\n{\r\n\xE8\xB7\xB3\xE8\xB7\x832\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(
            table.get(7),
            Some("\u{8df3}\u{8dc3}2"),
            "the generator's label must not be folded into the value"
        );
        assert!(
            !table.diagnostics().has_problems(),
            "a label is normal, not something to warn about"
        );
    }

    #[test]
    fn blank_lines_before_the_brace_are_skipped_too() {
        let raw = b"STRING 1\r\n// label\r\n\r\n// second label\r\n{\r\nvalue\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.get(1), Some("value"));
    }

    #[test]
    fn a_comment_after_the_brace_is_kept_verbatim() {
        // Inside the braces the text is the author's, so nothing is stripped.
        let raw = b"STRING 1\r\n{\r\n// not a label\r\nvalue\r\n}\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.get(1), Some("// not a label\nvalue"));
    }

    #[test]
    fn a_brace_less_entry_keeps_its_text_and_says_so() {
        let raw = b"STRING 4\r\nno brace here\r\n";
        let table = StringTable::parse(raw);
        assert_eq!(table.get(4), Some("no brace here"));
        assert!(table
            .diagnostics()
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::WtsMissingOpenBrace));
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

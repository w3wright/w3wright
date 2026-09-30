//! SYLK (`.slk`) parsing.
//!
//! The game's `*MetaData.slk`, `TerrainArt\Water.slk` and friends are SYLK, not
//! tab-separated text. The two look similar enough at a glance that reading them
//! as TSV yields an empty table with no error.
//!
//! # Structure
//!
//! ```text
//! ID;PWXL;N;E
//! B;X23;Y268;D0
//! C;X1;Y1;K"ID"       C;X2;K"field"      C;X3;K"slk"
//! C;X1;Y62;K"unam"
//! C;X2;K"Name"
//! C;X4;K0
//! ```
//!
//! - Records are separated by newlines, and the fields *within* a record by
//!   `;`. Both layers matter: a physical line can hold several `C` records
//!   (`C;X1;Y1;K"ID";C;X2;K"field"`), and a record can be alone on its line.
//! - `B`, `F`, `I`, `O`, `P` and `E` records are not cell data and are skipped.
//! - Records are sparse: when `X` is absent, the column from the last record
//!   that had one applies.
//! - Strings are quoted (`K"..."`), numbers are bare (`K-1`, `K0`), and an `E`
//!   field means the cell is empty.
//!
//! Row 1 holds column names and column 1 holds row keys, so the table offers
//! lookups by `(column name, row key)` rather than forcing callers to track
//! coordinates themselves.

use std::collections::BTreeMap;
use std::fmt;

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};

/// A parsed SYLK table.
#[derive(Debug, Clone, Default)]
pub struct SylkTable {
    /// Column number (1-based) to column name.
    columns: BTreeMap<u32, String>,
    /// Row number (1-based) to that row's key, taken from column 1.
    row_keys: BTreeMap<u32, String>,
    /// `(column, row)` to value.
    cells: BTreeMap<(u32, u32), String>,
    diagnostics: Diagnostics,
}

impl SylkTable {
    /// Column number to column name.
    #[must_use]
    pub fn columns(&self) -> &BTreeMap<u32, String> {
        &self.columns
    }

    /// Number of rows, including the header row.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_keys.len()
    }

    /// Total number of cells.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Finds a column number by name, case-insensitively.
    #[must_use]
    pub fn column_index(&self, name: &str) -> Option<u32> {
        self.columns
            .iter()
            .find(|(_, v)| v.eq_ignore_ascii_case(name))
            .map(|(k, _)| *k)
    }

    /// Looks up a cell by column name and row key.
    ///
    /// Both are matched case-insensitively, which matters because the game's own
    /// files are inconsistent about case.
    #[must_use]
    pub fn get(&self, column: &str, row_key: &str) -> Option<&str> {
        let col = self.column_index(column)?;
        let row = self
            .row_keys
            .iter()
            .find(|(_, v)| v.eq_ignore_ascii_case(row_key))
            .map(|(k, _)| *k)?;
        self.cells.get(&(col, row)).map(String::as_str)
    }

    /// Every row key except the header row.
    #[must_use]
    pub fn row_keys(&self) -> Vec<&str> {
        self.row_keys
            .iter()
            .filter(|(row, _)| **row != 1)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// A raw cell by coordinate.
    #[must_use]
    pub fn cell(&self, column: u32, row: u32) -> Option<&str> {
        self.cells.get(&(column, row)).map(String::as_str)
    }

    /// Diagnostics from parsing.
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
}

impl fmt::Display for SylkTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SylkTable({} columns, {} rows, {} cells)",
            self.columns.len(),
            self.row_keys.len(),
            self.cells.len()
        )
    }
}

/// Parses SYLK text.
///
/// # Two levels of separators
///
/// Getting either level wrong misaligns real files silently:
///
/// - records are separated by newlines;
/// - fields inside a record are separated by `;`.
///
/// So a record reads `C;X1;Y1;K"ID"`, and one line may contain several of them.
/// Treating every `;`-separated chunk as a record splits `X`, `Y` and `K` into
/// fake records and yields an empty table — without reporting anything.
#[must_use]
pub fn parse_sylk(text: &str) -> SylkTable {
    let mut table = SylkTable::default();

    let mut current_row: u32 = 1;
    // Sparse records: when `X` is absent, the last column seen applies.
    let mut last_x: u32 = 1;

    for line in text.split(['\n', '\r']) {
        // Only strip newline characters. Stripping spaces as well would remove a
        // separator such as the one in `...K"ID" C;X2;...`, merging the next
        // record into the previous one and dropping it silently.
        let line = line.trim_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }

        for record in split_records(line) {
            // Each record is trimmed individually: there may be whitespace
            // between the end of one record and the start of the next.
            let record = record.trim();
            if record.is_empty() {
                continue;
            }
            let mut fields = record.split(';');
            let Some(kind) = fields.next() else { continue };
            if kind != "C" {
                continue;
            }

            let mut x: Option<u32> = None;
            let mut y: Option<u32> = None;
            let mut value: Option<String> = None;

            for field in fields {
                let field = field.trim();
                let mut chars = field.chars();
                let Some(tag) = chars.next() else { continue };
                let payload: String = chars.collect();
                match tag {
                    'X' => x = payload.trim().parse::<u32>().ok(),
                    'Y' => y = payload.trim().parse::<u32>().ok(),
                    'K' => value = Some(unquote(&payload)),
                    // `C;X1;Y1;E` marks an empty cell and carries no value.
                    _ => {}
                }
            }

            if let Some(x) = x {
                last_x = x;
            }
            if let Some(y) = y {
                current_row = y;
            }
            let column = x.unwrap_or(last_x);

            // A record without `K` marks an empty cell.
            let Some(value) = value else { continue };

            if current_row == 1 {
                table.columns.insert(column, value.clone());
            }
            if column == 1 && current_row != 1 {
                table.row_keys.insert(current_row, value.clone());
            }
            table.cells.insert((column, current_row), value);
        }
    }

    if table.cells.is_empty() {
        table.diagnostics.push(Diagnostic::warn(
            DiagnosticCode::AssetFallbackUsed,
            "no C records found in the SYLK table; it may be tab-separated text, or empty",
        ));
    }

    table
}

/// Removes the quotes around a SYLK string value.
///
/// `K"abc"` becomes `abc`, `K0` stays `0`, `K` becomes empty.
fn unquote(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed)
        .to_string()
}

/// Splits one line into records.
///
/// # Why a naive "split on `C;`" is wrong
///
/// A record is `C;X<col>;Y<row>;K<value>`, and the value may contain any
/// character, `C` and `;` included. Real files hold field names, paths and
/// `WESTRING_*` keys in there.
///
/// Quoted spans therefore have to be skipped, and record boundaries recognised
/// only outside them.
///
/// # Boundary rule
///
/// A `C` starts a record when it is at the start of the line, or follows `;`, or
/// follows whitespace — and is itself followed by `;`. The whitespace case
/// matters: `B;X9;Y9;D0 C;X1;...` separates records with a space, and accepting
/// only `;` merges the first `C` record into the preceding `B` record, after
/// which its first field is not `C` and the record is dropped without a word.
fn split_records(line: &str) -> Vec<String> {
    let mut records = Vec::new();
    let bytes = line.as_bytes();
    let mut start = 0usize;
    let mut in_quotes = false;

    for (i, &byte) in bytes.iter().enumerate() {
        if byte == b'"' {
            in_quotes = !in_quotes;
            continue;
        }
        // Written as a plain `if` rather than a `match` guard: in a guard of the
        // form `a || b`, `b` is not evaluated when `a` holds, which would skip
        // the second half of the condition entirely.
        if byte != b'C' || in_quotes {
            continue;
        }
        let at_boundary = i == 0 || bytes[i - 1] == b';' || bytes[i - 1].is_ascii_whitespace();
        let ends_record = i + 1 == bytes.len() || bytes[i + 1] == b';';
        if at_boundary && ends_record && i > start {
            records.push(line[start..i].to_string());
            start = i;
        }
    }
    if start < bytes.len() {
        records.push(line[start..].to_string());
    }
    records
}

// ---------------------------------------------------------------------------
// Water.slk
// ---------------------------------------------------------------------------

/// One row of `TerrainArt\Water.slk`.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterRow {
    /// Row key, e.g. `Lsha` or `Ldrk`.
    pub key: String,
    /// The `height` column, in world-unit factors; multiply by 128 to use it.
    pub height: Option<f32>,
    /// Any other columns, kept verbatim.
    pub extra: BTreeMap<String, String>,
}

/// Water level data read from `Water.slk`.
///
/// The water zero offset varies by tileset, so it cannot be a constant: the
/// commonly quoted `-89.6` is only correct for the tileset whose `height` is
/// `-0.7`.
#[derive(Debug, Clone, Default)]
pub struct WaterTable {
    rows: BTreeMap<String, WaterRow>,
}

impl WaterTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `Water.slk` text.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let table = parse_sylk(text);
        let mut rows = BTreeMap::new();
        for key in table.row_keys() {
            let height = table.get("height", key).and_then(|v| v.parse::<f32>().ok());
            let mut extra = BTreeMap::new();
            for column_name in table.columns().values() {
                if column_name.eq_ignore_ascii_case("height") {
                    continue;
                }
                if let Some(v) = table.get(column_name, key) {
                    extra.insert(column_name.clone(), v.to_string());
                }
            }
            rows.insert(
                key.to_lowercase(),
                WaterRow {
                    key: key.to_string(),
                    height,
                    extra,
                },
            );
        }
        Self { rows }
    }

    /// A row by key.
    #[must_use]
    pub fn row(&self, key: &str) -> Option<&WaterRow> {
        self.rows.get(&key.to_lowercase())
    }

    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The `height` value for a tileset.
    ///
    /// Row keys are `<tileset letter>sha` for shallow water, so Lordaeron Summer
    /// is `Lsha`.
    ///
    /// `None` means the value is unknown; callers should not substitute `-89.6`,
    /// which is only one tileset's value and would shift every water level.
    #[must_use]
    pub fn height_for_tileset(&self, tileset: crate::w3e::Tileset) -> Option<f32> {
        let letter = (tileset.water_key() as char).to_ascii_lowercase();
        for suffix in ["sha", "shb"] {
            let key = format!("{letter}{suffix}");
            if let Some(row) = self.rows.get(&key) {
                if let Some(h) = row.height {
                    return Some(h);
                }
            }
        }
        None
    }
}

impl fmt::Display for WaterTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WaterTable({} rows)", self.rows.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fragment shaped like the real `*MetaData.slk` files.
    const SAMPLE: &str = "ID;PWXL;N;E\n\
B;X23;Y268;D0\n\
C;X1;Y1;K\"ID\";C;X2;K\"field\";C;X3;K\"slk\";C;X4;K\"index\"\n\
C;X1;Y62;K\"unam\";C;X2;K\"Name\";C;X3;K\"Profile\";C;X4;K0\n\
C;X1;Y63;K\"unsf\";C;X2;K\"Sfx\";C;X3;K\"Profile\";C;X4;K-1\n";

    #[test]
    fn parses_column_names_from_the_first_row() {
        let t = parse_sylk(SAMPLE);
        assert_eq!(t.column_index("ID"), Some(1));
        assert_eq!(t.column_index("field"), Some(2));
        assert_eq!(t.column_index("slk"), Some(3));
        assert_eq!(t.column_index("index"), Some(4));
    }

    #[test]
    fn column_lookup_is_case_insensitive() {
        let t = parse_sylk(SAMPLE);
        assert_eq!(t.column_index("id"), Some(1));
        assert_eq!(t.column_index("ID"), Some(1));
    }

    #[test]
    fn looks_up_cells_by_column_name_and_row_key() {
        let t = parse_sylk(SAMPLE);
        assert_eq!(t.get("field", "unam"), Some("Name"));
        assert_eq!(t.get("slk", "unam"), Some("Profile"));
        assert_eq!(t.get("index", "unam"), Some("0"));
        assert_eq!(t.get("field", "unsf"), Some("Sfx"));
        assert_eq!(t.get("index", "unsf"), Some("-1"));
    }

    #[test]
    fn numeric_values_are_not_quoted() {
        let t = parse_sylk(SAMPLE);
        assert_eq!(t.cell(4, 62), Some("0"));
        assert_eq!(t.cell(4, 63), Some("-1"));
    }

    #[test]
    fn row_keys_excludes_the_header_row() {
        let t = parse_sylk(SAMPLE);
        let keys = t.row_keys();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"unam"));
        assert!(keys.contains(&"unsf"));
    }

    #[test]
    fn multiple_c_records_on_one_line_are_split() {
        let t = parse_sylk(SAMPLE);
        assert_eq!(t.column_index("index"), Some(4));
        assert_eq!(t.row_keys(), vec!["unam", "unsf"]);
    }

    #[test]
    fn sparse_cells_fall_back_to_the_last_x() {
        // A record with X but no Y keeps the current row.
        let text = "C;X1;Y1;K\"h\";C;X2;K\"k\"\nC;X1;Y2;K\"a\";C;X2;K\"b\"\n";
        let t = parse_sylk(text);
        assert_eq!(t.get("h", "a"), Some("a"));
        assert_eq!(t.get("k", "a"), Some("b"));
    }

    #[test]
    fn header_and_boundary_records_are_ignored() {
        let t = parse_sylk("ID;PWXL;N;E\nB;X2;Y2;D0\nF;P0;DG0;G0\nC;X1;Y1;K\"a\"\n");
        assert!(
            t.row_keys().is_empty(),
            "only the header row should be parsed"
        );
        assert_eq!(t.cell_count(), 1);
        assert_eq!(t.column_index("a"), Some(1));
    }

    #[test]
    fn leading_non_c_record_does_not_swallow_the_first_cell() {
        // Guards two silent-loss cases at once: a space separating records must
        // still be recognised as a boundary, and a `C` inside a quoted value
        // must not be mistaken for one.
        let t =
            parse_sylk("B;X9;Y9;D0 C;X1;Y1;K\"ID\";C;X2;K\"field\"\nC;X1;Y2;K\"k\";C;X2;K\"v\"\n");
        assert_eq!(t.column_index("ID"), Some(1));
        assert_eq!(t.column_index("field"), Some(2));
        assert_eq!(t.get("field", "k"), Some("v"));
    }

    #[test]
    fn empty_input_is_diagnosed_not_silently_empty() {
        let t = parse_sylk("");
        assert!(t
            .diagnostics()
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::AssetFallbackUsed));
    }

    #[test]
    fn tsv_masquerading_as_sylk_is_diagnosed() {
        let t = parse_sylk("ID\tfield\tslk\nunam\tName\tProfile\n");
        assert_eq!(t.cell_count(), 0);
        assert!(t.diagnostics().has_problems());
    }

    #[test]
    fn unquote_handles_quoted_bare_and_empty() {
        assert_eq!(unquote("\"abc\""), "abc");
        assert_eq!(unquote("0"), "0");
        assert_eq!(unquote(""), "");
        // A single quote is left alone rather than force-matched.
        assert_eq!(unquote("\"abc"), "\"abc");
    }

    // ---- Water.slk ----

    fn water_slk() -> String {
        "ID;PWXL;N;E\n\
         C;X1;Y1;K\"type\";C;X2;K\"height\";C;X3;K\"minimapcolor\"\n\
         C;X1;Y2;K\"Lsha\";C;X2;K-0.7;C;X3;K255\n\
         C;X1;Y3;K\"Asha\";C;X2;K-0.6;C;X3;K254\n\
         C;X1;Y4;K\"Zsha\";C;X2;K-0.5;C;X3;K253\n"
            .to_string()
    }

    #[test]
    fn water_table_parses_rows() {
        let w = WaterTable::parse(&water_slk());
        assert_eq!(w.len(), 3);
        assert_eq!(w.row("Lsha").unwrap().height, Some(-0.7));
    }

    #[test]
    fn water_height_is_looked_up_per_tileset() {
        let w = WaterTable::parse(&water_slk());
        assert_eq!(
            w.height_for_tileset(crate::w3e::Tileset::LordaeronSummer),
            Some(-0.7)
        );
        assert_eq!(
            w.height_for_tileset(crate::w3e::Tileset::Ashenvale),
            Some(-0.6)
        );
        assert_eq!(
            w.height_for_tileset(crate::w3e::Tileset::SunkenRuins),
            Some(-0.5)
        );
    }

    #[test]
    fn unknown_tileset_returns_none_not_a_guess() {
        let w = WaterTable::parse(&water_slk());
        assert_eq!(w.height_for_tileset(crate::w3e::Tileset::Northrend), None);
    }

    #[test]
    fn empty_water_table_returns_none() {
        let w = WaterTable::new();
        assert!(w.is_empty());
        assert_eq!(
            w.height_for_tileset(crate::w3e::Tileset::LordaeronSummer),
            None
        );
    }

    #[test]
    fn tileset_letter_to_water_key_matches_the_slk_convention() {
        assert_eq!(crate::w3e::Tileset::LordaeronSummer.water_key(), b'L');
        assert_eq!(crate::w3e::Tileset::SunkenRuins.water_key(), b'Z');
    }
}

//! Just enough SYLK to read a table's rows by column name.
//!
//! # What this is for
//!
//! Two of the seven object kinds keep their names in a `.slk` rather than in a `*Strings.txt`:
//! `Doodads\Doodads.slk` and `Units\DestructableData.slk`. Both are large — 403 KB and 209 KB —
//! with 67 and 57 columns, of which this needs **two**.
//!
//! # Why not [`war3_meta::MetaTable`]
//!
//! That type is the object-*field* metadata reader: its rows describe fields (`unam`, `uhpm`) and
//! its API is built around that. These two files describe objects (`hfoo`, `APms`), have a
//! different shape, and `MetaTable` keeps only the columns a field definition uses. Reusing it
//! would mean widening a type whose doc comment says what it is for.
//!
//! # The two format rules that are easy to get wrong
//!
//! Both were got wrong here first, and both produce a table that parses without error and is
//! silently missing its data:
//!
//! 1. **An omitted `Y` keeps the previous row number.** The name row of `Doodads.slk` is
//!    `C;X1;Y1;K"doodID"` followed by `C;X2;K"comment"`, `C;X3;K"category"` … — one `Y` for the
//!    whole row. Treating each cell's row as "whatever this line says" leaves every column after
//!    the first belonging to row 0, and the header comes back with a single name in it.
//! 2. **An omitted `X` keeps the previous column.** Same rule for the other axis.
//!
//! The parser therefore carries both numbers forward, which is what the format says and what the
//! real files need.

use std::collections::BTreeMap;

/// One `.slk` file: column names, and the cells of each data row.
/// Splits one SYLK record into its fields.
///
/// ⚠️ **Not `line.split(';')`.** A quoted value may contain the separator — `K"one; two"` is one
/// field — and splitting first cuts it into three, so the value comes back truncated and the
/// remainder is read as further fields. That is a parse which succeeds and is wrong, which is the
/// failure mode this whole module is written to avoid.
///
/// Quotes are what delimit the value, so they have to be tracked while splitting rather than
/// stripped afterwards.
fn split_fields(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0usize;
    let mut in_quotes = false;

    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                fields.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    fields.push(&line[start..]);
    fields
}

#[derive(Debug, Clone, Default)]
pub struct SylkTable {
    /// Column number to the name in row 1.
    columns: BTreeMap<u32, String>,
    /// Row number to its cells, for rows after the header.
    rows: BTreeMap<u32, BTreeMap<u32, String>>,
}

impl SylkTable {
    /// Parses a `.slk` file.
    ///
    /// Returns `None` when the text holds no `C;` record at all, which is how a file that is not
    /// SYLK — or one that failed to decompress into text — is rejected instead of yielding an
    /// empty table that looks like a successful read of a table with no contents.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut table = Self::default();
        let mut saw_record = false;
        let mut col = 0u32;
        let mut row = 0u32;

        for line in text.lines() {
            // Records other than `C` — `ID`, `B`, `E` — carry no cell and are skipped. `B` in
            // particular states the sheet's bounds and is not data.
            if !line.starts_with("C;") {
                continue;
            }
            saw_record = true;

            let mut value: Option<String> = None;
            for field in split_fields(line) {
                if let Some(rest) = field.strip_prefix('X') {
                    // A non-numeric or absent X keeps the previous column, per the format.
                    if let Ok(n) = rest.parse::<u32>() {
                        col = n;
                    }
                } else if let Some(rest) = field.strip_prefix('Y') {
                    if let Ok(n) = rest.parse::<u32>() {
                        row = n;
                    }
                } else if let Some(rest) = field.strip_prefix("K\"") {
                    // A quoted value: the closing quote is the last character of the field.
                    value = Some(rest.strip_suffix('"').unwrap_or(rest).to_string());
                } else if let Some(rest) = field.strip_prefix('K') {
                    value = Some(rest.to_string());
                }
            }

            let Some(value) = value else { continue };
            if col == 0 || row == 0 {
                continue;
            }
            if row == 1 {
                table.columns.insert(col, value);
            } else {
                table.rows.entry(row).or_default().insert(col, value);
            }
        }

        saw_record.then_some(table)
    }

    /// The column number whose name is `name`, case-insensitively.
    #[must_use]
    pub fn column_number(&self, name: &str) -> Option<u32> {
        self.columns
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(name))
            .map(|(c, _)| *c)
    }

    /// The names of every column, in column order.
    #[must_use]
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.values().map(String::as_str).collect()
    }

    /// How many data rows the table has.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The rows, in row order.
    ///
    /// A view rather than a `Vec` of owned rows: `Doodads.slk` has thousands of rows and 67
    /// columns, and a caller that wants two columns should not pay for a copy of the other 65.
    #[must_use = "an iterator that is not consumed reads nothing"]
    pub fn rows(&self) -> impl Iterator<Item = SylkRow<'_>> {
        self.rows.values().map(|cells| SylkRow { cells })
    }
}

/// One row of a [`SylkTable`], read by column number.
#[derive(Debug, Clone, Copy)]
pub struct SylkRow<'a> {
    cells: &'a BTreeMap<u32, String>,
}

impl<'a> SylkRow<'a> {
    /// The cell in a column, or `None` when the row leaves it out.
    ///
    /// A row omits a column rather than writing an empty value, so `None` and `Some("")` are both
    /// possible and mean different things: absent, and present but blank.
    #[must_use]
    pub fn get(&self, column: u32) -> Option<&'a str> {
        self.cells.get(&column).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ The rule that was got wrong first: `Y` appears once per row and carries to the cells
    /// after it. This is the real shape of `Doodads.slk`'s header, shortened.
    #[test]
    fn an_omitted_y_row_number_carries_to_the_next_cell() {
        let text = "ID;PWXL;N;E\nB;X32;Y837;D0\n\
                    C;X1;Y1;K\"doodID\"\nC;X2;K\"comment\"\nC;X3;K\"category\"\n\
                    C;X1;Y2;K\"APms\"\nC;X2;K\"Mushrooms\"\nC;X3;K\"E\"\n";
        let table = SylkTable::parse(text).unwrap();

        assert_eq!(table.column_number("doodID"), Some(1));
        assert_eq!(table.column_number("comment"), Some(2));
        assert_eq!(table.column_number("category"), Some(3));
        assert_eq!(table.column_names(), vec!["doodID", "comment", "category"]);
        assert_eq!(table.row_count(), 1);

        let row = table.rows().next().unwrap();
        assert_eq!(row.get(1), Some("APms"));
        assert_eq!(row.get(2), Some("Mushrooms"));
        assert_eq!(row.get(3), Some("E"));
    }

    /// And on the other axis: an omitted `X` keeps the previous column.
    #[test]
    fn an_omitted_x_column_carries_too() {
        let text = "C;X1;Y1;K\"a\"\nC;X2;K\"b\"\nC;X1;Y2;K\"one\"\nC;X2;K\"two\"\n";
        let table = SylkTable::parse(text).unwrap();
        let row = table.rows().next().unwrap();
        assert_eq!(row.get(1), Some("one"));
        assert_eq!(row.get(2), Some("two"));
    }

    /// Column names are matched case-insensitively: the file writes `DestructableID` and a caller
    /// asking for `destructableid` means the same column.
    #[test]
    fn column_names_match_without_case() {
        let text = "C;X1;Y1;K\"DestructableID\"\nC;X11;K\"Name\"\n";
        let table = SylkTable::parse(text).unwrap();
        assert_eq!(table.column_number("destructableid"), Some(1));
        assert_eq!(table.column_number("DESTRUCTABLEID"), Some(1));
        assert_eq!(table.column_number("Name"), Some(11));
        assert_eq!(table.column_number("nope"), None);
    }

    /// A row that leaves a column out reports `None`, not `Some("")`. The difference matters:
    /// `Doodads.slk` omits unused columns rather than writing blanks.
    #[test]
    fn an_absent_cell_is_none_and_a_blank_one_is_empty() {
        let text = "C;X1;Y1;K\"id\"\nC;X2;K\"Name\"\nC;X1;Y2;K\"APms\"\nC;X2;K\"\"\n";
        let table = SylkTable::parse(text).unwrap();
        let row = table.rows().next().unwrap();
        assert_eq!(row.get(1), Some("APms"));
        assert_eq!(row.get(2), Some(""));
        assert_eq!(row.get(9), None);
    }

    /// A file with no `C;` record is not SYLK, and says so rather than parsing as empty.
    #[test]
    fn a_file_that_is_not_sylk_is_rejected() {
        assert!(SylkTable::parse("").is_none());
        assert!(SylkTable::parse("ID;PWXL;N;E\nB;X32;Y837;D0\nE\n").is_none());
    }

    /// `B` states the sheet's bounds. Reading it as data would invent a row.
    #[test]
    fn the_bounds_record_is_not_data() {
        let text = "ID;PWXL;N;E\nB;X32;Y837;D0\nC;X1;Y1;K\"id\"\nC;X1;Y2;K\"x\"\n";
        let table = SylkTable::parse(text).unwrap();
        assert_eq!(table.column_names(), vec!["id"]);
        assert_eq!(table.row_count(), 1);
    }

    /// A quoted value keeps its inner semicolons, because the quote is what delimits it.
    #[test]
    fn a_quoted_value_may_contain_the_separator() {
        let text = "C;X1;Y1;K\"id\"\nC;X2;K\"Name\"\nC;X1;Y2;K\"a\"\nC;X2;K\"one; two; three\"\n";
        let table = SylkTable::parse(text).unwrap();
        let row = table.rows().next().unwrap();
        assert_eq!(row.get(2), Some("one; two; three"));
    }
}

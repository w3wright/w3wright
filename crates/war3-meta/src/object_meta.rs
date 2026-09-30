//! Object field metadata tables, read from the game's `*MetaData.slk` files.
//!
//! # Two known traps
//!
//! 1. **When the `field` column reads `Data`, the real name is `Data` followed by
//!    `chr('A' + data - 1)`** — `data=1` gives `DataA` and `data=9` gives
//!    `DataI`. Two independent implementations agree on this, and the expansion
//!    happens here.
//! 2. **An `index` of zero or more means the value replaces one element of a
//!    comma-separated list.** `abilList`, `Requires` and `buttonpos` all rely on
//!    it. [`crate::field::FieldMeta::apply_index`] implements the operation.
//!
//! # The type column is a vocabulary word
//!
//! `type` holds values such as `int`, `real`, `unreal`, `bool`, `abilList` — not
//! a binary type code. The mapping from those words to codes should be generated
//! from `UI\UnitEditorData.txt` rather than maintained by hand; three hand-written
//! versions exist and disagree. This module keeps the vocabulary string as is and
//! leaves the mapping to the caller.

use std::collections::BTreeMap;
use std::fmt;

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};
use war3_core::FourCC;

use crate::field::{FieldMeta, ObjectKind};
use war3_terrain::parse_sylk;

/// The mechanical layer of one `*MetaData.slk` file.
#[derive(Debug, Clone, Default)]
pub struct MetaTable {
    /// Which category this table describes.
    pub kind: Option<ObjectKind>,
    /// Identifier to field metadata.
    fields: BTreeMap<FourCC, FieldMeta>,
    /// SLK column name to identifier; the first row wins on duplicates.
    by_field_name: BTreeMap<String, FourCC>,
    /// Diagnostics from parsing.
    pub diagnostics: Diagnostics,
}

impl MetaTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `*MetaData.slk` text.
    ///
    /// Passing tab-separated text yields an empty table **and a diagnostic**,
    /// rather than passing silently.
    #[must_use]
    pub fn parse_sylk_text(kind: ObjectKind, text: &str) -> Self {
        let slk = parse_sylk(text);
        let mut diagnostics = Diagnostics::new();
        diagnostics.merge(slk.diagnostics());

        let mut fields = BTreeMap::new();
        let mut by_field_name = BTreeMap::new();
        let mut skipped_unknown_id = 0usize;

        for row in slk.row_keys() {
            // The `ID` column holds the identifier written into object files.
            let Some(id_text) = slk.get("ID", row) else {
                continue;
            };
            if id_text.len() != 4 {
                skipped_unknown_id += 1;
                continue;
            }
            let id = FourCC::from_str_lossy(id_text);

            let mut field = slk.get("field", row).unwrap_or("").to_string();
            let data = slk
                .get("data", row)
                .and_then(|v| v.trim().parse::<i32>().ok());

            // The `Data` trap: the real name is `Data` plus a letter.
            if field == "Data" {
                match data {
                    Some(d) if (1..=9).contains(&d) => {
                        let letter = (b'A' + (d as u8 - 1)) as char;
                        field = format!("Data{letter}");
                    }
                    Some(d) => {
                        diagnostics.push(Diagnostic::warn(
                            DiagnosticCode::AssetFallbackUsed,
                            format!(
                                "field {id} has field=Data but data = {d}, outside 1..=9; the real \
                                 name cannot be expanded, leaving the literal Data"
                            ),
                        ));
                    }
                    None => {
                        diagnostics.push(Diagnostic::warn(
                            DiagnosticCode::AssetFallbackUsed,
                            format!(
                                "field {id} has field=Data but no data column; the real name cannot \
                                 be expanded"
                            ),
                        ));
                    }
                }
            }

            let meta = FieldMeta {
                id,
                field: field.clone(),
                slk: slk.get("slk", row).map(str::to_string),
                index: slk
                    .get("index", row)
                    .and_then(|v| v.trim().parse::<i32>().ok()),
                repeat: slk
                    .get("repeat", row)
                    .and_then(|v| v.trim().parse::<i32>().ok()),
                data,
                type_name: slk.get("type", row).map(str::to_string),
                string_ext: slk
                    .get("stringExt", row)
                    .map(|v| v.trim() != "0")
                    .unwrap_or(false),
                display_key: slk.get("displayName", row).map(str::to_string),
                use_specific: slk.get("useSpecific", row).map(str::to_string),
                can_be_empty: slk.get("canBeEmpty", row).map(|v| v.trim() != "0"),
                min_val: slk
                    .get("minVal", row)
                    .and_then(|v| v.trim().parse::<f32>().ok()),
                max_val: slk
                    .get("maxVal", row)
                    .and_then(|v| v.trim().parse::<f32>().ok()),
                // Hard-coded: only these three categories carry levels, and the
                // file has no column saying so.
                has_level: matches!(
                    kind,
                    ObjectKind::Ability | ObjectKind::Doodad | ObjectKind::Upgrade
                ),
            };

            if !field.is_empty() && !by_field_name.contains_key(&field) {
                by_field_name.insert(field, id);
            }
            fields.insert(id, meta);
        }

        if skipped_unknown_id > 0 {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::AssetFallbackUsed,
                format!("{skipped_unknown_id} rows had an ID column that is not 4 bytes; skipped"),
            ));
        }

        Self {
            kind: Some(kind),
            fields,
            by_field_name,
            diagnostics,
        }
    }

    /// How many fields are known.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// A field by identifier.
    #[must_use]
    pub fn get(&self, id: FourCC) -> Option<&FieldMeta> {
        self.fields.get(&id)
    }

    /// A field by SLK column name.
    #[must_use]
    pub fn get_by_field(&self, field: &str) -> Option<&FieldMeta> {
        let id = self.by_field_name.get(field)?;
        self.fields.get(id)
    }

    /// Every field.
    pub fn iter(&self) -> impl Iterator<Item = &FieldMeta> {
        self.fields.values()
    }

    /// Every `displayName` key.
    ///
    /// Only keys should appear here, never text.
    #[must_use]
    pub fn display_keys(&self) -> Vec<&str> {
        self.fields
            .values()
            .filter_map(|m| m.display_key.as_deref())
            .collect()
    }
}

impl fmt::Display for MetaTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MetaTable({}, {} fields)",
            self.kind.map_or("unknown".to_string(), |k| k.to_string()),
            self.fields.len()
        )
    }
}

/// Metadata tables for all seven categories.
#[derive(Debug, Clone, Default)]
pub struct MetaTableSet {
    /// One table per category.
    pub tables: BTreeMap<ObjectKind, MetaTable>,
    /// Diagnostics from loading.
    pub diagnostics: Diagnostics,
}

impl MetaTableSet {
    /// Empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a table.
    pub fn insert(&mut self, kind: ObjectKind, table: MetaTable) {
        self.diagnostics.merge(&table.diagnostics);
        self.tables.insert(kind, table);
    }

    /// A table by category.
    #[must_use]
    pub fn get(&self, kind: ObjectKind) -> Option<&MetaTable> {
        self.tables.get(&kind)
    }

    /// How many categories are loaded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// Whether nothing is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// Which categories are missing.
    #[must_use]
    pub fn missing(&self) -> Vec<ObjectKind> {
        ObjectKind::ALL
            .into_iter()
            .filter(|k| !self.tables.contains_key(k))
            .collect()
    }

    /// Loads every category it can find.
    ///
    /// Categories that cannot be found are reported and skipped rather than
    /// failing, so that a user without the game installed can still parse maps.
    #[must_use]
    pub fn load_from_assets(source: &dyn war3_core::AssetSource) -> Self {
        let mut set = Self::new();
        for kind in ObjectKind::ALL {
            let path = format!("{}\\{}", kind.metadata_dir(), kind.metadata_slk());
            match source.get_text(&path) {
                Some(text) => {
                    let table = MetaTable::parse_sylk_text(kind, &text);
                    set.insert(kind, table);
                }
                None => {
                    set.diagnostics.push(Diagnostic::info(
                        DiagnosticCode::AssetFallbackUsed,
                        format!(
                            "{path} is unavailable, so {kind} field metadata cannot be read and \
                             display names degrade to raw identifiers"
                        ),
                    ));
                }
            }
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fragment shaped like the real `UnitMetaData.slk`.
    const UNIT_META: &str = "ID;PWXL;N;E\n\
B;X8;Y268;D0\n\
C;X1;Y1;K\"ID\";C;X2;K\"field\";C;X3;K\"slk\";C;X4;K\"index\";C;X5;K\"type\";C;X6;K\"displayName\";C;X7;K\"stringExt\"\n\
C;X1;Y62;K\"unam\";C;X2;K\"Name\";C;X3;K\"Profile\";C;X4;K-1;C;X5;K\"string\";C;X6;K\"WESTRING_UEVAL_UNAM\";C;X7;K1\n\
C;X1;Y63;K\"uabi\";C;X2;K\"abilList\";C;X3;K\"Profile\";C;X4;K0;C;X5;K\"abilList\";C;X6;K\"WESTRING_UEVAL_UABI\"\n";

    /// The ability table has two extra columns between `index` and `type`.
    const ABILITY_META: &str = "ID;PWXL;N;E\n\
C;X1;Y1;K\"ID\";C;X2;K\"field\";C;X3;K\"slk\";C;X4;K\"index\";C;X5;K\"repeat\";C;X6;K\"data\";C;X7;K\"type\";C;X8;K\"displayName\"\n\
C;X1;Y2;K\"atp1\";C;X2;K\"Data\";C;X3;K\"Profile\";C;X4;K-1;C;X5;K1;C;X6;K1;C;X7;K\"unreal\";C;X8;K\"WESTRING_AEVAL_ATP1\"\n\
C;X1;Y3;K\"atp9\";C;X2;K\"Data\";C;X3;K\"Profile\";C;X4;K-1;C;X5;K1;C;X6;K9;C;X7;K\"unreal\";C;X8;K\"WESTRING_AEVAL_ATP9\"\n";

    #[test]
    fn parses_unit_metadata_fields() {
        let table = MetaTable::parse_sylk_text(ObjectKind::Unit, UNIT_META);
        assert_eq!(table.len(), 2);

        let name = table.get(FourCC::from_str_lossy("unam")).unwrap();
        assert_eq!(name.field, "Name");
        assert_eq!(name.slk.as_deref(), Some("Profile"));
        assert_eq!(name.index, Some(-1));
        assert_eq!(name.type_name.as_deref(), Some("string"));
        assert!(name.string_ext);
        assert_eq!(name.display_key.as_deref(), Some("WESTRING_UEVAL_UNAM"));
    }

    #[test]
    fn display_keys_contain_westring_keys_not_text() {
        let table = MetaTable::parse_sylk_text(ObjectKind::Unit, UNIT_META);
        for key in table.display_keys() {
            assert!(
                key.starts_with("WESTRING_"),
                "displayName must be a key: {key}"
            );
        }
    }

    #[test]
    fn indexes_split_semantics_are_carried_through() {
        let table = MetaTable::parse_sylk_text(ObjectKind::Unit, UNIT_META);
        let abil = table.get(FourCC::from_str_lossy("uabi")).unwrap();
        assert_eq!(abil.index, Some(0));
        assert_eq!(FieldMeta::apply_index("AInv,SCc1", 0, "Buhb"), "Buhb,SCc1");
    }

    #[test]
    fn data_field_name_is_expanded_to_dataa_through_datai() {
        let table = MetaTable::parse_sylk_text(ObjectKind::Ability, ABILITY_META);
        let atp1 = table.get(FourCC::from_str_lossy("atp1")).unwrap();
        assert_eq!(atp1.field, "DataA", "data=1 must expand to DataA");
        let atp9 = table.get(FourCC::from_str_lossy("atp9")).unwrap();
        assert_eq!(atp9.field, "DataI", "data=9 must expand to DataI");
    }

    #[test]
    fn ability_metadata_carries_repeat_and_data_columns() {
        let table = MetaTable::parse_sylk_text(ObjectKind::Ability, ABILITY_META);
        let atp1 = table.get(FourCC::from_str_lossy("atp1")).unwrap();
        assert_eq!(atp1.repeat, Some(1));
        assert_eq!(atp1.data, Some(1));
    }

    #[test]
    fn level_capability_is_hard_coded_per_kind() {
        let ability = MetaTable::parse_sylk_text(ObjectKind::Ability, ABILITY_META);
        assert!(ability.iter().all(|m| m.has_level));
        let unit = MetaTable::parse_sylk_text(ObjectKind::Unit, UNIT_META);
        assert!(unit.iter().all(|m| !m.has_level));
    }

    #[test]
    fn data_out_of_range_is_diagnosed_not_silently_kept() {
        let text = "C;X1;Y1;K\"ID\";C;X2;K\"field\";C;X3;K\"data\"\n\
                    C;X1;Y2;K\"xxxx\";C;X2;K\"Data\";C;X3;K99\n";
        let table = MetaTable::parse_sylk_text(ObjectKind::Ability, text);
        assert!(table
            .diagnostics
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::AssetFallbackUsed));
        assert_eq!(
            table.get(FourCC::from_str_lossy("xxxx")).unwrap().field,
            "Data"
        );
    }

    #[test]
    fn tsv_input_yields_an_empty_table_with_a_diagnostic() {
        let table = MetaTable::parse_sylk_text(ObjectKind::Unit, "ID\tfield\nunam\tName\n");
        assert!(table.is_empty());
        assert!(table.diagnostics.has_problems());
    }

    #[test]
    fn lookup_by_field_name_works() {
        let table = MetaTable::parse_sylk_text(ObjectKind::Unit, UNIT_META);
        assert_eq!(
            table.get_by_field("Name").map(|m| m.id),
            Some(FourCC::from_str_lossy("unam"))
        );
        assert!(table.get_by_field("Nope").is_none());
    }

    #[test]
    fn meta_table_set_reports_missing_kinds() {
        let mut set = MetaTableSet::new();
        set.insert(
            ObjectKind::Unit,
            MetaTable::parse_sylk_text(ObjectKind::Unit, UNIT_META),
        );
        assert_eq!(set.len(), 1);
        let missing = set.missing();
        assert_eq!(missing.len(), 6);
        assert!(!missing.contains(&ObjectKind::Unit));
    }

    #[test]
    fn loading_without_game_data_degrades_without_panicking() {
        let set = MetaTableSet::load_from_assets(&war3_core::EmptyAssetSource);
        assert!(set.is_empty());
        // Reported, but at info level: a missing installation is an environment
        // fact, not a data problem.
        assert!(!set.diagnostics.items().is_empty());
        assert!(!set
            .diagnostics
            .items()
            .iter()
            .any(|d| d.severity == war3_core::Severity::Error));
    }

    #[test]
    fn loading_from_a_memory_asset_source_works() {
        let source = war3_core::MemoryAssetSource::new()
            .with("Units\\UnitMetaData.slk", UNIT_META.as_bytes().to_vec());
        let set = MetaTableSet::load_from_assets(&source);
        // Two, not one: items and units share the same file.
        assert_eq!(set.len(), 2);
        assert_eq!(set.get(ObjectKind::Unit).unwrap().len(), 2);
        assert_eq!(set.get(ObjectKind::Item).unwrap().len(), 2);
    }
}

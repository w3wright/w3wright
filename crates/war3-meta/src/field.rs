//! The mechanical layer of object field metadata.
//!
//! No display text appears here. `display_key` holds a `WESTRING_*` key rather
//! than the text it resolves to, and a test enforces that.

use std::fmt;

use war3_core::FourCC;

/// Object category.
///
/// Seven categories, each with its own `.w3*` map file and metadata table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObjectKind {
    /// Units, `war3map.w3u`.
    Unit,
    /// Items, `war3map.w3t`, sharing `UnitMetaData.slk` with units.
    Item,
    /// Destructables, `war3map.w3b`.
    Destructable,
    /// Doodads, `war3map.w3d`.
    Doodad,
    /// Abilities, `war3map.w3a`.
    Ability,
    /// Buffs, `war3map.w3h`.
    Buff,
    /// Upgrades, `war3map.w3q`.
    Upgrade,
}

impl ObjectKind {
    /// All seven categories.
    pub const ALL: [Self; 7] = [
        Self::Unit,
        Self::Item,
        Self::Destructable,
        Self::Doodad,
        Self::Ability,
        Self::Buff,
        Self::Upgrade,
    ];

    /// The metadata file this category uses.
    ///
    /// Items and units **share** `UnitMetaData.slk`.
    #[must_use]
    pub const fn metadata_slk(self) -> &'static str {
        match self {
            Self::Unit | Self::Item => "UnitMetaData.slk",
            Self::Destructable => "DestructableMetaData.slk",
            Self::Doodad => "DoodadMetaData.slk",
            Self::Ability => "AbilityMetaData.slk",
            Self::Buff => "AbilityBuffMetaData.slk",
            Self::Upgrade => "UpgradeMetaData.slk",
        }
    }

    /// The directory the metadata file lives in.
    #[must_use]
    pub const fn metadata_dir(self) -> &'static str {
        match self {
            Self::Doodad => "Doodads",
            _ => "Units",
        }
    }

    /// The corresponding map file.
    #[must_use]
    pub const fn map_file(self) -> &'static str {
        match self {
            Self::Unit => "war3map.w3u",
            Self::Item => "war3map.w3t",
            Self::Destructable => "war3map.w3b",
            Self::Doodad => "war3map.w3d",
            Self::Ability => "war3map.w3a",
            Self::Buff => "war3map.w3h",
            Self::Upgrade => "war3map.w3q",
        }
    }

    /// Whether an object identifier denotes a hero.
    ///
    /// The test is whether the first letter is uppercase, matching how the game
    /// distinguishes heroes from ordinary units.
    #[must_use]
    pub fn is_hero_id(id: FourCC) -> bool {
        id.0[0].is_ascii_uppercase()
    }

    /// Sort rank, which puts uppercase identifiers before lowercase ones.
    ///
    /// This ordering matters beyond tidiness: with two objects sharing an
    /// identifier, the wrong order makes hero data come out incorrect.
    #[must_use]
    pub fn sort_rank(id: FourCC) -> u8 {
        if Self::is_hero_id(id) {
            0
        } else {
            1
        }
    }
}

impl fmt::Display for ObjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unit => "unit",
            Self::Item => "item",
            Self::Destructable => "destructable",
            Self::Doodad => "doodad",
            Self::Ability => "ability",
            Self::Buff => "buff",
            Self::Upgrade => "upgrade",
        })
    }
}

/// The binary type code stored in object files.
///
/// # Not the same as the metadata's type column
///
/// The `type` column in `*MetaData.slk` holds a vocabulary word — around seventy
/// of them, such as `int`, `real`, `unreal`, `bool`, `abilList`, `unitList`,
/// `attackBits`. Mapping those to the four codes below is generated from
/// `UI\UnitEditorData.txt`; three hand-written mappings exist in the wild and
/// they disagree with each other, so this one is not maintained by hand.
///
/// What is modelled here are the four values that actually appear in `.w3u` and
/// `.w3a`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldType {
    /// `0`, integer.
    Integer,
    /// `1`, real.
    Real,
    /// `2`, unsigned real.
    Unreal,
    /// `3`, NUL-terminated string.
    String,
    /// An unrecognised code, preserved rather than treated as a string.
    Unknown(i32),
}

impl FieldType {
    /// The on-disk code.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Integer => 0,
            Self::Real => 1,
            Self::Unreal => 2,
            Self::String => 3,
            Self::Unknown(c) => c,
        }
    }

    /// Parses an on-disk code.
    #[must_use]
    pub const fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Integer,
            1 => Self::Real,
            2 => Self::Unreal,
            3 => Self::String,
            other => Self::Unknown(other),
        }
    }

    /// How many bytes the value occupies, for fixed-width types.
    #[must_use]
    pub const fn fixed_width(self) -> Option<usize> {
        match self {
            Self::Integer | Self::Real | Self::Unreal => Some(4),
            Self::String | Self::Unknown(_) => None,
        }
    }
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer => f.write_str("int"),
            Self::Real => f.write_str("real"),
            Self::Unreal => f.write_str("unreal"),
            Self::String => f.write_str("string"),
            Self::Unknown(c) => write!(f, "unknown({c})"),
        }
    }
}

/// One row of a `*MetaData.slk` file.
///
/// No `Eq`, because the range fields are floats.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldMeta {
    /// The identifier written into object modifiers, from the `ID` column.
    pub id: FourCC,
    /// The SLK column name, from the `field` column.
    ///
    /// When the `field` column reads `Data`, the real name is `Data` followed by
    /// `chr('A' + data - 1)` — `data=1` gives `DataA` through to `data=9` giving
    /// `DataI`. That expansion is done by the loader, so what arrives here is
    /// already the real name.
    pub field: String,
    /// Where the default value lives, from the `slk` column.
    ///
    /// The magic value `Profile` means a profile text file.
    pub slk: Option<String>,
    /// Position in a comma-separated list, from the `index` column.
    ///
    /// `-1` means the field exists only on the object and takes the whole column.
    /// A value of zero or more means the existing value is split on commas and
    /// the element at that position is replaced, which is how `abilList`,
    /// `Requires` and `buttonpos` work.
    pub index: Option<i32>,
    /// Maximum number of levels, from the `repeat` column.
    pub repeat: Option<i32>,
    /// The `data` column, used to expand the `Data` field name.
    pub data: Option<i32>,
    /// The vocabulary type name from the `type` column, such as `int` or
    /// `abilList`.
    ///
    /// A string rather than a binary code; see [`FieldType`].
    pub type_name: Option<String>,
    /// Whether values may be `TRIGSTR_nnn` references, from `stringExt`.
    pub string_ext: bool,
    /// The `displayName` column, holding a `WESTRING_*` **key**.
    pub display_key: Option<String>,
    /// The `useSpecific` column, which limits a field to certain object codes.
    pub use_specific: Option<String>,
    /// The `canBeEmpty` column.
    pub can_be_empty: Option<bool>,
    /// The `minVal` column.
    pub min_val: Option<f32>,
    /// The `maxVal` column.
    pub max_val: Option<f32>,
    /// Whether the field carries level information.
    ///
    /// Hard-coded rather than read from the file: only abilities, doodads and
    /// upgrades do.
    pub has_level: bool,
}

impl FieldMeta {
    /// The field name at a given level. Levels are one-based.
    ///
    /// Vertex colour fields are the exception, using a two-digit suffix.
    #[must_use]
    pub fn level_field_name(&self, level: i32) -> String {
        if level <= 0 {
            return self.field.clone();
        }
        if matches!(self.field.as_str(), "vertR" | "vertG" | "vertB") {
            format!("{}{:02}", self.field, level)
        } else {
            format!("{}{}", self.field, level)
        }
    }

    /// Applies the `index` semantics to an existing value.
    ///
    /// A negative index replaces the whole value; otherwise the existing string
    /// is split on commas and the element at that position is replaced.
    #[must_use]
    pub fn apply_index(existing: &str, index: i32, new_value: &str) -> String {
        if index < 0 {
            return new_value.to_string();
        }
        let mut parts: Vec<String> = existing.split(',').map(|s| s.trim().to_string()).collect();
        let slot = index as usize;
        if parts.len() <= slot {
            parts.resize(slot + 1, String::new());
        }
        parts[slot] = new_value.to_string();
        parts.join(",")
    }
}

/// Whether modifiers of a category carry level and data-indicator fields.
///
/// Hard-coded, because the file does not say: only abilities, doodads and
/// upgrades do. Units, items, destructables and buffs do not.
#[must_use]
pub const fn has_level_in_binary(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Ability | ObjectKind::Doodad | ObjectKind::Upgrade
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_and_unit_share_the_same_metadata_slk() {
        assert_eq!(
            ObjectKind::Item.metadata_slk(),
            ObjectKind::Unit.metadata_slk()
        );
    }

    #[test]
    fn doodad_metadata_lives_in_the_doodads_directory() {
        assert_eq!(ObjectKind::Doodad.metadata_dir(), "Doodads");
        assert_eq!(ObjectKind::Unit.metadata_dir(), "Units");
    }

    #[test]
    fn every_kind_maps_to_a_distinct_map_file() {
        let mut files: Vec<&str> = ObjectKind::ALL.iter().map(|k| k.map_file()).collect();
        files.sort_unstable();
        let before = files.len();
        files.dedup();
        assert_eq!(
            files.len(),
            before,
            "the seven categories need distinct map files"
        );
    }

    #[test]
    fn field_type_codes_round_trip() {
        for code in 0..4 {
            assert_eq!(FieldType::from_code(code).code(), code);
        }
        assert_eq!(FieldType::from_code(9), FieldType::Unknown(9));
        assert_eq!(FieldType::Unknown(9).code(), 9);
    }

    #[test]
    fn string_type_has_no_fixed_width() {
        assert_eq!(FieldType::Integer.fixed_width(), Some(4));
        assert_eq!(FieldType::Real.fixed_width(), Some(4));
        assert_eq!(FieldType::Unreal.fixed_width(), Some(4));
        assert_eq!(FieldType::String.fixed_width(), None);
    }

    #[test]
    fn only_three_kinds_have_level_in_binary() {
        assert!(has_level_in_binary(ObjectKind::Ability));
        assert!(has_level_in_binary(ObjectKind::Doodad));
        assert!(has_level_in_binary(ObjectKind::Upgrade));
        assert!(!has_level_in_binary(ObjectKind::Unit));
        assert!(!has_level_in_binary(ObjectKind::Item));
        assert!(!has_level_in_binary(ObjectKind::Destructable));
        assert!(!has_level_in_binary(ObjectKind::Buff));
    }

    #[test]
    fn index_minus_one_replaces_the_whole_value() {
        assert_eq!(FieldMeta::apply_index("a,b,c", -1, "z"), "z");
    }

    #[test]
    fn index_replaces_one_comma_separated_slot() {
        assert_eq!(FieldMeta::apply_index("AInv,SCc1", 1, "Buhb"), "AInv,Buhb");
        assert_eq!(FieldMeta::apply_index("AInv,SCc1", 0, "X"), "X,SCc1");
    }

    #[test]
    fn index_past_the_end_pads_with_empty_slots() {
        assert_eq!(FieldMeta::apply_index("a", 3, "d"), "a,,,d");
    }

    #[test]
    fn level_field_names_are_one_based() {
        let meta = FieldMeta {
            id: FourCC::from_str_lossy("Ncl1"),
            field: "Name".into(),
            slk: None,
            index: None,
            repeat: None,
            data: None,
            type_name: None,
            string_ext: false,
            display_key: None,
            use_specific: None,
            can_be_empty: None,
            min_val: None,
            max_val: None,
            has_level: true,
        };
        assert_eq!(meta.level_field_name(1), "Name1");
        assert_eq!(meta.level_field_name(4), "Name4");
        assert_eq!(meta.level_field_name(0), "Name");
    }

    #[test]
    fn vertex_colour_fields_use_two_digit_suffixes() {
        let mut meta = FieldMeta {
            id: FourCC::from_str_lossy("vcrt"),
            field: "vertR".into(),
            slk: None,
            index: None,
            repeat: None,
            data: None,
            type_name: None,
            string_ext: false,
            display_key: None,
            use_specific: None,
            can_be_empty: None,
            min_val: None,
            max_val: None,
            has_level: true,
        };
        assert_eq!(meta.level_field_name(3), "vertR03");
        meta.field = "Other".into();
        assert_eq!(meta.level_field_name(3), "Other3");
    }

    #[test]
    fn hero_ids_are_detected_by_uppercase_first_letter() {
        assert!(ObjectKind::is_hero_id(FourCC::from_str_lossy("Hpal")));
        assert!(!ObjectKind::is_hero_id(FourCC::from_str_lossy("hfoo")));
    }

    #[test]
    fn sort_rank_puts_uppercase_ids_first() {
        let hero = ObjectKind::sort_rank(FourCC::from_str_lossy("Hpal"));
        let unit = ObjectKind::sort_rank(FourCC::from_str_lossy("hfoo"));
        assert!(hero < unit, "uppercase identifiers must sort first");
    }
}

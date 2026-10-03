//! What an object field is called, and what kind of value it holds.
//!
//! # The chain this closes
//!
//! An object file stores `unam`, and a reader wants 「名字」. Three hops, and every one of them is a
//! fact about the game's data rather than a convention:
//!
//! ```text
//! unam                        the field id, as stored
//!   → WESTRING_UEVAL_UNAM     `*MetaData.slk`'s displayName column, a *key*
//!   → 名字                     `UI\WorldEditStrings.txt`
//! ```
//!
//! ⚠️ **The middle hop is why this cannot be a table in the interface.** The metadata holds a
//! `WESTRING_*` key and the text lives in another file, so a hard-coded `unam → "Name"` in the UI
//! would be right for English and wrong for every other language — and it would be a second copy of
//! a mapping the game already ships.
//!
//! # The type column, which is what makes values resolvable
//!
//! The same row carries a `type` word, and it is the thing that says whether a value is **a list of
//! object ids**:
//!
//! | Field | `type` | Value |
//! | --- | --- | --- |
//! | `unam` | `string` | text, or a `TRIGSTR_nnn` reference |
//! | `uabi` | `abilityList` | `A04M,A03Y,Ahrp` — ids, one per element |
//! | `ubui` | `unitList` | `h002,h001,n000` |
//! | `upgr` | `upgradeList` | `R01G,R00T` |
//! | `ua1t` | `attackType` | `hero` — a *word*, whose text is in `UI\UnitEditorData.txt` |
//!
//! Measured, not assumed: those are the four shapes that appear in a real map's object data, and
//! they need three different treatments. The type word is preferred over reading a prefix off the
//! field id (`u` = unit, `i` = item) because the metadata states it, and a prefix would be a guess
//! that happens to work until an ability's field is named `a…`.
//!
//! # What is cached, and why it has to be
//!
//! Resolving a label per field, per row, would search 267 rows of `UnitMetaData.slk` and then a
//! 370 KB string table with a linear scan. Both are done **once**, at load, into one flat map per
//! kind — see [`FieldFacts`]. The load is what costs; a lookup afterwards is a `HashMap` hit.

use std::collections::HashMap;

use war3_core::AssetSource;
use war3_meta::{EditorData, MetaTable, ObjectKind};

use crate::strings::WorldStrings;

/// What a field is called, and what kind of value it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldFact {
    /// The label a reader sees, already resolved from its `WESTRING_*` key.
    ///
    /// `None` when the metadata names a key the string table does not define. The field id is then
    /// what a caller shows, which is the same degradation everything else in this crate follows.
    pub label: Option<String>,
    /// The `type` word from the metadata, e.g. `abilityList`.
    ///
    /// A `String` rather than an enum because it is the game's vocabulary, not this crate's: an
    /// unknown word is data to pass on, not a parse failure.
    pub type_name: Option<String>,
    /// Whether values may be `TRIGSTR_nnn` references.
    ///
    /// From the metadata's `stringExt` column. It decides whether a value goes through the map's
    /// string table before anything else is tried.
    pub string_ext: bool,
}

/// Every field's label and type, per kind, resolved once.
#[derive(Debug, Default, Clone)]
pub struct FieldFacts {
    /// Per kind, lower-cased field id to what is known about it.
    by_kind: HashMap<u8, HashMap<String, FieldFact>>,
    /// `(type word, value word)` to the text or `WESTRING_*` key for it.
    ///
    /// ⚠️ Built by **inverting** `UI\UnitEditorData.txt`, because that file is keyed by number and
    /// walks the other way round:
    ///
    /// ```text
    /// [attackType]
    /// 01=hero,WESTRING_UE_ATTACKTYPE_HERO
    /// ```
    ///
    /// The metadata says a field is an `attackType` and the object data stores `hero`. Neither is
    /// the file's own key — `01` is — so a lookup by value needs this table rather than
    /// `EditorData::get`, which answers by number. Using `get` directly was the first attempt and it
    /// silently returned nothing for every word-valued field.
    words: HashMap<(String, String), String>,
}

impl FieldFacts {
    /// Reads the metadata and string tables and resolves every label once.
    ///
    /// ⚠️ This is the expensive call — six `.slk` tables and a 370 KB string table — and it is
    /// deliberately separate from a lookup. `Resolver::load` makes it once per installation.
    #[must_use]
    pub fn load(source: &dyn AssetSource, strings: &WorldStrings) -> Self {
        let meta = war3_meta::MetaTableSet::load_from_assets(source);
        let editor = EditorData::load_from_assets(source);

        let mut by_kind: HashMap<u8, HashMap<String, FieldFact>> = HashMap::new();
        for kind in ObjectKind::ALL {
            let Some(table) = meta.get(kind) else {
                continue;
            };
            let fields = by_kind.entry(kind_key(kind)).or_default();
            for field in table.iter() {
                let fact = FieldFact {
                    label: field
                        .display_key
                        .as_deref()
                        .and_then(|key| strings.get(key))
                        .map(str::to_string),
                    type_name: field.type_name.clone(),
                    string_ext: field.string_ext,
                };
                fields.insert(field.id.to_string().to_lowercase(), fact);
            }
        }

        // ⚠️ Only the type words that some field actually uses are inverted: `UnitEditorData.txt` has
        // far more sections than the object metadata references, and walking all of them would read
        // tables nothing can ask about.
        let mut type_names: Vec<&str> = by_kind
            .values()
            .flat_map(|fields| fields.values())
            .filter_map(|fact| fact.type_name.as_deref())
            .collect();
        type_names.sort_unstable();
        type_names.dedup();

        let mut words = HashMap::new();
        for type_name in type_names {
            // The file's entries are keyed `00`, `01`, … — two digits, which is why the walk is
            // bounded rather than a general iterator. `EditorData` answers by that key and has no
            // way to enumerate a section, so inverting it means asking for each key in turn.
            for index in 0..100u32 {
                let Some(entry) = editor.get(type_name, &format!("{index:02}")) else {
                    continue;
                };
                // `hero,WESTRING_UE_ATTACKTYPE_HERO` — the stored value first, then its text.
                let Some((value, text)) = entry.split_once(',') else {
                    continue;
                };
                words.insert(
                    (type_name.to_string(), value.trim().to_lowercase()),
                    text.trim().to_string(),
                );
            }
        }

        Self { by_kind, words }
    }

    /// An empty table, for a machine with no game installed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What is known about one field.
    #[must_use]
    pub fn fact(&self, kind: ObjectKind, field_id: &str) -> Option<&FieldFact> {
        self.by_kind
            .get(&kind_key(kind))?
            .get(&field_id.to_lowercase())
    }

    /// The `WESTRING_*` key (or literal text) for a word-valued field, e.g. `attackType` + `hero`.
    ///
    /// The result is usually a key and not the text — measured:
    /// `hero → WESTRING_UE_ATTACKTYPE_HERO`, whose text is 英雄. A caller resolves the second hop
    /// through the same string table every other label comes from; see `Resolver::word`, which is
    /// the API for that and does both.
    #[must_use]
    pub fn word_text(&self, type_name: &str, value: &str) -> Option<&str> {
        self.words
            .get(&(type_name.to_string(), value.trim().to_lowercase()))
            .map(String::as_str)
    }

    /// How many word mappings were inverted, for a report or a test.
    #[must_use]
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// How many fields are known, across every kind.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_kind.values().map(HashMap::len).sum()
    }

    /// Whether nothing is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_kind.values().all(HashMap::is_empty)
    }
}

/// A stable key for a kind, matching `war3_object::names`.
fn kind_key(kind: ObjectKind) -> u8 {
    match kind {
        ObjectKind::Unit => 0,
        ObjectKind::Item => 1,
        ObjectKind::Destructable => 2,
        ObjectKind::Doodad => 3,
        ObjectKind::Ability => 4,
        ObjectKind::Buff => 5,
        ObjectKind::Upgrade => 6,
    }
}

/// The object kind a list-valued field's elements belong to.
///
/// ⚠️ Read from the **type word**, not from the field id's first letter. The metadata states the
/// element kind — `uabi` is an `abilityList` — while a prefix rule (`u` = unit) would be a
/// convention this crate invented that happens to hold for the fields someone looked at.
///
/// Returns `None` for any other type, so a caller resolves ids only for a value that really is a
/// list of them.
#[must_use]
pub fn list_element_kind(type_name: &str) -> Option<ObjectKind> {
    match type_name {
        "abilityList" => Some(ObjectKind::Ability),
        "unitList" => Some(ObjectKind::Unit),
        "upgradeList" => Some(ObjectKind::Upgrade),
        _ => None,
    }
}

/// Reads one kind's metadata table, for a caller that wants the table itself.
#[must_use]
pub fn table_for(kind: ObjectKind, tables: &war3_meta::MetaTableSet) -> Option<&MetaTable> {
    tables.get(kind)
}

/// How a field's value should be read, which decides who can turn it into text.
///
/// # Why this is a type rather than a boolean
///
/// A value is not "a list or not": measured against a real map's object data there are three shapes,
/// and they need three different treatments by two different layers.
///
/// | Shape | Example | Who resolves it |
/// | --- | --- | --- |
/// | [`ValueShape::Single`] | `unam` = `TRIGSTR_542`, `uhpm` = `100` | either — it is one lookup |
/// | [`ValueShape::IdList`] | `uabi` = `Avul,A000,A014` | the interface, one batched name lookup |
/// | [`ValueShape::WordList`] | `ua1g` = `air,debris,…` | the interface, one lookup per word (each may itself be a key) |
///
/// ⚠️ The interface does the lists because it has a **coalescing** resolver: 121 rows asking about the
/// same six abilities make one round trip. The backend resolving them would be one call per row or a
/// second batching mechanism. Single values are the backend's because a `TRIGSTR_` reference needs the
/// map's string table, which the interface does not have.
///
/// The classification is a function here rather than a rule in each layer for the reason everything
/// else in this file is: two copies of "does this type hold ids" disagree eventually, and the symptom
/// would be a column of unresolved codes with no error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueShape {
    /// One value: a word, a reference or a number.
    Single,
    /// Comma-separated **object ids**. `techList` is here rather than with the singles because it holds
    /// ids of *any* of four kinds, which is a lookup question and not a text one.
    IdList,
    /// Comma-separated **words** — flag names, each a key in the editor's own table.
    WordList,
}

/// Classifies a field's value from its metadata type word.
///
/// `None` (a field the metadata does not describe) is [`ValueShape::Single`]: showing it as stored is
/// the only thing that cannot be wrong.
#[must_use]
pub fn value_shape(type_name: Option<&str>) -> ValueShape {
    match type_name {
        // The three the element kind is known for, and `techList`, whose elements may be a unit, an
        // item, an ability or an upgrade.
        Some("abilityList" | "unitList" | "upgradeList" | "techList") => ValueShape::IdList,
        // ⚠️ `targetList` is a list of **words** (`air`, `ground`, `structure`) and not of ids; the
        // name is the trap. `stringList` is free text and must not be split at all, because a
        // comma inside one of its elements is data rather than a separator.
        Some("targetList") => ValueShape::WordList,
        _ => ValueShape::Single,
    }
}

/// The field id a map writes an object's display name to, per kind.
///
/// # Why this cannot be one constant
///
/// It was `unam` for every kind, and that was wrong in a way nothing reported: **an upgrade's name
/// field is `gnam`**, so a map's names for its upgrades — and for its items, destructables, doodads
/// and abilities — were never picked up at all. Only units resolved, and the symptom looked like
/// missing data rather than a wrong lookup.
///
/// Measured, by finding the metadata row whose `field` column is `Name` for each kind:
///
/// | Kind | Field | Label key |
/// | --- | --- | --- |
/// | unit, item | `unam` | `WESTRING_UEVAL_UNAM` |
/// | destructable | `bnam` | `WESTRING_BEVAL_BNAM` |
/// | doodad | `dnam` | `WESTRING_DEVAL_DNAM` |
/// | ability | `anam` | `WESTRING_AEVAL_ANAM` |
/// | upgrade | `gnam` | `WESTRING_GEVAL_GNAM` |
/// | buff | *(none)* | — |
///
/// ⚠️ `buff` has **no** name field: its `Name` row carries an empty id. This returns `None` rather
/// than a guess, so a caller skips the kind instead of reading whichever four letters were nearest.
///
/// ⚠️ The values are hard-coded rather than read from the metadata at call time, because finding them
/// *requires* the metadata and this is needed where there is none — the CLI reads object files with no
/// game directory, and the object crate must not depend on the game crate. The table is checked
/// against the real metadata by `war3 meta check`, which reports a mismatch.
#[must_use]
pub fn name_field(kind: ObjectKind) -> Option<&'static str> {
    match kind {
        ObjectKind::Unit | ObjectKind::Item => Some("unam"),
        ObjectKind::Destructable => Some("bnam"),
        ObjectKind::Doodad => Some("dnam"),
        ObjectKind::Ability => Some("anam"),
        ObjectKind::Upgrade => Some("gnam"),
        ObjectKind::Buff => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use war3_core::MemoryAssetSource;

    /// A metadata table with one field, and the string table that names it.
    fn source() -> MemoryAssetSource {
        MemoryAssetSource::new()
            .with(
                "Units\\UnitMetaData.slk",
                "C;X1;Y1;K\"ID\"\nC;X2;K\"field\"\nC;X3;K\"type\"\nC;X4;K\"stringExt\"\n\
                 C;X5;K\"displayName\"\n\
                 C;X1;Y2;K\"unam\"\nC;X2;K\"Name\"\nC;X3;K\"string\"\nC;X5;K\"WESTRING_UEVAL_UNAM\"\n\
                 C;X1;Y3;K\"uabi\"\nC;X2;K\"abilList\"\nC;X3;K\"abilityList\"\nC;X5;K\"WESTRING_UEVAL_UABI\"\n\
                 C;X1;Y4;K\"uept\"\nC;X2;K\"elevPts\"\nC;X3;K\"int\"\nC;X4;K1\nC;X5;K\"WESTRING_UEVAL_UEPT\"\n\
                 C;X1;Y5;K\"ua1t\"\nC;X2;K\"atkType1\"\nC;X3;K\"attackType\"\nC;X5;K\"WESTRING_UEVAL_UA1T\"\n",
            )
            .with(
                "UI\\WorldEditStrings.txt",
                "[WorldEditStrings]\nWESTRING_UEVAL_UNAM=名字\nWESTRING_UEVAL_UABI=技能\n\
                 WESTRING_UEVAL_UEPT=提升点数\nWESTRING_UEVAL_UA1T=攻击类型\n",
            )
            // The editor's word table. ⚠️ Keyed by **number** and holding `value,text` — see the note
            // on `FieldFacts::words` for why that shape is what makes a by-value lookup need a table.
            .with(
                "UI\\UnitEditorData.txt",
                "[attackType]\n00=unknown,WESTRING_NONE\n01=hero,WESTRING_UE_ATTACKTYPE_HERO\n",
            )
            .with(
                "UI\\WorldEditGameStrings.txt",
                "[WorldEditGameStrings]\nWESTRING_UE_ATTACKTYPE_HERO=英雄\n",
            )
    }

    /// ⚠️ The three-hop chain: field id → `WESTRING_*` key → text.
    #[test]
    fn a_field_label_comes_from_the_metadata_key_and_the_string_table() {
        let strings = WorldStrings::parse(&source().get_text("UI\\WorldEditStrings.txt").unwrap());
        let facts = FieldFacts::load(&source(), &strings);

        let fact = facts.fact(ObjectKind::Unit, "unam").expect("unam is known");
        assert_eq!(fact.label.as_deref(), Some("名字"));
        assert_eq!(fact.type_name.as_deref(), Some("string"));
    }

    /// The type word is what says a value is a list of ids, and which kind they are.
    #[test]
    fn the_type_word_names_the_element_kind() {
        let strings = WorldStrings::parse(&source().get_text("UI\\WorldEditStrings.txt").unwrap());
        let facts = FieldFacts::load(&source(), &strings);

        let ability = facts.fact(ObjectKind::Unit, "uabi").expect("uabi");
        assert_eq!(ability.type_name.as_deref(), Some("abilityList"));
        assert_eq!(
            list_element_kind(ability.type_name.as_deref().unwrap()),
            Some(ObjectKind::Ability)
        );

        // A field that is not a list resolves to nothing, so no caller splits it by accident.
        let scalar = facts.fact(ObjectKind::Unit, "unam").expect("unam");
        assert_eq!(
            list_element_kind(scalar.type_name.as_deref().unwrap()),
            None
        );
    }

    /// `stringExt` is carried through, because it decides whether a value is a `TRIGSTR_` reference.
    #[test]
    fn string_ext_is_carried() {
        let strings = WorldStrings::parse(&source().get_text("UI\\WorldEditStrings.txt").unwrap());
        let facts = FieldFacts::load(&source(), &strings);

        assert!(
            facts
                .fact(ObjectKind::Unit, "uept")
                .expect("uept")
                .string_ext
        );
        assert!(
            !facts
                .fact(ObjectKind::Unit, "unam")
                .expect("unam")
                .string_ext
        );
    }

    /// A label whose key the string table does not define is `None` rather than the key itself, so a
    /// caller shows the field id and not `WESTRING_UEVAL_NOPE`.
    #[test]
    fn an_undefined_label_key_is_none() {
        let src = MemoryAssetSource::new().with(
            "Units\\UnitMetaData.slk",
            "C;X1;Y1;K\"ID\"\nC;X5;K\"displayName\"\nC;X1;Y2;K\"unam\"\nC;X5;K\"WESTRING_MISSING\"\n",
        );
        let facts = FieldFacts::load(&src, &WorldStrings::default());
        assert_eq!(facts.fact(ObjectKind::Unit, "unam").unwrap().label, None);
    }

    /// Field ids are matched without regard to case, like every other id in this workspace.
    #[test]
    fn field_ids_match_without_case() {
        let strings = WorldStrings::parse(&source().get_text("UI\\WorldEditStrings.txt").unwrap());
        let facts = FieldFacts::load(&source(), &strings);

        for spelling in ["unam", "UNAM", "Unam"] {
            assert_eq!(
                facts
                    .fact(ObjectKind::Unit, spelling)
                    .and_then(|f| f.label.clone()),
                Some("名字".to_string()),
                "{spelling}"
            );
        }
    }

    /// A word-valued field's text comes from the editor's own table, keyed by the type word.
    #[test]
    fn a_word_value_resolves_through_the_editor_table() {
        let strings = WorldStrings::parse(&source().get_text("UI\\WorldEditStrings.txt").unwrap());
        let facts = FieldFacts::load(&source(), &strings);
        // The value in the table is a WESTRING key, and the game's own strings carry the text.
        assert_eq!(
            facts.word_text("attackType", "hero"),
            Some("WESTRING_UE_ATTACKTYPE_HERO")
        );
        // Case is ignored, like every other id and word in this workspace.
        assert_eq!(
            facts.word_text("attackType", "HERO"),
            Some("WESTRING_UE_ATTACKTYPE_HERO")
        );
        // A word the table does not define resolves to nothing rather than to a guess.
        assert_eq!(facts.word_text("attackType", "nonsense"), None);
    }

    /// No metadata at all is an empty table, not a failure: a machine without the game still parses
    /// and edits maps.
    #[test]
    fn no_metadata_is_an_empty_table() {
        let facts = FieldFacts::load(&MemoryAssetSource::new(), &WorldStrings::default());
        assert!(facts.is_empty());
        assert_eq!(facts.fact(ObjectKind::Unit, "unam"), None);
    }

    /// ⚠️ **The test `name_field` exists for.**
    ///
    /// `name_field` is a hard-coded table because the callers that need it have no metadata to ask —
    /// the CLI reads object files with no game directory, and `war3-object` must not depend on this
    /// crate. A hard-coded table that disagrees with the data it describes is the failure mode, and it
    /// is silent: the object just shows its id, which looks like missing map data.
    ///
    /// So the table is checked against the real metadata: for each kind, the field whose `field` column
    /// is `Name` must be the id `name_field` returns. That is exactly how the table was produced.
    ///
    /// Skipped when `W3_TEST_GAME` is unset, like every other test that needs the game.
    ///
    /// ⚠️ `#[cfg(feature = "mpq")]` because opening the archives is what that feature is — the field
    /// tables themselves are read from any `AssetSource`.
    #[cfg(feature = "mpq")]
    #[test]
    fn the_hard_coded_name_field_matches_the_real_metadata() {
        let Ok(root) = std::env::var("W3_TEST_GAME") else {
            return;
        };
        let assets = crate::mpq::GameAssets::open(&root);
        let meta = war3_meta::MetaTableSet::load_from_assets(&assets);

        for kind in ObjectKind::ALL {
            let Some(table) = meta.get(kind) else {
                continue;
            };
            // The row whose `field` column is `Name`, whose id is what the metadata calls the name.
            let from_metadata = table
                .iter()
                .find(|m| m.field.eq_ignore_ascii_case("Name"))
                .map(|m| m.id.to_string().to_lowercase())
                // An empty id is the metadata saying "this kind has no name field", which is what
                // `None` means.
                .filter(|id| !id.is_empty());

            assert_eq!(
                name_field(kind).map(str::to_string),
                from_metadata,
                "name_field({kind}) disagrees with {}'s `field=Name` row",
                kind.map_file()
            );
        }
    }
}

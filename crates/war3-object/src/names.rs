//! What a map calls its own objects.
//!
//! # Why this is a core concern
//!
//! An object's display name lives in **two places**, and a map's own objects have it nowhere else:
//!
//! | Object | Name | Where it is |
//! | --- | --- | --- |
//! | a Blizzard object the map does not touch | 步兵 | the game's `Units\*UnitStrings.txt` |
//! | a Blizzard object the map modifies | 守卫 (基本建造者) | the map's `.w3u`, field `unam` |
//! | an object the map creates (`hC06`) | 守卫 (基本建造者) | the map's `.w3u`, field `unam` |
//!
//! So without this, everything the author actually made shows as a four-character id — which on a
//! real map is most of what a reader wants to look at. Measured on `(6)BlizzardTD.w3x`: 207 placed
//! units, of which the game's own tables name about 20.
//!
//! # The second hop, which is not optional
//!
//! ⚠️ **`unam` is usually not text.** It is a `TRIGSTR_nnn` reference into the map's
//! `war3map.wts`, so a resolver has to carry the map's string table along. Measured: every one of
//! BlizzardTD's `unam` values is a reference — `hC06` is `TRIGSTR_117`, whose text is
//! `守卫 (基本建造者)`.
//!
//! A caller that passes no table gets the references themselves, which is the honest answer for a
//! map whose `.wts` could not be read (it may be PKWare-imploded) rather than an invented one.
//!
//! # Which field is the name
//!
//! `unam` for units and items, `unam` also for the other five kinds — the Object Editor uses one
//! field id across all seven. That is a **fact about the game's field vocabulary** rather than a
//! convention this crate invented, which is why it is a constant here and not a parameter: a caller
//! that could choose the field could choose a wrong one.
//!
//! ⚠️ The field is `unam` and not `Name`: the metadata tables map `unam` to the *label* "Name", and
//! the binary stores the id. See `war3_meta` for that mapping.

use std::collections::HashMap;

use war3_core::FourCC;
use war3_map::StringTable;

use crate::object::{FieldValue, ObjectFile};

/// The field id the Object Editor writes a display name to.
///
/// ⚠️ **Not one id for every kind.** This was `unam` for all seven, and that silently broke five of
/// them: a destructable's name field is `bnam`, an upgrade's is `gnam`, a doodad's is `dnam`, an
/// ability's is `anam`. Only units and items resolved, and the symptom — an id where a name belongs —
/// looked like missing map data rather than a wrong lookup.
///
/// It is still the default here, because this crate has no metadata to ask and must not depend on the
/// crate that does. A caller that knows the kind passes the right one to [`ObjectNames::add_with`];
/// [`names_for`] is what the CLI and the editor use.
const NAME_FIELD: &str = "unam";

/// What a map calls one of its objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectName {
    /// The object's id, as it appears in the placed-object files.
    pub id: FourCC,
    /// The name the map gives it.
    pub name: String,
}

/// Every name a map's object files set, by kind.
///
/// A `HashMap` rather than a `Vec` because the consumer's question is "what is this object called",
/// asked once per row of a table.
#[derive(Debug, Default, Clone)]
pub struct ObjectNames {
    /// Per kind, lower-cased id to name.
    by_kind: HashMap<u8, HashMap<String, String>>,
}

impl ObjectNames {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads one parsed object file, using the field id this crate knows for the kind.
    ///
    /// ⚠️ That is `unam` for units **and for items**, and wrong for the other five — see the note on
    /// [`NAME_FIELD`]. A caller that can ask the game's metadata should use [`Self::add_with`], which
    /// is what the editor and the CLI both do.
    pub fn add(&mut self, file: &ObjectFile, strings: Option<&StringTable>) {
        self.add_with(file, strings, NAME_FIELD);
    }

    /// Reads one parsed object file, taking the name from `name_field`.
    ///
    /// `name_field` is the field id that holds the display name **for this kind** — `unam`, `bnam`,
    /// `anam`, `gnam`, `dnam`. See `war3_game::name_field`, which is where the per-kind table lives.
    pub fn add_with(&mut self, file: &ObjectFile, strings: Option<&StringTable>, name_field: &str) {
        let kind = kind_key(file.kind);
        let table = self.by_kind.entry(kind).or_default();

        for object in file.table.all() {
            let Some(value) = object
                .modifications
                .iter()
                .find(|m| m.field.to_string().eq_ignore_ascii_case(name_field))
                .map(|m| &m.value)
            else {
                // ⚠️ No name set, so the game's own name stands. This is the common case for a
                // modified Blizzard object that only had its hit points changed, and inventing an
                // entry here would override the localised name with nothing.
                continue;
            };

            let FieldValue::String(raw) = value else {
                // A name field holds a string; a binary that stores it as a number is not something
                // this build understands, and guessing would produce a name nobody wrote.
                continue;
            };

            let name = resolve(raw, strings);
            if name.is_empty() {
                continue;
            }
            table.insert(object.id.to_string().to_lowercase(), name);
        }
    }

    /// The name a map gives an object, or `None` when the map does not name it.
    ///
    /// Case is ignored: object files and placed-object files disagree about the case of an id —
    /// `hC06` in one and `hC06` in the other is the same object, and the game treats it as one.
    #[must_use]
    pub fn get(&self, kind: war3_meta::ObjectKind, id: &str) -> Option<&str> {
        self.by_kind
            .get(&kind_key(kind))?
            .get(&id.to_lowercase())
            .map(String::as_str)
    }

    /// Every name, as `(kind, id, name)`.
    ///
    /// ⚠️ The ids come back **lower-cased**, because that is how they are keyed. A caller that needs
    /// the id as the map spells it should read the object table for that; what this iterator is for
    /// is handing a whole map's names to something that will look them up by folded id, which is
    /// what the game does.
    pub fn iter(&self) -> impl Iterator<Item = (war3_meta::ObjectKind, &str, &str)> {
        self.by_kind.iter().flat_map(|(key, names)| {
            let kind = kind_from_key(*key);
            names
                .iter()
                .map(move |(id, name)| (kind, id.as_str(), name.as_str()))
        })
    }

    /// How many names are known, across every kind.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_kind.values().map(HashMap::len).sum()
    }

    /// Whether the map names none of its objects.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_kind.values().all(HashMap::is_empty)
    }
}

/// Resolves a `TRIGSTR_nnn` reference, or returns the text.
///
/// A reference with no table to resolve it against is returned **as written** — `TRIGSTR_117` — and
/// not as the empty string: a reader who sees a reference learns that the map's string table was
/// unreadable, while a blank cell tells them nothing.
fn resolve(value: &str, strings: Option<&StringTable>) -> String {
    let Some(strings) = strings else {
        return value.to_string();
    };
    // `resolve` needs somewhere to put its complaint. It is a scratch collection because this
    // crate has no diagnostics of its own to add it to — the caller reading a map already collects
    // the string table's problems when it parses the table.
    let mut scratch = war3_core::diag::Diagnostics::new();
    strings.resolve(value, &mut scratch)
}

/// A stable key for a kind, since `ObjectKind` is not `Hash`.
///
/// Indexed by position in [`war3_meta::ObjectKind::ALL`], which is the crate's own exhaustive list —
/// so a kind added there is a kind this function has a slot for, and the compiler says so when the
/// `match` stops covering it.
fn kind_key(kind: war3_meta::ObjectKind) -> u8 {
    use war3_meta::ObjectKind;
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

/// The inverse of [`kind_key`].
///
/// ⚠️ Kept beside it so the two are read together: a key that maps to the wrong kind would file names
/// under an object category they do not belong to, and the symptom — an ability named like a unit —
/// would look like a data problem rather than a lookup one.
fn kind_from_key(key: u8) -> war3_meta::ObjectKind {
    use war3_meta::ObjectKind;
    match key {
        1 => ObjectKind::Item,
        2 => ObjectKind::Destructable,
        3 => ObjectKind::Doodad,
        4 => ObjectKind::Ability,
        5 => ObjectKind::Buff,
        6 => ObjectKind::Upgrade,
        // 0 and anything a future key could be: unit is the kind an unrecognised id is most likely to
        // be, and the fallback is visible rather than a panic.
        _ => ObjectKind::Unit,
    }
}

/// Reads a map's whole object data into one table of names, taking the name field per kind from
/// `name_field`.
///
/// # Why this exists rather than a loop at each call site
///
/// The two things that are easy to get wrong here are both silent:
///
/// 1. **The name field differs per kind** — `unam`, `bnam`, `anam`, `gnam`, `dnam` — and using one for
///    all of them resolves units and quietly fails for the rest. See [`NAME_FIELD`].
/// 2. **A kind with no name field must be skipped**, not read with a fallback. Buffs have none.
///
/// `name_field` returns `None` for such a kind, and a caller that wrote its own loop would have to
/// remember to check. Folding the loop in here means the check happens once, where it is testable.
///
/// `files` is `(kind, parsed file)` for whatever the map actually has; a map omits the files it does
/// not use.
pub fn names_for<'a, I>(
    files: I,
    strings: Option<&StringTable>,
    name_field: fn(war3_meta::ObjectKind) -> Option<&'static str>,
) -> ObjectNames
where
    I: IntoIterator<Item = (war3_meta::ObjectKind, &'a ObjectFile)>,
{
    let mut names = ObjectNames::new();
    for (kind, file) in files {
        let Some(field) = name_field(kind) else {
            continue;
        };
        names.add_with(file, strings, field);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use war3_meta::ObjectKind;

    /// A minimal object file with one custom object called `name`.
    ///
    /// Built from bytes rather than from the model, so the test exercises the same path a real map
    /// does. Three details are easy to get wrong, and the first version of this fixture got all
    /// three — producing a file that parsed into a different object than intended, with no error:
    ///
    /// - ⚠️ The **custom** table stores the *parent* first and the **new object's id second**.
    ///   `ObjectTable::parse` reads two ids and swaps them for a custom table, so `hfoo` then `hC06`
    ///   is an object `hC06` inheriting `hfoo`. Writing them the other way round makes an object
    ///   called `hfoo` that inherits `hC06` — which parses, and is backwards.
    /// - A string value is a **C string**, terminated by a zero byte, not length-prefixed.
    /// - The four bytes the game ignores follow it.
    fn file_with(kind: ObjectKind, id: &str, name: &str) -> ObjectFile {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2i32.to_le_bytes()); // version
        bytes.extend_from_slice(&0i32.to_le_bytes()); // original table: empty
        bytes.extend_from_slice(&1i32.to_le_bytes()); // custom table: one object
        bytes.extend_from_slice(b"hfoo"); // the parent, which a custom row stores first
        bytes.extend_from_slice(id.as_bytes()); // the new object's id second
        bytes.extend_from_slice(&1i32.to_le_bytes()); // one modification
        bytes.extend_from_slice(b"unam"); // the name field
        bytes.extend_from_slice(&3i32.to_le_bytes()); // value type 3: string
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0); // the C string's terminator
        bytes.extend_from_slice(&[0u8; 4]); // the four bytes the game ignores

        let file = ObjectFile::parse(kind, &bytes).expect("the fixture parses");
        // ⚠️ Asserted, not assumed. A fixture whose ids are swapped parses perfectly and tests the
        // wrong object, which is exactly what happened here first.
        assert_eq!(
            file.table.all().next().map(|o| o.id.to_string()),
            Some(id.to_string()),
            "the fixture must contain an object called {id}"
        );
        file
    }

    /// The plain case: the map wrote text into `unam`.
    #[test]
    fn a_name_written_as_text_is_taken_as_is() {
        let file = file_with(ObjectKind::Unit, "hC06", "守卫");
        let mut names = ObjectNames::new();
        names.add(&file, None);

        assert_eq!(names.get(ObjectKind::Unit, "hC06"), Some("守卫"));
        assert_eq!(names.len(), 1);
        // And an id the map does not name is not invented.
        assert_eq!(names.get(ObjectKind::Unit, "hfoo"), None);
    }

    /// ⚠️ The common case: `unam` is a `TRIGSTR_nnn` reference, so the map's string table is what
    /// makes the name readable.
    #[test]
    fn a_trigstr_reference_is_resolved_through_the_map_string_table() {
        let file = file_with(ObjectKind::Unit, "hC06", "TRIGSTR_117");
        // The `.wts` text for index 117.
        let table = StringTable::parse(b"STRING 117\r\n{\r\n\xE5\xAE\x88\xE5\x8D\xAB\r\n}\r\n");

        let mut names = ObjectNames::new();
        names.add(&file, Some(&table));

        assert_eq!(names.get(ObjectKind::Unit, "hC06"), Some("守卫"));
    }

    /// ⚠️ A reference with no table to resolve it against comes back **as written**, not blank: a
    /// reader who sees `TRIGSTR_117` learns the string table was unreadable, while a blank cell tells
    /// them nothing.
    #[test]
    fn an_unresolvable_reference_is_kept_rather_than_blanked() {
        let file = file_with(ObjectKind::Unit, "hC06", "TRIGSTR_117");
        let mut names = ObjectNames::new();
        names.add(&file, None);

        assert_eq!(names.get(ObjectKind::Unit, "hC06"), Some("TRIGSTR_117"));
    }

    /// Case is ignored on both sides, because the object files and the placed-object files disagree
    /// about it and the game treats the two spellings as one object.
    #[test]
    fn ids_match_without_regard_to_case() {
        let file = file_with(ObjectKind::Unit, "hC06", "守卫");
        let mut names = ObjectNames::new();
        names.add(&file, None);

        for spelling in ["hC06", "HC06", "hc06"] {
            assert_eq!(
                names.get(ObjectKind::Unit, spelling),
                Some("守卫"),
                "{spelling}"
            );
        }
    }

    /// Two kinds are two tables: `hfoo` the unit and `hfoo` an item are different objects.
    #[test]
    fn names_are_per_kind() {
        let mut names = ObjectNames::new();
        names.add(&file_with(ObjectKind::Unit, "hC06", "守卫"), None);
        names.add(&file_with(ObjectKind::Item, "hC06", "守卫之剑"), None);

        assert_eq!(names.get(ObjectKind::Unit, "hC06"), Some("守卫"));
        assert_eq!(names.get(ObjectKind::Item, "hC06"), Some("守卫之剑"));
        // A kind the map said nothing about has nothing.
        assert_eq!(names.get(ObjectKind::Ability, "hC06"), None);
    }

    /// An empty map names nothing, and says so rather than looking like a failure.
    #[test]
    fn no_names_is_an_empty_table_rather_than_an_error() {
        let names = ObjectNames::new();
        assert!(names.is_empty());
        assert_eq!(names.len(), 0);
        assert_eq!(names.get(ObjectKind::Unit, "hC06"), None);
    }
}

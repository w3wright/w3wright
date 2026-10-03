//! One symbol table per object kind.
//!
//! # The three places an object name lives, and why there are three
//!
//! Measured against a 1.27 installation rather than assumed. These are the files an ID-to-name
//! lookup has to consult, and they are **not** the same file for all seven kinds:
//!
//! | Kind | Where the name is | What is stored there |
//! | --- | --- | --- |
//! | unit, item, ability, buff, upgrade | `Units\*UnitStrings.txt`, `Units\*AbilityStrings.txt`, `Units\*UpgradeStrings.txt` | the text, directly |
//! | doodad | `Doodads\Doodads.slk`, column `Name` | a `WESTRING_*` **key** |
//! | destructable | `Units\DestructableData.slk`, column `Name` | a `WESTRING_*` **key** |
//!
//! So doodads and destructables need a second hop through [`super::strings::WorldStrings`], and
//! the other five do not. That is not a design decision anyone would make twice — it is what the
//! game's own data does, and it is why this table exists rather than one uniform loader.
//!
//! ⚠️ **A name is not always in `WorldEditStrings.txt` either.** `WESTRING_DEST_ASHENVALE_TREE_WALL`
//! is in both that file and `WorldEditGameStrings.txt`, with *different* text, and the editor's
//! answer is the `WorldEditStrings` one. See [`super::strings`] for the order and the measurement.
//!
//! # What is deliberately absent
//!
//! The `.slk` files also carry a `comment`/`comment(s)` column holding an English label — `hfoo`
//! has `comment(s) = Footman`. It is **not** used. It is an annotation for Blizzard's own tools,
//! it is English on every installation, and showing it beside a localised name would be showing a
//! third thing that is neither the ID nor the name.

use std::collections::HashMap;

use war3_meta::ObjectKind;

use crate::sylk::SylkTable;

/// `concat!` needs literals, so a `const` prefix cannot be pasted into one. These two macros say the
/// directory once per path and are the reason the tables below read as file names rather than as
/// long strings: whether a table sits under `Units\` or `Doodads\` is the one structural fact here
/// worth seeing at a glance, and it is visible at the start of every line.
macro_rules! units {
    ($name:literal) => {
        concat!("Units\\", $name)
    };
}

macro_rules! doodads {
    ($name:literal) => {
        concat!("Doodads\\", $name)
    };
}

/// The `*Strings.txt` files, as `(path, kind)`.
///
/// Nine files for five kinds, because Blizzard splits them by race *and* by kind: there is no
/// `UnitStrings.txt`, only a human/orc/undead/nightelf/neutral/campaign one. All nine are read and
/// merged into one table per kind, because a map may place any race's object and the caller asking
/// "what is `hfoo`" should not have to know that Footmen are Human.
///
/// ⚠️ The table's order matters for nothing here — the paths are disjoint, and no ID appears in two
/// of them. It is alphabetical by kind so that a reader can see at a glance what is covered.
const STRING_FILES: &[(&str, ObjectKind)] = &[
    // Units.
    (units!("CampaignUnitStrings.txt"), ObjectKind::Unit),
    (units!("HumanUnitStrings.txt"), ObjectKind::Unit),
    (units!("NeutralUnitStrings.txt"), ObjectKind::Unit),
    (units!("NightElfUnitStrings.txt"), ObjectKind::Unit),
    (units!("OrcUnitStrings.txt"), ObjectKind::Unit),
    (units!("UndeadUnitStrings.txt"), ObjectKind::Unit),
    // Items.
    (units!("ItemStrings.txt"), ObjectKind::Item),
    // Abilities. `CommonAbilityStrings` holds text shared between races.
    (units!("CampaignAbilityStrings.txt"), ObjectKind::Ability),
    (units!("CommonAbilityStrings.txt"), ObjectKind::Ability),
    (units!("HumanAbilityStrings.txt"), ObjectKind::Ability),
    (units!("ItemAbilityStrings.txt"), ObjectKind::Ability),
    (units!("NeutralAbilityStrings.txt"), ObjectKind::Ability),
    (units!("NightElfAbilityStrings.txt"), ObjectKind::Ability),
    (units!("OrcAbilityStrings.txt"), ObjectKind::Ability),
    (units!("UndeadAbilityStrings.txt"), ObjectKind::Ability),
    // Buffs.
    (units!("AbilityBuffStrings.txt"), ObjectKind::Buff),
    // Upgrades.
    (units!("CampaignUpgradeStrings.txt"), ObjectKind::Upgrade),
    (units!("HumanUpgradeStrings.txt"), ObjectKind::Upgrade),
    (units!("NeutralUpgradeStrings.txt"), ObjectKind::Upgrade),
    (units!("NightElfUpgradeStrings.txt"), ObjectKind::Upgrade),
    (units!("OrcUpgradeStrings.txt"), ObjectKind::Upgrade),
    (units!("UndeadUpgradeStrings.txt"), ObjectKind::Upgrade),
];

/// The `.slk` tables whose `Name` column holds a key: `(kind, path, the table's ID column)`.
///
/// Two kinds, both one file, and each names its ID column differently — that is the file's choice,
/// not a convention this code invented. See the module documentation for why these two kinds do not
/// use Strings files.
const KEYED_SLK: &[(ObjectKind, &str, &str)] = &[
    (ObjectKind::Doodad, doodads!("Doodads.slk"), "doodID"),
    (
        ObjectKind::Destructable,
        units!("DestructableData.slk"),
        "DestructableID",
    ),
];

/// The column both of those tables call their display name.
///
/// The same word in both, which is why it is one constant rather than a third field.
const SLK_NAME_COLUMN: &str = "Name";

/// The text stored against one ID, and whether it is the text or a key to it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stored {
    /// The display text, as the `*Strings.txt` files store it.
    Text(String),
    /// A `WESTRING_*` key, as the two `.slk` `Name` columns store it.
    Key(String),
}

/// Object names by kind and ID.
///
/// # Why `String` keys rather than `FourCC`
///
/// `war3_core::FourCC` is four bytes and would halve the memory. It is not used because the data
/// these are built from is **not reliably four bytes**: a map's object file can hold an ID of any
/// length, and the corpus contains rows whose ID column is not four bytes — `war3 meta check`
/// reports one such row in the metadata tables themselves and skips it. A `FourCC` that has to be
/// padded or truncated is a lookup that silently misses, which is the one failure this table
/// cannot afford.
#[derive(Debug, Default, Clone)]
pub struct Names {
    /// Per kind, lower-cased ID to what is stored for it.
    by_kind: HashMap<ObjectKind, HashMap<String, Stored>>,
}

impl Names {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads every name table it can find.
    ///
    /// A missing file is skipped rather than reported: an installation may legitimately lack a
    /// localisation pack, and a partly-filled table is more useful than none. What was found is
    /// available through [`Names::stats`], so a caller can tell "no names at all" from "names".
    #[must_use]
    pub fn load(source: &dyn war3_core::AssetSource) -> Self {
        let mut names = Self::new();

        for (path, kind) in STRING_FILES {
            let Some(text) = source.get_text(path) else {
                continue;
            };
            let entries = crate::strings::parse_sections(&text);
            let table = names.by_kind.entry(*kind).or_default();
            for (id, fields) in entries {
                // `Name` is the display name. The other fields in these blocks — `Tip`,
                // `Ubertip`, `Hotkey` — are also the object's, but a table is not the place to
                // decide which of them a caller wanted.
                if let Some(name) = fields.get("name") {
                    table.insert(id.to_lowercase(), Stored::Text(name.clone()));
                }
            }
        }

        for (kind, path, id_column) in KEYED_SLK {
            let Some(bytes) = source.get(path) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            let Some(table) = SylkTable::parse(&text) else {
                continue;
            };
            let Some(id_col) = table.column_number(id_column) else {
                continue;
            };
            let Some(name_col) = table.column_number(SLK_NAME_COLUMN) else {
                continue;
            };
            let target = names.by_kind.entry(*kind).or_default();
            for row in table.rows() {
                let (Some(id), Some(name)) = (row.get(id_col), row.get(name_col)) else {
                    continue;
                };
                if id.is_empty() || name.is_empty() {
                    continue;
                }
                target.insert(id.to_lowercase(), Stored::Key(name.to_string()));
            }
        }

        names
    }

    /// The name stored for one ID, before keys are resolved and before a map's own override.
    ///
    /// Returns the stored text when the kind keeps its names as text. For a kind that stores keys,
    /// use [`Names::key_for`] and resolve it against [`crate::strings::WorldStrings`] — this
    /// returns `None` for those, because handing back `WESTRING_DOOD_APMS` where a name was
    /// promised is worse than handing back nothing.
    #[must_use]
    pub fn text_for(&self, kind: ObjectKind, id: &str) -> Option<&str> {
        match self.by_kind.get(&kind)?.get(&id.to_lowercase())? {
            Stored::Text(text) => Some(text),
            Stored::Key(_) => None,
        }
    }

    /// The key stored for one ID, for a kind that stores keys.
    #[must_use]
    pub fn key_for(&self, kind: ObjectKind, id: &str) -> Option<&str> {
        match self.by_kind.get(&kind)?.get(&id.to_lowercase())? {
            Stored::Key(key) => Some(key),
            Stored::Text(_) => None,
        }
    }

    /// How many IDs are known, per kind and in total.
    #[must_use]
    pub fn stats(&self) -> NameStats {
        let mut per_kind: Vec<(ObjectKind, usize)> = self
            .by_kind
            .iter()
            .map(|(kind, table)| (*kind, table.len()))
            .collect();
        // A stable order, because this is printed: a report that reshuffles between runs cannot be
        // compared with the last one.
        per_kind.sort_by_key(|(kind, _)| format!("{kind:?}"));
        NameStats {
            total: per_kind.iter().map(|(_, n)| n).sum(),
            per_kind,
        }
    }

    /// How many IDs one kind has.
    #[must_use]
    pub fn len_of(&self, kind: ObjectKind) -> usize {
        self.by_kind.get(&kind).map_or(0, HashMap::len)
    }
}

/// What a [`Names`] table holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameStats {
    /// How many IDs are known across all kinds.
    pub total: usize,
    /// Per kind, in a stable order.
    pub per_kind: Vec<(ObjectKind, usize)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use war3_core::MemoryAssetSource;

    /// The unit table is a `*Strings.txt`, so the text is stored directly.
    #[test]
    fn a_strings_file_yields_the_name_itself() {
        let source = MemoryAssetSource::new().with(
            "Units\\HumanUnitStrings.txt",
            b"[hfoo]\nName=Footman\nHotkey=F\n\n[hpea]\nName=Peasant\n",
        );
        let names = Names::load(&source);

        assert_eq!(names.text_for(ObjectKind::Unit, "hfoo"), Some("Footman"));
        assert_eq!(names.text_for(ObjectKind::Unit, "hpea"), Some("Peasant"));
        // And nothing is stored as a key, so the key lookup is empty for this kind.
        assert_eq!(names.key_for(ObjectKind::Unit, "hfoo"), None);
    }

    /// Lookup is case-insensitive, because the same object is written `hfoo` in one file and
    /// `Hfoo` in another and the game treats them as one ID.
    #[test]
    fn ids_are_matched_without_regard_to_case() {
        let source =
            MemoryAssetSource::new().with("Units\\HumanUnitStrings.txt", b"[hfoo]\nName=Footman\n");
        let names = Names::load(&source);
        for spelling in ["hfoo", "HFOO", "HFoo"] {
            assert_eq!(names.text_for(ObjectKind::Unit, spelling), Some("Footman"));
        }
    }

    /// The doodad table stores a `WESTRING_*` key. `text_for` must **not** hand that back as if it
    /// were a name — that is the whole reason the two accessors are separate.
    #[test]
    fn a_keyed_slk_stores_a_key_and_not_a_name() {
        let source = MemoryAssetSource::new().with(
            "Doodads\\Doodads.slk",
            b"C;X1;Y1;K\"doodID\"\nC;X7;K\"Name\"\nC;X1;Y2;K\"APms\"\nC;X7;K\"WESTRING_DOOD_APMS\"\n",
        );
        let names = Names::load(&source);

        assert_eq!(
            names.key_for(ObjectKind::Doodad, "APms"),
            Some("WESTRING_DOOD_APMS")
        );
        assert_eq!(
            names.text_for(ObjectKind::Doodad, "APms"),
            None,
            "a key must never be served as a name"
        );
    }

    /// A missing file is skipped, not an error: a stripped installation still resolves what it has.
    #[test]
    fn missing_files_leave_an_empty_table_rather_than_failing() {
        let source = MemoryAssetSource::new();
        let names = Names::load(&source);
        assert_eq!(names.stats().total, 0);
        assert_eq!(names.len_of(ObjectKind::Unit), 0);
        assert_eq!(names.text_for(ObjectKind::Unit, "hfoo"), None);
    }

    /// Several files merge into one table per kind, which is what lets a caller ask about any
    /// race's object without knowing the race.
    #[test]
    fn files_of_the_same_kind_merge() {
        let source = MemoryAssetSource::new()
            .with("Units\\HumanUnitStrings.txt", b"[hfoo]\nName=Footman\n")
            .with("Units\\OrcUnitStrings.txt", b"[opeo]\nName=Peon\n")
            .with("Units\\ItemStrings.txt", b"[pghe]\nName=Potion\n");
        let names = Names::load(&source);

        assert_eq!(names.text_for(ObjectKind::Unit, "hfoo"), Some("Footman"));
        assert_eq!(names.text_for(ObjectKind::Unit, "opeo"), Some("Peon"));
        assert_eq!(names.len_of(ObjectKind::Unit), 2);
        // Items are their own kind even though their file lives under `Units\`.
        assert_eq!(names.text_for(ObjectKind::Item, "pghe"), Some("Potion"));
    }

    /// The report is ordered, so two runs can be compared with each other.
    #[test]
    fn stats_are_in_a_stable_order() {
        let source = MemoryAssetSource::new()
            .with("Units\\HumanUnitStrings.txt", b"[hfoo]\nName=Footman\n")
            .with("Units\\ItemStrings.txt", b"[pghe]\nName=Potion\n");
        let names = Names::load(&source);
        let stats = names.stats();
        assert_eq!(stats.total, 2);
        let kinds: Vec<String> = stats
            .per_kind
            .iter()
            .map(|(k, _)| format!("{k:?}"))
            .collect();
        let mut sorted = kinds.clone();
        sorted.sort();
        assert_eq!(kinds, sorted, "the report must be ordered");
    }

    /// Every path in the two tables is spelled the way the archives spell it: under `Units\` or
    /// `Doodads\`, with the extension the table is read as.
    ///
    /// This is worth a test rather than a comment because **a misspelled path is not an error
    /// anywhere in this crate.** A missing file is skipped by design, so that a stripped installation
    /// still resolves what it has — which means a typo produces exactly one symptom, every object of
    /// that kind showing its ID, and nothing that points at the cause.
    #[test]
    fn every_table_path_is_spelled_as_the_archives_spell_it() {
        for (path, _) in STRING_FILES {
            assert!(
                path.starts_with("Units\\"),
                "{path} must be under Units\\, or it will never be found"
            );
            assert!(path.ends_with(".txt"), "{path} is not a text table");
        }
        for (_, path, _) in KEYED_SLK {
            assert!(
                path.starts_with("Units\\") || path.starts_with("Doodads\\"),
                "{path} must be under Units\\ or Doodads\\"
            );
            assert!(path.ends_with(".slk"), "{path} is not a SYLK table");
        }
    }

    /// The five kinds that have Strings files are exactly the five that do not have a keyed `.slk`,
    /// and the other way round.
    ///
    /// This is the invariant behind the three-sources table in the module documentation: a kind
    /// with a file in both lists would resolve through whichever branch ran first, and a kind in
    /// neither would silently never resolve at all.
    #[test]
    fn each_kind_has_exactly_one_source() {
        for kind in ObjectKind::ALL {
            let text_files = STRING_FILES.iter().filter(|(_, k)| *k == kind).count();
            let slk_files = KEYED_SLK.iter().filter(|(k, _, _)| *k == kind).count();
            assert!(
                (text_files > 0) ^ (slk_files > 0),
                "{kind:?} has {text_files} Strings files and {slk_files} keyed .slk tables; \
                 exactly one of those must be non-zero, or its names come from nowhere or two places"
            );
        }
    }
}

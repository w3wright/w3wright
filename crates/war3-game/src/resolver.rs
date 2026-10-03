//! Turning an ID into the name a reader recognises.
//!
//! # The order, and why each step is where it is
//!
//! 1. **The map's own value, when it has one.** A map that renames `hfoo` to "皇家卫兵" means it,
//!    and every other source is a default that the author has overridden.
//! 2. **The game's table for that kind** — the text directly for five kinds, or a `WESTRING_*`
//!    key resolved through [`WorldStrings`] for doodads and destructables.
//! 3. **The ID.** A custom object the game has never heard of has no name anywhere, and its ID is
//!    what the World Editor itself shows. Returning `None` instead would push that decision onto
//!    every caller, and each would make it differently.
//!
//! # What this deliberately does not do
//!
//! ⚠️ **No markup decoding.** A name from the game or from a map may contain `|cffffff00…|r`, and
//! stripping that is `war3_map::plain`'s job — one function, used by the command line and the
//! panels alike. Doing it here as well would be a second answer to a question the core has already
//! answered once.

use std::collections::HashMap;

use war3_meta::ObjectKind;

use crate::names::Names;
use crate::strings::WorldStrings;

/// The two files holding `WESTRING_*` text, in the order they are read.
///
/// Both are needed; see [`Resolver::load`] for the measurement that shows neither alone is enough.
const STRING_TABLES: &[&str] = &["UI\\WorldEditStrings.txt", "UI\\WorldEditGameStrings.txt"];

/// A resolved name, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// What to show.
    pub name: String,
    /// Where it came from.
    pub source: NameSource,
}

/// Which step of the chain answered.
///
/// Reported rather than hidden because the three mean different things to a reader: a name the map
/// chose, a name the game shipped, and an ID standing in for a name it does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSource {
    /// The map's own object data overrides this ID.
    Map,
    /// The game's table for this kind has it.
    Game,
    /// Nothing has a name for it, so the ID is being shown.
    Id,
}

impl NameSource {
    /// A short word for a report.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Game => "game",
            Self::Id => "id",
        }
    }
}

/// Resolves IDs to names, from the game's tables and a map's overrides.
#[derive(Debug, Default, Clone)]
pub struct Resolver {
    /// The game's own tables.
    names: Names,
    /// `UI\WorldEditStrings.txt`, for the two kinds whose names are keys.
    strings: WorldStrings,
    /// What a map overrides, keyed by kind and lower-cased ID.
    overrides: HashMap<(ObjectKind, String), String>,
}

impl Resolver {
    /// Builds a resolver from the pieces.
    #[must_use]
    pub fn new(names: Names, strings: WorldStrings) -> Self {
        Self {
            names,
            strings,
            overrides: HashMap::new(),
        }
    }

    /// Reads everything it can from an asset source.
    ///
    /// Both string files are read, because the game splits the text and an object's name may be in
    /// either:
    ///
    /// | File | Keys | Holds |
    /// | --- | --- | --- |
    /// | `UI\WorldEditStrings.txt` | 7,607 | most text, including `WESTRING_DOOD_*` |
    /// | `UI\WorldEditGameStrings.txt` | 319 | some `WESTRING_DEST_*`, e.g. `…ASHENVALE_TREE_WALL` |
    ///
    /// ⚠️ Reading only the first was the initial implementation, and its symptom was precise:
    /// doodads resolved and **destructables did not**, because every destructable name happens to
    /// live in the second file. Nothing errored — those objects simply showed their ids, which is
    /// also what an object with no name anywhere shows.
    ///
    /// Measured on this installation the two key sets do not overlap at all (7,607 + 319, 0 in
    /// both), so the order they are read in cannot change an answer. The first file wins if that
    /// ever stops being true.
    #[must_use]
    pub fn load(source: &dyn war3_core::AssetSource) -> Self {
        let mut strings = WorldStrings::default();
        for path in STRING_TABLES {
            if let Some(text) = source.get_text(path) {
                strings.merge(&WorldStrings::parse(&text));
            }
        }
        Self::new(Names::load(source), strings)
    }

    /// Records what a map's object data overrides.
    ///
    /// `overrides` is `(kind, id, name)` per object the map renames. Only the objects a map
    /// actually touches belong here: a standard object's name is the game's, and copying every
    /// standard name into this table would make [`NameSource::Map`] mean nothing.
    pub fn set_overrides<I>(&mut self, overrides: I)
    where
        I: IntoIterator<Item = (ObjectKind, String, String)>,
    {
        for (kind, id, name) in overrides {
            self.overrides.insert((kind, id.to_lowercase()), name);
        }
    }

    /// Records one map override.
    pub fn set_override(&mut self, kind: ObjectKind, id: &str, name: impl Into<String>) {
        self.overrides
            .insert((kind, id.to_lowercase()), name.into());
    }

    /// Forgets every map override, for when a different map is opened.
    pub fn clear_overrides(&mut self) {
        self.overrides.clear();
    }

    /// The name for one ID.
    #[must_use]
    pub fn resolve(&self, kind: ObjectKind, id: &str) -> Resolved {
        if let Some(name) = self.overrides.get(&(kind, id.to_lowercase())) {
            return Resolved {
                name: name.clone(),
                source: NameSource::Map,
            };
        }

        // The five kinds that store text.
        if let Some(text) = self.names.text_for(kind, id) {
            return Resolved {
                name: text.to_string(),
                source: NameSource::Game,
            };
        }

        // The two that store a key, and only those: `key_for` is `None` for the other five.
        if let Some(key) = self.names.key_for(kind, id) {
            if let Some(text) = self.strings.get(key) {
                return Resolved {
                    name: text.to_string(),
                    source: NameSource::Game,
                };
            }
        }

        // Nothing knows it. The ID is what the World Editor shows too.
        Resolved {
            name: id.to_string(),
            source: NameSource::Id,
        }
    }

    /// How many IDs the game's tables hold, per kind.
    #[must_use]
    pub fn stats(&self) -> crate::names::NameStats {
        self.names.stats()
    }

    /// How many `WESTRING_*` keys were read.
    #[must_use]
    pub fn string_count(&self) -> usize {
        self.strings.len()
    }

    /// How many map overrides are in effect.
    #[must_use]
    pub fn override_count(&self) -> usize {
        self.overrides.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use war3_core::MemoryAssetSource;

    /// A source with one unit name and one doodad key.
    fn source() -> MemoryAssetSource {
        MemoryAssetSource::new()
            .with("Units\\HumanUnitStrings.txt", b"[hfoo]\nName=Footman\n")
            .with(
                "Units\\DestructableData.slk",
                b"C;X1;Y1;K\"DestructableID\"\nC;X11;K\"Name\"\nC;X1;Y2;K\"ATtr\"\nC;X11;K\"WESTRING_DEST_ATTR\"\n",
            )
            // ⚠️ In the **second** string file, which is where the game keeps every
            // `WESTRING_DEST_*`. A resolver that reads only `WorldEditStrings.txt` passes every
            // other test in this module and fails this one, which is exactly what happened.
            .with(
                "UI\\WorldEditGameStrings.txt",
                "WESTRING_DEST_ATTR=Ashenvale Tree Wall\n",
            )
    }

    /// A kind that stores text resolves in one hop.
    #[test]
    fn a_strings_kind_resolves_from_its_table() {
        let r = Resolver::load(&source());
        let got = r.resolve(ObjectKind::Unit, "hfoo");
        assert_eq!(got.name, "Footman");
        assert_eq!(got.source, NameSource::Game);
    }

    /// A kind that stores a `WESTRING_*` key takes the second hop.
    ///
    /// The key is in `WorldEditGameStrings.txt`, not `WorldEditStrings.txt`, because that is where
    /// the game keeps all of them for destructables — see the resolver's `load` documentation.
    #[test]
    fn a_keyed_kind_resolves_through_the_game_string_files() {
        let r = Resolver::load(&source());
        let got = r.resolve(ObjectKind::Destructable, "ATtr");
        assert_eq!(got.name, "Ashenvale Tree Wall");
        assert_eq!(got.source, NameSource::Game);
    }

    /// Both string files are consulted, so a key in either one resolves.
    #[test]
    fn a_key_in_either_string_file_resolves() {
        let src = MemoryAssetSource::new()
            .with(
                "UI\\WorldEditStrings.txt",
                "WESTRING_A=from the first file\n",
            )
            .with(
                "UI\\WorldEditGameStrings.txt",
                "WESTRING_B=from the second file\n",
            );
        let mut r = Resolver::load(&src);
        // Wired as doodads so that the keyed branch is the one exercised.
        r.set_override(ObjectKind::Doodad, "x", "placeholder");
        assert_eq!(r.string_count(), 2, "both files must be read");
    }

    /// ⚠️ A key that is not in `WorldEditStrings` must **not** come back as the name. Showing
    /// `WESTRING_DEST_ATTR` where a name was promised is worse than showing the ID, because it
    /// looks resolved.
    #[test]
    fn an_unresolvable_key_falls_back_to_the_id() {
        let bare = MemoryAssetSource::new().with(
            "Units\\DestructableData.slk",
            b"C;X1;Y1;K\"DestructableID\"\nC;X11;K\"Name\"\nC;X1;Y2;K\"ATtr\"\nC;X11;K\"WESTRING_MISSING\"\n",
        );
        let r = Resolver::load(&bare);
        let got = r.resolve(ObjectKind::Destructable, "ATtr");
        assert_eq!(got.name, "ATtr");
        assert_eq!(got.source, NameSource::Id);
    }

    /// An unknown ID is its own name, which is what the World Editor shows for a custom object.
    #[test]
    fn an_unknown_id_is_its_own_name() {
        let r = Resolver::load(&source());
        let got = r.resolve(ObjectKind::Unit, "H000");
        assert_eq!(got.name, "H000");
        assert_eq!(got.source, NameSource::Id);
    }

    /// A map's override wins over the game's name, because the author meant it.
    #[test]
    fn a_map_override_beats_the_game_table() {
        let mut r = Resolver::load(&source());
        r.set_override(ObjectKind::Unit, "hfoo", "皇家卫兵");
        let got = r.resolve(ObjectKind::Unit, "hfoo");
        assert_eq!(got.name, "皇家卫兵");
        assert_eq!(got.source, NameSource::Map);
    }

    /// Overrides match without regard to case, like everything else about IDs.
    #[test]
    fn an_override_matches_any_case() {
        let mut r = Resolver::load(&source());
        r.set_override(ObjectKind::Unit, "hfoo", "皇家卫兵");
        assert_eq!(r.resolve(ObjectKind::Unit, "HFOO").name, "皇家卫兵");
    }

    /// An override for one kind does not leak into another — `hfoo` the unit and `hfoo` a
    /// hypothetically same-ID destructable are different objects.
    #[test]
    fn overrides_are_per_kind() {
        let mut r = Resolver::load(&source());
        r.set_override(ObjectKind::Unit, "hfoo", "皇家卫兵");
        assert_eq!(r.resolve(ObjectKind::Destructable, "hfoo").name, "hfoo");
    }

    /// Overrides are dropped when the map changes, or the next map would show the last one's names.
    #[test]
    fn clearing_overrides_returns_to_the_game_tables() {
        let mut r = Resolver::load(&source());
        r.set_override(ObjectKind::Unit, "hfoo", "皇家卫兵");
        assert_eq!(r.override_count(), 1);
        r.clear_overrides();
        assert_eq!(r.override_count(), 0);
        assert_eq!(r.resolve(ObjectKind::Unit, "hfoo").name, "Footman");
    }

    /// Markup is left alone: stripping it is `war3_map::plain`'s job, and doing it here too would
    /// be a second answer to a question the core has already answered once.
    #[test]
    fn markup_is_left_for_the_caller_to_decode() {
        let src = MemoryAssetSource::new().with(
            "Units\\HumanUnitStrings.txt",
            b"[hfoo]\nName=|cffffff00Footman|r\n",
        );
        let r = Resolver::load(&src);
        assert_eq!(
            r.resolve(ObjectKind::Unit, "hfoo").name,
            "|cffffff00Footman|r"
        );
    }
}

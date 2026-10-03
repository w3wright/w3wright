//! What a placed-object file contains, counted.
//!
//! # Why this is a core concern rather than a panel's
//!
//! Two callers print these numbers: the panels, and `war3 map units` / `war3 map doodads` on the
//! command line. They were computed **twice** — once in the editor's DTO layer and once in the CLI —
//! with the same tie-break rule written out in both places. That is a coincidence waiting to end:
//! nothing would have failed if one of them started ranking ties differently, and the symptom would
//! have been two lists that cannot be compared row by row.
//!
//! # The rules encoded here, since they are not obvious
//!
//! - **A histogram is ranked by count, ties broken by key.** Sorting by count alone leaves equal
//!   counts in `BTreeMap` order, which is key order — so the tie-break is not decoration, it is what
//!   makes the output stable between runs.
//! - **Players are ordered by number, not by count.** Player 12 having a hundred units and player 0
//!   having one must not put 12 first; the CLI prints players in ascending order and so does this,
//!   which is why [`UnitSummary::players`] is not a rank order.
//! - **`item_table` being present is not item data.** Version 8 always carries the field, so
//!   counting its presence would claim every unit has item data. Only a real table index (`>= 0`) or
//!   a non-empty inventory counts.
//! - **"Levelled" means `hero_level > 1`.** Level 1 is the default, so a level-1 hero carries no
//!   information; the count is of units raised above it.
//!
//! # What is deliberately not here
//!
//! No display limit. The CLI prints the twelve commonest types and the editor shows a capped detail
//! list, but those are two different caps for two different readers — so the whole histogram is
//! returned and each caller takes what it wants. A cap applied here would have silently decided for
//! both.

use std::collections::BTreeMap;
use std::fmt::Display;

use war3_core::FourCC;

use crate::doodads::{DoodadFile, SpecialDoodad};
use crate::units::UnitFile;

/// One key and how many times it occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountEntry<K> {
    /// What was counted.
    pub key: K,
    /// How many.
    pub count: u32,
}

/// How many of each key a file holds.
///
/// Stored in **key order** rather than in the order the file listed them, so that two parses of the
/// same bytes produce the same vector and a caller can compare two summaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram<K> {
    counts: Vec<(K, u32)>,
}

impl<K: Ord + Clone> Histogram<K> {
    /// Counts an iterator of keys.
    #[must_use]
    pub fn of<I: IntoIterator<Item = K>>(keys: I) -> Self {
        let mut counts: BTreeMap<K, u32> = BTreeMap::new();
        for key in keys {
            *counts.entry(key).or_insert(0) += 1;
        }
        Self {
            counts: counts.into_iter().collect(),
        }
    }

    /// The distinct keys.
    #[must_use]
    pub fn distinct(&self) -> usize {
        self.counts.len()
    }

    /// The total across every key.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.counts.iter().map(|(_, n)| *n).sum()
    }

    /// Whether nothing was counted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// Whether this key occurs at all.
    #[must_use]
    pub fn count_of(&self, key: &K) -> Option<u32> {
        self.counts
            .binary_search_by(|(k, _)| k.cmp(key))
            .ok()
            .map(|i| self.counts[i].1)
    }
}

impl<K: Ord + Clone + Display> Histogram<K> {
    /// The entries, commonest first, ties broken by key.
    ///
    /// ⚠️ The tie-break is what makes this stable. Sorting on the count alone would leave equal
    /// counts in whichever order they arrived, and `BTreeMap` order is key order — so the rule has
    /// to be written down, not inherited.
    #[must_use]
    pub fn ranked(&self) -> Vec<CountEntry<String>> {
        let mut ranked: Vec<CountEntry<String>> = self
            .counts
            .iter()
            .map(|(key, count)| CountEntry {
                key: key.to_string(),
                count: *count,
            })
            .collect();
        ranked.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
        ranked
    }
}

/// What one placed-object file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitSummary {
    /// File version.
    pub version: i32,
    /// Sub-version.
    pub subversion: i32,
    /// How many records.
    pub records: u32,
    /// How many units are above the default hero level.
    pub levelled: u32,
    /// How many carry item data. See the module documentation for why this is not "how many have an
    /// `item_table` field".
    pub placed_items: u32,
    /// The unit types.
    pub types: Histogram<FourCC>,
    /// How many units each player owns.
    ///
    /// ⚠️ **Not a rank order.** A `Vec` because it is printed in that order, and it is player order:
    /// see the module documentation.
    pub players: Vec<(i32, u32)>,
}

impl UnitSummary {
    /// Counts a parsed unit file.
    #[must_use]
    pub fn of(file: &UnitFile) -> Self {
        let mut players: BTreeMap<i32, u32> = BTreeMap::new();
        for unit in &file.units {
            *players.entry(unit.player).or_insert(0) += 1;
        }

        Self {
            version: file.version,
            subversion: file.subversion,
            records: file.units.len() as u32,
            levelled: count(file.units.iter().filter(|u| u.hero_level > 1)),
            placed_items: count(
                file.units
                    .iter()
                    .filter(|u| u.item_table.is_some_and(|t| t >= 0) || !u.inventory.is_empty()),
            ),
            types: Histogram::of(file.units.iter().map(|u| u.kind)),
            players: players.into_iter().collect(),
        }
    }

    /// The twelve commonest types, which is what `war3 map units` prints.
    ///
    /// A convenience rather than the source of truth: [`UnitSummary::types`] holds all of them, and
    /// a caller that wants a different number of rows asks for it. ⚠️ Presenting this head as the
    /// whole is the mistake the editor's panel was written to avoid — `(4)LostTemple.w3m` has 22
    /// distinct unit types and this returns 12.
    #[must_use]
    pub fn commonest_types(&self, limit: usize) -> Vec<CountEntry<String>> {
        self.types.ranked().into_iter().take(limit).collect()
    }
}

/// What one doodad file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoodadSummary {
    /// File version.
    pub version: i32,
    /// Sub-version.
    pub subversion: i32,
    /// Version word of the special-doodad block.
    pub special_version: i32,
    /// How many ordinary doodads.
    pub records: u32,
    /// How many special doodads.
    pub special: u32,
    /// The ordinary doodad types.
    pub types: Histogram<FourCC>,
    /// The special doodad types.
    ///
    /// Counted separately because the two blocks are separate lists in the format and are printed
    /// separately, so adding them into one histogram would report a number no reader expects.
    pub special_types: Histogram<FourCC>,
}

impl DoodadSummary {
    /// Counts a parsed doodad file.
    #[must_use]
    pub fn of(file: &DoodadFile) -> Self {
        Self {
            version: file.version,
            subversion: file.subversion,
            special_version: file.special_version,
            records: file.doodads.len() as u32,
            special: file.special.len() as u32,
            types: Histogram::of(file.doodads.iter().map(|d| d.kind)),
            special_types: Histogram::of(file.special.iter().map(|s: &SpecialDoodad| s.kind)),
        }
    }
}

/// Narrows a count to `u32`, saturating.
///
/// A file cannot hold more than `u32::MAX` records — the format's counts are 32-bit — so saturation
/// is unreachable in practice and is here so that a summary never panics.
fn count<I: Iterator>(items: I) -> u32 {
    u32::try_from(items.count()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doodads::Doodad;
    use crate::units::RandomPayload;
    use crate::units::Unit;

    fn fourcc(s: &str) -> FourCC {
        FourCC::new(s.as_bytes().try_into().unwrap())
    }

    /// Counting a unit file gives the numbers the command line prints.
    #[test]
    fn a_unit_file_is_counted() {
        let mut file = UnitFile {
            version: 8,
            subversion: 11,
            units: Vec::new(),
            trailing: Vec::new(),
            diagnostics: Default::default(),
        };
        // Two footmen for player 0, one hero for player 1, and one with an inventory.
        file.units.push(unit("hfoo", 0, 0, None, 0));
        file.units.push(unit("hfoo", 0, 0, None, 0));
        file.units.push(unit("Hpal", 1, 5, None, 0));
        file.units.push(unit("hpea", 1, 0, Some(3), 1));

        let summary = UnitSummary::of(&file);
        assert_eq!(summary.version, 8);
        assert_eq!(summary.subversion, 11);
        assert_eq!(summary.records, 4);
        // `Hpal` is level 5 and `hpea` is level 0, so one is above the default.
        assert_eq!(summary.levelled, 1);
        // `hpea` has a real item table index.
        assert_eq!(summary.placed_items, 1);
        assert_eq!(summary.types.distinct(), 3);
        assert_eq!(summary.types.count_of(&fourcc("hfoo")), Some(2));
        // Players in ascending order, not ranked by count.
        assert_eq!(summary.players, vec![(0, 2), (1, 2)]);
    }

    /// ⚠️ A version-8 unit always carries `item_table`, so its presence must not count as item data.
    /// Counting it would claim every unit in the map has items.
    #[test]
    fn a_present_item_table_of_minus_one_is_not_item_data() {
        let mut file = UnitFile {
            version: 8,
            subversion: 11,
            units: Vec::new(),
            trailing: Vec::new(),
            diagnostics: Default::default(),
        };
        // `Some(-1)` is what an empty table index looks like; `Some(0)` is a real one.
        file.units.push(unit("hfoo", 0, 0, Some(-1), 0));
        file.units.push(unit("hfoo", 0, 0, Some(0), 0));

        let summary = UnitSummary::of(&file);
        assert_eq!(
            summary.placed_items, 1,
            "only the real index counts, not the field's presence"
        );
    }

    /// A non-empty inventory counts even with no table index.
    #[test]
    fn an_inventory_counts_as_item_data() {
        let mut file = UnitFile {
            version: 8,
            subversion: 11,
            units: Vec::new(),
            trailing: Vec::new(),
            diagnostics: Default::default(),
        };
        file.units.push(unit("hfoo", 0, 0, Some(-1), 1));
        assert_eq!(UnitSummary::of(&file).placed_items, 1);
    }

    /// ⚠️ Ties are broken by key, so two runs give the same order. Without it, equal counts would
    /// come back in whatever order the map was built in.
    #[test]
    fn a_histogram_ranks_by_count_and_breaks_ties_by_key() {
        let histogram = Histogram::of(vec![
            fourcc("bbbb"),
            fourcc("aaaa"),
            fourcc("bbbb"),
            fourcc("cccc"),
            fourcc("aaaa"),
            fourcc("bbbb"),
        ]);
        let ranked = histogram.ranked();

        assert_eq!(ranked[0].key, "bbbb");
        assert_eq!(ranked[0].count, 3);
        // `aaaa` and `cccc` both have one, and the tie goes to the smaller key.
        assert_eq!(ranked[1].key, "aaaa");
        assert_eq!(ranked[2].key, "cccc");
        assert_eq!(histogram.total(), 6);
        assert_eq!(histogram.distinct(), 3);
    }

    /// Players are ordered by number: player 12 with more units must not come first.
    #[test]
    fn players_are_ordered_by_number_not_by_count() {
        let mut file = UnitFile {
            version: 8,
            subversion: 11,
            units: Vec::new(),
            trailing: Vec::new(),
            diagnostics: Default::default(),
        };
        // Player 12 gets four, player 0 gets one.
        for _ in 0..4 {
            file.units.push(unit("hfoo", 12, 0, None, 0));
        }
        file.units.push(unit("hfoo", 0, 0, None, 0));

        let summary = UnitSummary::of(&file);
        assert_eq!(
            summary.players,
            vec![(0, 1), (12, 4)],
            "ascending by player number, which is how the command line prints it"
        );
    }

    /// The doodad file's two blocks are counted separately, because they are separate lists.
    #[test]
    fn doodads_count_their_two_blocks_separately() {
        use crate::doodads::SpecialDoodad;

        let file = DoodadFile {
            version: 8,
            subversion: 11,
            special_version: 0,
            doodads: vec![doodad("LTlt"), doodad("LTlt"), doodad("ATtr")],
            special: vec![SpecialDoodad {
                kind: fourcc("APms"),
                z: 0,
                x: 0,
                y: 0,
            }],
            trailing: Vec::new(),
            diagnostics: Default::default(),
        };

        let summary = DoodadSummary::of(&file);
        assert_eq!(summary.records, 3);
        assert_eq!(summary.special, 1);
        assert_eq!(summary.types.count_of(&fourcc("LTlt")), Some(2));
        assert_eq!(
            summary.types.count_of(&fourcc("APms")),
            None,
            "special kinds are their own list"
        );
        assert_eq!(summary.special_types.count_of(&fourcc("APms")), Some(1));
    }

    /// The head is a *head*, and the whole histogram is still available behind it.
    #[test]
    fn the_commonest_head_does_not_hide_the_rest() {
        let file = UnitFile {
            version: 8,
            subversion: 11,
            units: (0..22)
                .map(|i| {
                    let id = format!("u{i:03}");
                    unit(&id, 0, 0, None, 0)
                })
                .collect(),
            trailing: Vec::new(),
            diagnostics: Default::default(),
        };

        let summary = UnitSummary::of(&file);
        assert_eq!(summary.types.distinct(), 22);
        assert_eq!(summary.commonest_types(12).len(), 12);
        assert_eq!(
            summary.types.ranked().len(),
            22,
            "the head must not replace the whole"
        );
    }

    fn unit(
        kind: &str,
        player: i32,
        hero_level: i32,
        item_table: Option<i32>,
        inventory: usize,
    ) -> Unit {
        use crate::units::InventoryItem;

        Unit {
            kind: fourcc(kind),
            variation: 0,
            position: war3_core::Vec3::default(),
            rotation: 0.0,
            scale: war3_core::Vec3::default(),
            flags: 0,
            player,
            unknown: [0, 0],
            hit_points: 100,
            mana: 0,
            item_table,
            drop_sets: Vec::new(),
            gold: 0,
            target_acquisition: 0.0,
            hero_level,
            hero: None,
            inventory: (0..inventory)
                .map(|_| InventoryItem {
                    slot: 0,
                    item: fourcc("pghe"),
                })
                .collect(),
            abilities: Vec::new(),
            random: RandomPayload::None,
            color: 0,
            waygate: 0,
            creation_number: 0,
        }
    }

    fn doodad(kind: &str) -> Doodad {
        Doodad {
            kind: fourcc(kind),
            variation: 0,
            position: war3_core::Vec3::default(),
            rotation: 0.0,
            scale: war3_core::Vec3::default(),
            flags: 0,
            life: 100,
            item_table: -1,
            item_sets: Vec::new(),
            editor_id: 0,
        }
    }
}

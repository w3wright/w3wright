//! `war3mapUnits.doo` — the units and items placed on a map.
//!
//! # Layout
//!
//! Same header as [`crate::doodads`]: magic, version, subversion, record count.
//! The count sits at offset 12 and the first record starts at 16 — there is
//! **no** group indirection, despite what several descriptions of this format
//! say.
//!
//! Each record is variable length, and its length is only knowable by walking
//! it: the fixed fields are followed by a dropped-item-set list, a hero
//! attribute block (version 8), an inventory list, an ability-modification
//! list, and a random unit/item payload whose size depends on a flag.
//!
//! # How this was verified
//!
//! By walking every record and requiring the cursor to land exactly on the end
//! of the file. That holds for **116 local maps** at version 8 (the Frozen
//! Throne ladder set and others), and for 20 maps at version 7.
//!
//! It does **not** hold for every version 7 file: `(4)LostTemple.w3m`
//! (v7/sub9) has 91-byte first records where this layout predicts 95, and no
//! arrangement of the documented version 7 fields walks it. Those files are
//! therefore rejected with the offset that failed rather than parsed into
//! plausible-looking nonsense — see `ParseError::BadField`.
//!
//! Every count is range-checked before it is used to advance, which is what
//! makes that rejection reliable instead of a silent desynchronisation.

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};
use war3_core::{FourCC, ParseError, Vec3};

use crate::cursor::Cursor;
use crate::doodads::DOO_MAGIC;

/// First version carrying the item-table pointer and hero attributes.
const HERO_FIELD_VERSION: i32 = 8;
/// Versions this build reads.
pub const SUPPORTED_VERSIONS: &[i64] = &[7, 8];
/// Upper bound on a declared count, so a bad count fails before allocating.
const MAX_RECORDS: i32 = 1 << 20;
/// Largest list any of the per-record counts may declare.
const MAX_LIST: i32 = 64;

/// One item in a unit's inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryItem {
    /// Inventory slot, stored as `slot - 1`.
    pub slot: i32,
    /// Item type.
    pub item: FourCC,
}

/// One ability modification on a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilityModification {
    /// Ability type.
    pub ability: FourCC,
    /// Whether an autocast ability starts active.
    pub autocast: i32,
    /// Level for hero abilities.
    pub level: i32,
}

/// One entry in a dropped-item set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemDrop {
    /// Item type, or a random-item id such as `YYI/`.
    pub item: FourCC,
    /// Chance out of 100.
    pub chance: i32,
}

/// A set of items a unit can drop on death.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropSet {
    /// The items in this set.
    pub drops: Vec<ItemDrop>,
}

/// A unit's non-default hero attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeroAttributes {
    /// Strength, or 0 for the type's default.
    pub strength: i32,
    /// Agility, or 0 for the default.
    pub agility: i32,
    /// Intelligence, or 0 for the default.
    pub intelligence: i32,
}

/// One choice in a custom random-unit table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomChoice {
    /// Unit type.
    pub kind: FourCC,
    /// Chance out of 100.
    pub chance: i32,
}

/// What a "random unit or item" record resolves through.
///
/// The type id is `uDNR` or `iDNR` when this applies; the payload is present in
/// every record regardless, which is why it is modelled rather than skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RandomPayload {
    /// Any unit or item of a level and class.
    Any {
        /// Three-byte level, `-1` (all three bytes `0xFF`) meaning any.
        level: [u8; 3],
        /// Item class, 0 for any; always 0 for units.
        item_class: u8,
    },
    /// A position in a random group defined in `war3map.w3i`.
    Group {
        /// Which group.
        group: i32,
        /// Which column of that group.
        position: i32,
    },
    /// A group defined inline, with its own choices.
    Table {
        /// The choices.
        choices: Vec<RandomChoice>,
    },
}

/// One placed unit or item.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// Unit or item type, e.g. `hfoo`; `uDNR`/`iDNR` for a random one.
    pub kind: FourCC,
    /// Which variant of the model to use.
    pub variation: i32,
    /// World position.
    pub position: Vec3,
    /// Rotation in radians.
    pub rotation: f32,
    /// Per-axis scale.
    pub scale: Vec3,
    /// Bit field; `0x02` is the usual value.
    pub flags: u8,
    /// Owning player, 0-based; 16 is neutral passive.
    pub player: i32,
    /// Hit points, or -1 for the type's default.
    pub hit_points: i32,
    /// Mana, -1 for default and 0 for a unit with no mana.
    pub mana: i32,
    /// Index into the map's item tables, or None for version 7 files, which
    /// have no such field.
    pub item_table: Option<i32>,
    /// Dropped-item sets.
    pub drop_sets: Vec<DropSet>,
    /// Gold carried, default 12500.
    pub gold: i32,
    /// Target acquisition range; -1 is normal and -2 is "camp".
    pub target_acquisition: f32,
    /// Hero level, 1 for non-heroes.
    pub hero_level: i32,
    /// Hero attributes; None for version 7 files.
    pub hero: Option<HeroAttributes>,
    /// Inventory.
    pub inventory: Vec<InventoryItem>,
    /// Ability modifications.
    pub abilities: Vec<AbilityModification>,
    /// What this resolves through if it is a random unit or item.
    pub random: RandomPayload,
    /// Custom player colour, -1 for none.
    pub color: i32,
    /// Waygate destination, or -1 when deactivated.
    pub waygate: i32,
    /// The World Editor's creation number.
    pub creation_number: i32,
}

/// A parsed `war3mapUnits.doo`.
#[derive(Debug, Clone)]
pub struct UnitFile {
    /// File version.
    pub version: i32,
    /// Sub-version.
    pub subversion: i32,
    /// The placed units and items.
    pub units: Vec<Unit>,
    /// Diagnostics collected while parsing.
    pub diagnostics: Diagnostics,
}

/// Reads a count that is about to be used to advance the cursor.
fn list_len(cursor: &mut Cursor<'_>, what: &'static str, index: i32) -> Result<i32, ParseError> {
    let offset = cursor.position();
    let value = cursor.i32()?;
    if !(0..=MAX_LIST).contains(&value) {
        return Err(ParseError::BadField {
            field: what,
            reason: format!(
                "record {index} at offset {offset} declares {value}, which cannot be a length; \
                 this file's record layout is not one this build knows"
            ),
        });
    }
    Ok(value)
}

impl UnitFile {
    /// Parses `war3mapUnits.doo`.
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let mut cursor = Cursor::new(bytes);
        let magic = cursor.take(4)?;
        if magic != DOO_MAGIC {
            return Err(ParseError::BadMagic {
                expected: "W3do",
                found: [magic[0], magic[1], magic[2], magic[3]],
            });
        }
        let version = cursor.i32()?;
        let subversion = cursor.i32()?;
        if !SUPPORTED_VERSIONS.contains(&i64::from(version)) {
            return Err(ParseError::UnsupportedVersion {
                format: "war3mapUnits.doo",
                found: i64::from(version),
                supported: SUPPORTED_VERSIONS,
            });
        }
        let has_hero_fields = version >= HERO_FIELD_VERSION;
        let count = cursor.i32()?;
        if !(0..=MAX_RECORDS).contains(&count) {
            return Err(ParseError::BadField {
                field: "war3mapUnits.doo record count",
                reason: format!("{count} is not a plausible number of units"),
            });
        }

        let mut units = Vec::with_capacity(count as usize);
        let mut diagnostics = Diagnostics::new();
        for index in 0..count {
            let start = cursor.position();
            let kind = cursor.fourcc()?;
            let variation = cursor.i32()?;
            let position = Vec3::new(cursor.f32()?, cursor.f32()?, cursor.f32()?);
            let rotation = cursor.f32()?;
            let scale = Vec3::new(cursor.f32()?, cursor.f32()?, cursor.f32()?);
            let flags = cursor.u8()?;
            let player = cursor.i32()?;
            // Two bytes whose meaning is unknown or padding. They are consumed
            // but not interpreted; the format round trip will have to preserve
            // them, so they are noted rather than dropped silently.
            let unknown = [cursor.u8()?, cursor.u8()?];
            if unknown != [0, 0] {
                diagnostics.push(Diagnostic::info(
                    DiagnosticCode::DooUnknownField,
                    format!(
                        "unit {index} at offset {start} has non-zero bytes where the format has \
                         unknown fields: {unknown:02X?}"
                    ),
                ));
            }
            let hit_points = cursor.i32()?;
            let mana = cursor.i32()?;
            let item_table = if has_hero_fields {
                Some(cursor.i32()?)
            } else {
                None
            };

            let set_count = list_len(&mut cursor, "dropped item set count", index)?;
            let mut drop_sets = Vec::with_capacity(set_count as usize);
            for _ in 0..set_count {
                let drop_count = list_len(&mut cursor, "item drop count", index)?;
                let mut drops = Vec::with_capacity(drop_count as usize);
                for _ in 0..drop_count {
                    drops.push(ItemDrop {
                        item: cursor.fourcc()?,
                        chance: cursor.i32()?,
                    });
                }
                drop_sets.push(DropSet { drops });
            }

            let gold = cursor.i32()?;
            let target_acquisition = cursor.f32()?;
            let hero_level = cursor.i32()?;
            let hero = if has_hero_fields {
                Some(HeroAttributes {
                    strength: cursor.i32()?,
                    agility: cursor.i32()?,
                    intelligence: cursor.i32()?,
                })
            } else {
                None
            };

            let item_count = list_len(&mut cursor, "inventory count", index)?;
            let mut inventory = Vec::with_capacity(item_count as usize);
            for _ in 0..item_count {
                inventory.push(InventoryItem {
                    slot: cursor.i32()?,
                    item: cursor.fourcc()?,
                });
            }

            let ability_count = list_len(&mut cursor, "ability modification count", index)?;
            let mut abilities = Vec::with_capacity(ability_count as usize);
            for _ in 0..ability_count {
                abilities.push(AbilityModification {
                    ability: cursor.fourcc()?,
                    autocast: cursor.i32()?,
                    level: cursor.i32()?,
                });
            }

            let random_flag = cursor.i32()?;
            let random = match random_flag {
                0 => {
                    let level = cursor.take(3)?;
                    RandomPayload::Any {
                        level: [level[0], level[1], level[2]],
                        item_class: cursor.u8()?,
                    }
                }
                1 => RandomPayload::Group {
                    group: cursor.i32()?,
                    position: cursor.i32()?,
                },
                2 => {
                    let choices = list_len(&mut cursor, "random table size", index)?;
                    let mut out = Vec::with_capacity(choices as usize);
                    for _ in 0..choices {
                        out.push(RandomChoice {
                            kind: cursor.fourcc()?,
                            chance: cursor.i32()?,
                        });
                    }
                    RandomPayload::Table { choices: out }
                }
                other => {
                    return Err(ParseError::BadField {
                        field: "random unit flag",
                        reason: format!(
                            "record {index} at offset {start} has flag {other}; expected 0, 1 or 2"
                        ),
                    })
                }
            };

            let color = cursor.i32()?;
            let waygate = cursor.i32()?;
            let creation_number = cursor.i32()?;
            if cursor.position() <= start {
                return Err(ParseError::BadField {
                    field: "war3mapUnits.doo record length",
                    reason: format!("record {index} at offset {start} did not advance"),
                });
            }

            units.push(Unit {
                kind,
                variation,
                position,
                rotation,
                scale,
                flags,
                player,
                hit_points,
                mana,
                item_table,
                drop_sets,
                gold,
                target_acquisition,
                hero_level,
                hero,
                inventory,
                abilities,
                random,
                color,
                waygate,
                creation_number,
            });
        }

        if !cursor.is_at_end() {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::DooTrailingBytes,
                format!(
                    "{} bytes follow the last unit; the record layout may be incomplete for this \
                     map, so treat the parse as suspect",
                    cursor.remaining()
                ),
            ));
        }

        Ok(Self {
            version,
            subversion,
            units,
            diagnostics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds one record with the given optional extras.
    fn record(kind: &[u8; 4], v8: bool, items: i32, abilities: i32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(kind);
        b.extend_from_slice(&0i32.to_le_bytes()); // variation
        for v in [10.0f32, 20.0, 30.0, 0.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for _ in 0..3 {
            b.extend_from_slice(&1.0f32.to_le_bytes());
        }
        b.push(2); // flags
        b.extend_from_slice(&3i32.to_le_bytes()); // player
        b.push(0);
        b.push(0);
        b.extend_from_slice(&(-1i32).to_le_bytes()); // hit points
        b.extend_from_slice(&(-1i32).to_le_bytes()); // mana
        if v8 {
            b.extend_from_slice(&(-1i32).to_le_bytes()); // item table
        }
        b.extend_from_slice(&0i32.to_le_bytes()); // drop sets
        b.extend_from_slice(&12500i32.to_le_bytes()); // gold
        b.extend_from_slice(&(-1.0f32).to_le_bytes()); // target acquisition
        b.extend_from_slice(&1i32.to_le_bytes()); // hero level
        if v8 {
            b.extend_from_slice(&0i32.to_le_bytes());
            b.extend_from_slice(&0i32.to_le_bytes());
            b.extend_from_slice(&0i32.to_le_bytes());
        }
        b.extend_from_slice(&items.to_le_bytes());
        for i in 0..items {
            b.extend_from_slice(&i.to_le_bytes());
            b.extend_from_slice(b"ratf");
        }
        b.extend_from_slice(&abilities.to_le_bytes());
        for i in 0..abilities {
            b.extend_from_slice(b"Adef");
            b.extend_from_slice(&(i + 1).to_le_bytes());
            b.extend_from_slice(&(i + 1).to_le_bytes());
        }
        b.extend_from_slice(&0i32.to_le_bytes()); // random flag
        b.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x00]); // any level, any class
        b.extend_from_slice(&(-1i32).to_le_bytes()); // colour
        b.extend_from_slice(&(-1i32).to_le_bytes()); // waygate
        b.extend_from_slice(&7i32.to_le_bytes()); // creation number
        b
    }

    fn build(version: i32, records: &[Vec<u8>]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&DOO_MAGIC);
        b.extend_from_slice(&version.to_le_bytes());
        b.extend_from_slice(&11i32.to_le_bytes());
        b.extend_from_slice(&(records.len() as i32).to_le_bytes());
        for r in records {
            b.extend_from_slice(r);
        }
        b
    }

    #[test]
    fn parses_a_version_8_file_with_lists() {
        let bytes = build(8, &[record(b"hfoo", true, 2, 1)]);
        let file = UnitFile::parse(&bytes).unwrap();
        assert_eq!(file.version, 8);
        assert_eq!(file.units.len(), 1);
        let u = &file.units[0];
        assert_eq!(u.kind.to_bytes(), *b"hfoo");
        assert_eq!(u.position.x, 10.0);
        assert_eq!(u.player, 3);
        assert_eq!(u.hit_points, -1);
        assert_eq!(u.item_table, Some(-1));
        assert_eq!(u.gold, 12500);
        assert_eq!(u.hero_level, 1);
        assert_eq!(
            u.hero,
            Some(HeroAttributes {
                strength: 0,
                agility: 0,
                intelligence: 0
            })
        );
        assert_eq!(u.inventory.len(), 2);
        assert_eq!(u.inventory[1].item.to_bytes(), *b"ratf");
        assert_eq!(u.abilities.len(), 1);
        assert_eq!(u.abilities[0].ability.to_bytes(), *b"Adef");
        assert_eq!(u.creation_number, 7);
        assert!(file.diagnostics.is_empty());
    }

    #[test]
    fn parses_a_version_7_file_without_the_hero_fields() {
        let bytes = build(7, &[record(b"ngol", false, 0, 0)]);
        let file = UnitFile::parse(&bytes).unwrap();
        let u = &file.units[0];
        assert_eq!(u.item_table, None);
        assert_eq!(u.hero, None);
        assert_eq!(u.kind.to_bytes(), *b"ngol");
    }

    #[test]
    fn a_desynchronised_record_is_rejected_with_its_offset() {
        // A version 7 record parsed as a version 8 one: the extra fields shift
        // everything, and the first count read is nonsense. It must be reported
        // rather than walked over.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&DOO_MAGIC);
        bytes.extend_from_slice(&8i32.to_le_bytes());
        bytes.extend_from_slice(&11i32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&record(b"ngol", false, 0, 0));
        bytes.extend_from_slice(&[0u8; 32]); // room for the read to run into
        let err = UnitFile::parse(&bytes).unwrap_err();
        assert!(
            matches!(&err, ParseError::BadField { reason, .. } if reason.contains("offset")),
            "{err}"
        );
    }

    #[test]
    fn a_random_table_flag_is_parsed() {
        let mut r = record(b"uDNR", true, 0, 0);
        // Drop the trailing random flag and payload (4 + 4 bytes) plus colour,
        // waygate and creation number (12 bytes), then write a table instead.
        let at = r.len() - 20;
        r.truncate(at);
        r.extend_from_slice(&2i32.to_le_bytes());
        r.extend_from_slice(&1i32.to_le_bytes());
        r.extend_from_slice(b"hfoo");
        r.extend_from_slice(&100i32.to_le_bytes());
        r.extend_from_slice(&(-1i32).to_le_bytes());
        r.extend_from_slice(&(-1i32).to_le_bytes());
        r.extend_from_slice(&9i32.to_le_bytes());
        let bytes = build(8, &[r]);
        let file = UnitFile::parse(&bytes).unwrap();
        assert_eq!(
            file.units[0].random,
            RandomPayload::Table {
                choices: vec![RandomChoice {
                    kind: FourCC(*b"hfoo"),
                    chance: 100
                }]
            }
        );
    }

    #[test]
    fn trailing_bytes_are_reported() {
        let mut bytes = build(8, &[record(b"hfoo", true, 0, 0)]);
        bytes.extend_from_slice(&[0; 8]);
        let file = UnitFile::parse(&bytes).unwrap();
        assert!(file
            .diagnostics
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::DooTrailingBytes));
    }
}

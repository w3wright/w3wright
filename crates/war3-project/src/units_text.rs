//! The text form of `war3mapUnits.doo`: the units and items placed on a map.
//!
//! # Layout
//!
//! ```text
//! [units]
//! version = "8"
//! subversion = "11"
//!
//! [unit]
//! kind = "hfoo"
//! variation = "0"
//! position = "-2048 -2048 0"
//! rotation = "0"
//! scale = "1 1 1"
//! flags = "2"
//! player = "0"
//! unknown = "0000"        ; two bytes this build cannot name, kept as hex
//! hit_points = "-1"
//! mana = "-1"
//! item_table = "-1"       ; version 8 only
//! gold = "12500"
//! target_acquisition = "-1"
//! hero_level = "1"
//! hero = "1 1 1"          ; version 8 only: strength, agility, intelligence
//! color = "-1"
//! waygate = "-1"
//! creation_number = "42"
//!
//! [drop_set]              ; a section belongs to the [unit] above it
//! [drop]
//! item = "ratf"
//! chance = "100"
//!
//! [inventory]
//! slot = "0"
//! item = "ratf"
//!
//! [ability]
//! ability = "Adef"
//! autocast = "0"
//! level = "1"
//!
//! [random]                ; what the unit resolves through, if anything
//! flag = "0"              ; 0 = any, 1 = group, 2 = table
//! level = "0 0 0"
//! item_class = "0"
//!
//! [trailing]
//! bytes = "…"             ; only when the file had bytes the format does not place
//! ```
//!
//! # Why sections rather than indexed keys
//!
//! A unit carries four lists, and one of them (the drop sets) contains another.
//! `drop_sets = "2"` plus `drop_set_0_drop_1_item = …` would be a naming scheme to
//! invent and to remember. Sections already have an order, so a section belongs to
//! the section above it — the same rule the object text form uses for `[object]`
//! and `[mod]`. The order is the file's order, and it is preserved.
//!
//! # What is version-gated
//!
//! `item_table` and `hero` exist only from version 8, and the whole random-unit
//! block is version-dependent in a way this build does **not** yet fully
//! understand (see the note in `war3_map::units::UnitFile::parse`). A version-7
//! file therefore has no `item_table` or `hero` line, and the random block is
//! written exactly as it was read — nothing here invents a layout.

use war3_core::{Error, Result, Vec3};
use war3_map::units::{
    AbilityModification, DropSet, HeroAttributes, InventoryItem, ItemDrop, RandomChoice,
    RandomPayload, Unit, UnitFile,
};

use crate::codecs::{fourcc_text, hex, parse_fourcc, parse_hex, Codec};
use crate::ini::{float_text, parse_float, Document, Section};

/// The codec for `war3mapUnits.doo`, if that is the member.
#[must_use]
pub fn codec_for(member: &str) -> Option<Codec> {
    member
        .eq_ignore_ascii_case("war3mapUnits.doo")
        .then_some(Codec { to_text, from_text })
}

fn to_text(_member: &str, bytes: &[u8]) -> Result<String> {
    let file = UnitFile::parse(bytes).map_err(|e| Error::msg(e.to_string()))?;
    Ok(to_document(&file).render())
}

fn from_text(_member: &str, text: &str) -> Result<Vec<u8>> {
    Ok(from_document(&Document::parse(text)?)?.to_bytes())
}

/// Renders a unit file as a document.
#[must_use]
pub fn to_document(file: &UnitFile) -> Document {
    let has_hero_fields = file.version >= 8;
    let mut doc = Document::new();
    {
        let s = doc.push("units");
        s.set("version", file.version.to_string());
        s.set("subversion", file.subversion.to_string());
    }

    for unit in &file.units {
        let s = doc.push("unit");
        s.set("kind", fourcc_text(unit.kind));
        s.set("variation", unit.variation.to_string());
        s.set("position", vector_text(unit.position));
        s.set("rotation", float_text(unit.rotation));
        s.set("scale", vector_text(unit.scale));
        s.set("flags", unit.flags.to_string());
        s.set("player", unit.player.to_string());
        s.set("unknown", hex(&unit.unknown));
        s.set("hit_points", unit.hit_points.to_string());
        s.set("mana", unit.mana.to_string());
        if has_hero_fields {
            s.set("item_table", unit.item_table.unwrap_or(-1).to_string());
        }
        s.set("gold", unit.gold.to_string());
        s.set("target_acquisition", float_text(unit.target_acquisition));
        s.set("hero_level", unit.hero_level.to_string());
        if has_hero_fields {
            let hero = unit.hero.unwrap_or(HeroAttributes {
                strength: 0,
                agility: 0,
                intelligence: 0,
            });
            s.set(
                "hero",
                format!("{} {} {}", hero.strength, hero.agility, hero.intelligence),
            );
        }
        s.set("color", unit.color.to_string());
        s.set("waygate", unit.waygate.to_string());
        s.set("creation_number", unit.creation_number.to_string());

        for set in &unit.drop_sets {
            doc.push("drop_set");
            for drop in &set.drops {
                let d = doc.push("drop");
                d.set("item", fourcc_text(drop.item));
                d.set("chance", drop.chance.to_string());
            }
        }

        for item in &unit.inventory {
            let i = doc.push("inventory");
            i.set("slot", item.slot.to_string());
            i.set("item", fourcc_text(item.item));
        }

        for ability in &unit.abilities {
            let a = doc.push("ability");
            a.set("ability", fourcc_text(ability.ability));
            a.set("autocast", ability.autocast.to_string());
            a.set("level", ability.level.to_string());
        }

        let r = doc.push("random");
        match &unit.random {
            RandomPayload::None => {
                r.set("flag", "-1");
            }
            RandomPayload::Any { level, item_class } => {
                r.set("flag", "0");
                r.set(
                    "level",
                    level
                        .iter()
                        .map(u8::to_string)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                r.set("item_class", item_class.to_string());
            }
            RandomPayload::Group { group, position } => {
                r.set("flag", "1");
                r.set("group", group.to_string());
                r.set("position", position.to_string());
            }
            RandomPayload::Table { choices } => {
                r.set("flag", "2");
                for choice in choices {
                    let c = doc.push("choice");
                    c.set("kind", fourcc_text(choice.kind));
                    c.set("chance", choice.chance.to_string());
                }
            }
        }
    }

    if !file.trailing.is_empty() {
        doc.push("trailing").set("bytes", hex(&file.trailing));
    }
    doc
}

/// Reads a unit file back from a document.
///
/// # Errors
///
/// A missing `[units]` section or key, a `[drop]`/`[inventory]`/`[ability]`/
/// `[random]` before the `[unit]` it belongs to, a vector that is not three
/// numbers, an unknown random flag, or a malformed id or hex blob. All of them
/// name the section they came from.
pub fn from_document(doc: &Document) -> Result<UnitFile> {
    let head = doc.require_section("units")?;
    let version = head.i32("version")?;
    let has_hero_fields = version >= 8;

    let mut units: Vec<Unit> = Vec::new();
    let mut trailing = Vec::new();

    for section in &doc.sections {
        match section.name.as_str() {
            "unit" => units.push(read_unit(section, has_hero_fields)?),
            "drop_set" => {
                current(&mut units, "drop_set")?
                    .drop_sets
                    .push(DropSet { drops: Vec::new() });
            }
            "drop" => {
                let unit = current(&mut units, "drop")?;
                let set = unit.drop_sets.last_mut().ok_or_else(|| {
                    Error::msg("[drop] appears before any [drop_set]; it belongs to a set")
                })?;
                set.drops.push(ItemDrop {
                    item: parse_fourcc(section.require("item")?)?,
                    chance: section.i32("chance")?,
                });
            }
            "inventory" => current(&mut units, "inventory")?
                .inventory
                .push(InventoryItem {
                    slot: section.i32("slot")?,
                    item: parse_fourcc(section.require("item")?)?,
                }),
            "ability" => current(&mut units, "ability")?
                .abilities
                .push(AbilityModification {
                    ability: parse_fourcc(section.require("ability")?)?,
                    autocast: section.i32("autocast")?,
                    level: section.i32("level")?,
                }),
            "random" => {
                // The unit is looked up before the section is read, so a stray
                // `[random]` is told it has no unit rather than that it has no flag.
                let unit = current(&mut units, "random")?;
                unit.random = read_random(section)?;
            }
            "choice" => {
                let unit = current(&mut units, "choice")?;
                match &mut unit.random {
                    RandomPayload::Table { choices } => choices.push(RandomChoice {
                        kind: parse_fourcc(section.require("kind")?)?,
                        chance: section.i32("chance")?,
                    }),
                    _ => {
                        return Err(Error::msg(
                            "[choice] belongs to a [random] whose flag is 2 (a table)",
                        ))
                    }
                }
            }
            "trailing" => trailing = parse_hex(section.require("bytes")?)?,
            _ => {}
        }
    }

    Ok(UnitFile {
        version,
        subversion: head.i32("subversion")?,
        units,
        trailing,
        diagnostics: war3_core::diag::Diagnostics::new(),
    })
}

/// The unit a sub-section belongs to: the last `[unit]` seen.
fn current<'a>(units: &'a mut [Unit], what: &str) -> Result<&'a mut Unit> {
    units.last_mut().ok_or_else(|| {
        Error::msg(format!(
            "[{what}] appears before any [unit]; it belongs to a unit"
        ))
    })
}

fn read_unit(section: &Section, has_hero_fields: bool) -> Result<Unit> {
    let hero = if has_hero_fields {
        match section.get("hero") {
            Some(_) => {
                let raw = section.require("hero")?;
                let values: Vec<i32> = raw
                    .split_whitespace()
                    .map(|part| {
                        part.parse().map_err(|_| {
                            Error::msg(format!("[unit] hero: {part:?} is not an integer"))
                        })
                    })
                    .collect::<Result<Vec<i32>>>()?;
                let [strength, agility, intelligence]: [i32; 3] =
                    values.try_into().map_err(|v: Vec<i32>| {
                        Error::msg(format!("[unit] hero has {} numbers, not 3", v.len()))
                    })?;
                Some(HeroAttributes {
                    strength,
                    agility,
                    intelligence,
                })
            }
            None => None,
        }
    } else {
        None
    };

    let unknown = parse_hex(section.require("unknown")?)?;
    let unknown: [u8; 2] = unknown
        .try_into()
        .map_err(|v: Vec<u8>| Error::msg(format!("[unit] unknown has {} bytes, not 2", v.len())))?;

    Ok(Unit {
        kind: parse_fourcc(section.require("kind")?)?,
        variation: section.i32("variation")?,
        position: read_vector(section, "position")?,
        rotation: section.f32("rotation")?,
        scale: read_vector(section, "scale")?,
        flags: read_byte(section, "flags")?,
        player: section.i32("player")?,
        unknown,
        hit_points: section.i32("hit_points")?,
        mana: section.i32("mana")?,
        item_table: if has_hero_fields {
            Some(section.i32("item_table")?)
        } else {
            None
        },
        drop_sets: Vec::new(),
        gold: section.i32("gold")?,
        target_acquisition: section.f32("target_acquisition")?,
        hero_level: section.i32("hero_level")?,
        hero,
        inventory: Vec::new(),
        abilities: Vec::new(),
        // Replaced when the [random] section is reached; a unit always has one.
        random: RandomPayload::Any {
            level: [0; 3],
            item_class: 0,
        },
        color: section.i32("color")?,
        waygate: section.i32("waygate")?,
        creation_number: section.i32("creation_number")?,
    })
}

fn read_random(section: &Section) -> Result<RandomPayload> {
    match section.i32("flag")? {
        // `-1` carries no payload at all; it is not the same as "any with zeros".
        -1 => Ok(RandomPayload::None),
        0 => {
            let raw = section.require("level")?;
            let values: Vec<u8> = raw
                .split_whitespace()
                .map(|part| {
                    part.parse()
                        .map_err(|_| Error::msg(format!("[random] level: {part:?} is not a byte")))
                })
                .collect::<Result<Vec<u8>>>()?;
            let [a, b, c]: [u8; 3] = values.try_into().map_err(|v: Vec<u8>| {
                Error::msg(format!("[random] level has {} values, not 3", v.len()))
            })?;
            Ok(RandomPayload::Any {
                level: [a, b, c],
                item_class: read_byte(section, "item_class")?,
            })
        }
        1 => Ok(RandomPayload::Group {
            group: section.i32("group")?,
            position: section.i32("position")?,
        }),
        2 => Ok(RandomPayload::Table {
            choices: Vec::new(),
        }),
        other => Err(Error::msg(format!(
            "[random] flag = {other} is not -1, 0, 1 or 2"
        ))),
    }
}

/// A byte written as a decimal number.
fn read_byte(section: &Section, key: &str) -> Result<u8> {
    let raw = section.require(key)?;
    raw.parse()
        .map_err(|_| Error::msg(format!("[{}] {key} = {raw:?} is not a byte", section.name)))
}

fn vector_text(v: Vec3) -> String {
    [v.x, v.y, v.z]
        .iter()
        .map(|axis| float_text(*axis))
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_vector(section: &Section, key: &str) -> Result<Vec3> {
    let raw = section.require(key)?;
    let mut parts = raw.split_whitespace();
    let mut next = |what: &str| -> Result<f32> {
        let part = parts
            .next()
            .ok_or_else(|| Error::msg(format!("[{}] {key} is missing its {what}", section.name)))?;
        parse_float(part).map_err(|e| Error::msg(format!("[{}] {key}: {e}", section.name)))
    };
    let x = next("x")?;
    let y = next("y")?;
    let z = next("z")?;
    if parts.next().is_some() {
        return Err(Error::msg(format!(
            "[{}] {key} has more than three numbers",
            section.name
        )));
    }
    Ok(Vec3::new(x, y, z))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version 8 unit with one drop set, one inventory item, one ability and a
    /// group random payload, plus a version 7 unit with none of the v8 fields.
    const SAMPLE: &str = "\
[units]
version = \"8\"
subversion = \"11\"

[unit]
kind = \"hfoo\"
variation = \"0\"
position = \"-2048.5 -1536 128\"
rotation = \"0\"
scale = \"1 1 1\"
flags = \"2\"
player = \"0\"
unknown = \"0102\"
hit_points = \"-1\"
mana = \"-1\"
item_table = \"-1\"
gold = \"12500\"
target_acquisition = \"-1\"
hero_level = \"1\"
hero = \"10 11 12\"
color = \"-1\"
waygate = \"-1\"
creation_number = \"42\"

[drop_set]
[drop]
item = \"ratf\"
chance = \"100\"
[drop]
item = \"ratc\"
chance = \"50\"

[inventory]
slot = \"0\"
item = \"ratf\"

[ability]
ability = \"Adef\"
autocast = \"0\"
level = \"1\"

[random]
flag = \"1\"
group = \"7\"
position = \"3\"

[trailing]
bytes = \"DEADBEEF\"
";

    #[test]
    fn the_document_round_trips_through_its_own_text() {
        let file = from_document(&Document::parse(SAMPLE).unwrap()).unwrap();
        let rendered = to_document(&file).render();
        let back = from_document(&Document::parse(&rendered).unwrap()).unwrap();
        assert_eq!(back.to_bytes(), file.to_bytes());
        // Floats are exact, not merely close.
        assert_eq!(file.units[0].position.x.to_bits(), (-2048.5f32).to_bits());
    }

    #[test]
    fn the_nested_tables_attach_to_the_unit_above_them() {
        let file = from_document(&Document::parse(SAMPLE).unwrap()).unwrap();
        let unit = &file.units[0];
        assert_eq!(unit.drop_sets.len(), 1);
        assert_eq!(unit.drop_sets[0].drops.len(), 2);
        assert_eq!(
            unit.drop_sets[0].drops[1].item.as_str().as_deref(),
            Some("ratc")
        );
        assert_eq!(unit.inventory.len(), 1);
        assert_eq!(unit.abilities.len(), 1);
        assert_eq!(
            unit.hero,
            Some(HeroAttributes {
                strength: 10,
                agility: 11,
                intelligence: 12
            })
        );
        assert_eq!(unit.unknown, [1, 2]);
        assert_eq!(file.trailing, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn a_version_7_unit_has_no_v8_fields_and_keeps_its_random_block() {
        let text = SAMPLE
            .replace("version = \"8\"", "version = \"7\"")
            .replace("item_table = \"-1\"\n", "")
            .replace("hero = \"10 11 12\"\n", "");
        let file = from_document(&Document::parse(&text).unwrap()).unwrap();
        assert_eq!(file.units[0].item_table, None);
        assert_eq!(file.units[0].hero, None);
        // The random block is not invented and not dropped: it round-trips as read.
        assert!(matches!(
            file.units[0].random,
            RandomPayload::Group {
                group: 7,
                position: 3
            }
        ));
        let back = from_document(&Document::parse(&to_document(&file).render()).unwrap()).unwrap();
        assert_eq!(back.to_bytes(), file.to_bytes());
    }

    #[test]
    fn a_random_table_is_built_from_its_choice_sections() {
        let text = SAMPLE.replace(
            "[random]\nflag = \"1\"\ngroup = \"7\"\nposition = \"3\"",
            "[random]\nflag = \"2\"\n\n[choice]\nkind = \"hfoo\"\nchance = \"70\"\n\n[choice]\nkind = \"hkni\"\nchance = \"30\"",
        );
        let file = from_document(&Document::parse(&text).unwrap()).unwrap();
        match &file.units[0].random {
            RandomPayload::Table { choices } => {
                assert_eq!(choices.len(), 2);
                assert_eq!(choices[1].chance, 30);
            }
            other => panic!("expected a table, got {other:?}"),
        }
        let back = from_document(&Document::parse(&to_document(&file).render()).unwrap()).unwrap();
        assert_eq!(back.to_bytes(), file.to_bytes());
    }

    #[test]
    fn a_subsection_before_any_unit_is_refused() {
        for section in [
            "[drop_set]",
            "[drop]",
            "[inventory]",
            "[ability]",
            "[random]",
        ] {
            let text = format!("[units]\nversion = \"8\"\nsubversion = \"11\"\n\n{section}\n");
            let err = from_document(&Document::parse(&text).unwrap())
                .unwrap_err()
                .to_string();
            assert!(err.contains("before any [unit]"), "{section}: {err}");
        }
    }

    #[test]
    fn a_drop_before_any_drop_set_is_refused() {
        let text = SAMPLE.replace("[drop_set]\n", "");
        let err = from_document(&Document::parse(&text).unwrap())
            .unwrap_err()
            .to_string();
        assert!(err.contains("before any [drop_set]"), "{err}");
    }

    #[test]
    fn an_unknown_random_flag_is_refused() {
        let text = SAMPLE.replace("flag = \"1\"", "flag = \"9\"");
        let err = from_document(&Document::parse(&text).unwrap())
            .unwrap_err()
            .to_string();
        assert!(err.contains("is not -1, 0, 1 or 2"), "{err}");
    }

    #[test]
    fn the_codec_covers_the_units_file_only() {
        assert!(codec_for("war3mapUnits.doo").is_some());
        assert!(codec_for("WAR3MAPUNITS.DOO").is_some());
        assert!(codec_for("war3map.doo").is_none());
        assert!(codec_for("war3map.w3i").is_none());
    }
}

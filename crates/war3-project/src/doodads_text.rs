//! The text form of `war3map.doo`: the doodads placed on a map.
//!
//! # Layout
//!
//! ```text
//! [doodads]
//! version = "8"
//! subversion = "11"
//! special_version = "9"
//!
//! [doodad]                  ; one section per placed doodad, in file order
//! kind = "LTlt"
//! variation = "0"
//! position = "-2048 -2048 0"  ; three floats, exact (see below)
//! rotation = "0"
//! scale = "1 1 1"
//! flags = "2"
//! life = "100"
//! editor_id = "7"
//!
//! [special]                 ; the cliffs the editor will not let you edit
//! kind = "LTlt"
//! z = "0"
//! x = "-2048"
//! y = "-2048"
//!
//! [trailing]
//! bytes = "…"               ; only when the file had bytes the format does not place
//! ```
//!
//! # Why the coordinates are floats and the special ones are not
//!
//! A placed doodad stores its position as three floats, so they are written with
//! the exact round-tripping form in [`crate::ini::float_text`] — a decimal that
//! does not come back as the same bits would silently move a tree. A *special*
//! doodad stores its coordinates as three integers, which is how the format writes
//! them, so they stay integers here. Making them look alike would be a lie about
//! the file.
//!
//! # What is not modelled
//!
//! A version-8 record can carry a random item table. [`war3_map::DoodadFile`]
//! refuses those records rather than guessing their length, so such a map never
//! reaches this text form; it stays a stored block instead.

use war3_core::{Error, Result, Vec3};
use war3_map::units::{DropSet, ItemDrop};
use war3_map::{Doodad, DoodadFile, SpecialDoodad};

use crate::codecs::{fourcc_text, hex, parse_fourcc, parse_hex, Codec};
use crate::ini::{float_text, parse_float, Document, Section};

/// The codec for `war3map.doo`, if that is the member.
#[must_use]
pub fn codec_for(member: &str) -> Option<Codec> {
    member
        .eq_ignore_ascii_case("war3map.doo")
        .then_some(Codec { to_text, from_text })
}

fn to_text(_member: &str, bytes: &[u8]) -> Result<String> {
    let file = DoodadFile::parse(bytes).map_err(|e| Error::msg(e.to_string()))?;
    Ok(to_document(&file).render())
}

fn from_text(_member: &str, text: &str) -> Result<Vec<u8>> {
    Ok(from_document(&Document::parse(text)?)?.to_bytes())
}

/// Renders a doodad file as a document.
#[must_use]
pub fn to_document(file: &DoodadFile) -> Document {
    let mut doc = Document::new();
    {
        let s = doc.push("doodads");
        s.set("version", file.version.to_string());
        s.set("subversion", file.subversion.to_string());
        s.set("special_version", file.special_version.to_string());
    }

    for doodad in &file.doodads {
        let s = doc.push("doodad");
        s.set("kind", fourcc_text(doodad.kind));
        s.set("variation", doodad.variation.to_string());
        s.set("position", vector_text(doodad.position));
        s.set("rotation", float_text(doodad.rotation));
        s.set("scale", vector_text(doodad.scale));
        s.set("flags", doodad.flags.to_string());
        s.set("life", doodad.life.to_string());
        s.set("item_table", doodad.item_table.to_string());
        s.set("editor_id", doodad.editor_id.to_string());
        // The sets follow the doodad they belong to, which is the order the format
        // stores them in and the rule the reader uses to attach them.
        for set in &doodad.item_sets {
            doc.push("drop_set");
            for drop in &set.drops {
                let d = doc.push("drop");
                d.set("item", fourcc_text(drop.item));
                d.set("chance", drop.chance.to_string());
            }
        }
    }

    for special in &file.special {
        let s = doc.push("special");
        s.set("kind", fourcc_text(special.kind));
        s.set("z", special.z.to_string());
        s.set("x", special.x.to_string());
        s.set("y", special.y.to_string());
    }

    if !file.trailing.is_empty() {
        doc.push("trailing").set("bytes", hex(&file.trailing));
    }
    doc
}

/// Reads a doodad file back from a document.
///
/// # Errors
///
/// A missing `[doodads]` section or key, a vector that is not three numbers, an
/// unparsable id, or a `[special]` before the header. Section order is preserved,
/// because the file's order is the map's order.
pub fn from_document(doc: &Document) -> Result<DoodadFile> {
    let head = doc.require_section("doodads")?;
    let mut doodads = Vec::new();
    let mut special = Vec::new();
    let mut trailing = Vec::new();

    for section in &doc.sections {
        match section.name.as_str() {
            "doodad" => doodads.push(read_doodad(section)?),
            "special" => special.push(SpecialDoodad {
                kind: parse_fourcc(section.require("kind")?)?,
                z: section.i32("z")?,
                x: section.i32("x")?,
                y: section.i32("y")?,
            }),
            "drop_set" => current(&mut doodads, "drop_set")?
                .item_sets
                .push(DropSet { drops: Vec::new() }),
            "drop" => {
                let doodad = current(&mut doodads, "drop")?;
                let set = doodad.item_sets.last_mut().ok_or_else(|| {
                    Error::msg("[drop] appears before any [drop_set]; it belongs to a set")
                })?;
                set.drops.push(ItemDrop {
                    item: parse_fourcc(section.require("item")?)?,
                    chance: section.i32("chance")?,
                });
            }
            "trailing" => trailing = parse_hex(section.require("bytes")?)?,
            _ => {}
        }
    }

    Ok(DoodadFile {
        version: head.i32("version")?,
        subversion: head.i32("subversion")?,
        special_version: head.i32("special_version")?,
        doodads,
        special,
        trailing,
        diagnostics: war3_core::diag::Diagnostics::new(),
    })
}

/// The doodad a sub-section belongs to: the last `[doodad]` seen.
fn current<'a>(doodads: &'a mut [Doodad], what: &str) -> Result<&'a mut Doodad> {
    doodads.last_mut().ok_or_else(|| {
        Error::msg(format!(
            "[{what}] appears before any [doodad]; it belongs to a doodad"
        ))
    })
}

fn read_doodad(section: &Section) -> Result<Doodad> {
    Ok(Doodad {
        kind: parse_fourcc(section.require("kind")?)?,
        variation: section.i32("variation")?,
        position: read_vector(section, "position")?,
        rotation: section.f32("rotation")?,
        scale: read_vector(section, "scale")?,
        flags: read_byte(section, "flags")?,
        life: read_byte(section, "life")?,
        item_table: section.i32("item_table")?,
        // Filled by the `[drop_set]`/`[drop]` sections that follow, which is how the
        // format orders them: a doodad's sets come after the doodad itself.
        item_sets: Vec::new(),
        editor_id: section.i32("editor_id")?,
    })
}

/// A byte written as a decimal number.
///
/// Not the letter form the tileset uses: these two are counts, and a map that
/// stored `76` as the letter `L` would be a different file.
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

    const SAMPLE: &str = "\
[doodads]
version = \"7\"
subversion = \"9\"
special_version = \"9\"

[doodad]
kind = \"LTlt\"
variation = \"2\"
position = \"-2048.5 -1536 128\"
rotation = \"1.5707964\"
scale = \"1 0.5 1\"
flags = \"2\"
life = \"100\"
item_table = \"-1\"
editor_id = \"7\"

[special]
kind = \"LTlt\"
z = \"0\"
x = \"-2048\"
y = \"-1536\"

[trailing]
bytes = \"DEADBEEF\"
";

    #[test]
    fn the_document_round_trips_through_its_own_text() {
        let file = from_document(&Document::parse(SAMPLE).unwrap()).unwrap();
        let rendered = to_document(&file).render();
        let back = from_document(&Document::parse(&rendered).unwrap()).unwrap();
        assert_eq!(back.to_bytes(), file.to_bytes());
        // And the floats are exact, not merely close.
        assert_eq!(file.doodads[0].rotation.to_bits(), 1.5707964f32.to_bits());
        assert_eq!(file.doodads[0].scale.y.to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn order_is_the_map_order_and_is_preserved() {
        let text = SAMPLE.replace(
            "[special]",
            "[doodad]\nkind = \"B000\"\nvariation = \"0\"\nposition = \"0 0 0\"\nrotation = \"0\"\n\
             scale = \"1 1 1\"\nflags = \"0\"\nlife = \"100\"\nitem_table = \"-1\"\neditor_id = \"8\"\n\n[special]",
        );
        let file = from_document(&Document::parse(&text).unwrap()).unwrap();
        assert_eq!(file.doodads.len(), 2);
        assert_eq!(file.doodads[1].kind.as_str().as_deref(), Some("B000"));
        let again = from_document(&Document::parse(&to_document(&file).render()).unwrap()).unwrap();
        assert_eq!(again.to_bytes(), file.to_bytes());
    }

    #[test]
    fn the_special_coordinates_stay_integers() {
        let file = from_document(&Document::parse(SAMPLE).unwrap()).unwrap();
        assert_eq!(file.special[0].x, -2048);
        assert_eq!(file.special[0].y, -1536);
        // A float there would be a different file, so it is refused.
        let text = SAMPLE.replace("x = \"-2048\"", "x = \"-2048.5\"");
        assert!(from_document(&Document::parse(&text).unwrap()).is_err());
    }

    #[test]
    fn a_vector_that_is_not_three_numbers_is_refused() {
        for bad in ["\"1 2\"", "\"1 2 3 4\"", "\"1 2 x\""] {
            let text = SAMPLE.replace("scale = \"1 0.5 1\"", &format!("scale = {bad}"));
            let err = from_document(&Document::parse(&text).unwrap())
                .unwrap_err()
                .to_string();
            assert!(err.contains("scale"), "{bad}: {err}");
        }
    }

    #[test]
    fn trailing_bytes_come_back() {
        let file = from_document(&Document::parse(SAMPLE).unwrap()).unwrap();
        assert_eq!(file.trailing, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn the_codec_covers_war3map_doo_only() {
        assert!(codec_for("war3map.doo").is_some());
        assert!(codec_for("WAR3MAP.DOO").is_some());
        assert!(codec_for("war3mapUnits.doo").is_none());
        assert!(codec_for("war3map.w3u").is_none());
    }
}

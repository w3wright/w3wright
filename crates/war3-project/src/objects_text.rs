//! The text form of object data (`.w3u`, `.w3t`, `.w3a`, `.w3b`, `.w3d`, `.w3h`, `.w3q`).
//!
//! # Layout
//!
//! ```text
//! [objects]
//! kind = "Unit"          ; cross-checked against the member name on the way back
//! version = "2"
//! extra_tables = "0"     ; empty tables that followed the custom table
//!
//! [original]             ; marker: the file carried this table, even if empty
//! [object]
//! id = "hpea"
//! base_id = "0000"       ; zero for an original entry: it *is* the base object
//! [mod]
//! field = "unam"
//! type = "string"        ; integer | real | unreal | string
//! value = "Custom Peasant"
//!
//! [custom]               ; a created object, naming the object it inherits from
//! [object]
//! id = "h001"
//! base_id = "hpea"
//! [mod]
//! field = "uhpm"
//! type = "integer"
//! value = "500"
//! level = "1"            ; only for the kinds that carry a level block
//! data = "0"
//!
//! [trailing]
//! bytes = "…"            ; only when the file had bytes the format does not place
//! ```
//!
//! # The three things this form refuses to do
//!
//! - **It does not name fields.** A field is its four-character id (`unam`), not
//!   `Name`. Names come from `war3-meta`, which needs a game install; a text form
//!   that silently degraded to ids on a machine without one would make the same
//!   file read differently in two places. Ids are honest everywhere.
//! - **It does not normalise a boolean into `0`/`1`.** Integers are written and
//!   read as the integer that was there, because `2` and `1` are different bytes
//!   even when the game reads both as "yes".
//! - **It does not drop the unexplained.** `extra_tables` and `trailing` are in
//!   the model only so a write can reproduce the input, and they are in the text
//!   for the same reason.
//!
//! A `tail` line appears only when those four bytes are not zero, and a missing
//! line means zeros: the model's value is a fixed array, so "absent" cannot be
//! confused with anything else.

use war3_core::{Error, Result};
use war3_object::{
    FieldValue, Levelled, Modification, Object, ObjectFile, ObjectKind, ObjectTable,
};

use crate::codecs::{fourcc_text, hex, parse_fourcc, Codec};
use crate::ini::{float_text, parse_float, Document, Section};

/// The codec for a member, if it is one of the seven object files.
#[must_use]
pub fn codec_for(member: &str) -> Option<Codec> {
    kind_for(member).map(|_| Codec { to_text, from_text })
}

/// The object kind a member name holds, if any.
fn kind_for(member: &str) -> Option<ObjectKind> {
    match member.to_ascii_lowercase().as_str() {
        "war3map.w3u" => Some(ObjectKind::Unit),
        "war3map.w3t" => Some(ObjectKind::Item),
        "war3map.w3a" => Some(ObjectKind::Ability),
        "war3map.w3b" => Some(ObjectKind::Destructable),
        "war3map.w3d" => Some(ObjectKind::Doodad),
        "war3map.w3h" => Some(ObjectKind::Buff),
        "war3map.w3q" => Some(ObjectKind::Upgrade),
        _ => None,
    }
}

fn to_text(member: &str, bytes: &[u8]) -> Result<String> {
    let kind = kind_for(member)
        .ok_or_else(|| Error::msg(format!("{member} is not one of the seven object members")))?;
    let file = ObjectFile::parse(kind, bytes).map_err(|e| Error::msg(e.to_string()))?;
    Ok(to_document(&file).render())
}

fn from_text(member: &str, text: &str) -> Result<Vec<u8>> {
    let document = Document::parse(text)?;
    Ok(from_document(member, &document)?.to_bytes())
}

/// Renders an object file as a document.
#[must_use]
pub fn to_document(file: &ObjectFile) -> Document {
    let levelled = war3_object::is_levelled(file.kind);
    let mut doc = Document::new();
    {
        let s = doc.push("objects");
        s.set("kind", format!("{:?}", file.kind));
        s.set("version", file.version.to_string());
        s.set("extra_tables", file.extra_tables.to_string());
    }

    for (name, objects) in [
        ("original", &file.table.original),
        ("custom", &file.table.custom),
    ] {
        doc.push(name);
        for object in objects {
            let s = doc.push("object");
            s.set("id", fourcc_text(object.id));
            s.set("base_id", fourcc_text(object.base_id));
            for modification in &object.modifications {
                let s = doc.push("mod");
                s.set("field", fourcc_text(modification.field));
                let (tag, value) = value_text(&modification.value);
                s.set("type", tag);
                s.set("value", value);
                if levelled {
                    let level = modification.level.unwrap_or(Levelled {
                        level: 0,
                        data_indicator: 0,
                    });
                    s.set("level", level.level.to_string());
                    s.set("data", level.data_indicator.to_string());
                }
                if modification.tail != [0; 4] {
                    s.set("tail", hex(&modification.tail));
                }
            }
        }
    }

    if !file.trailing.is_empty() {
        doc.push("trailing").set("bytes", hex(&file.trailing));
    }
    doc
}

/// Reads an object file back from a document.
///
/// # Errors
///
/// A missing `[objects]` section, a `kind` that disagrees with the member the text
/// was loaded from, an unknown value type, a malformed id, or a level block that
/// the kind requires and the text does not carry. All of these are reported
/// against the section they came from, because "the text is wrong" is not an
/// actionable message.
pub fn from_document(member: &str, doc: &Document) -> Result<ObjectFile> {
    let head = doc.require_section("objects")?;
    let kind_name = head.require("kind")?;
    let kind = kind_for(member).ok_or_else(|| {
        Error::msg(format!(
            "{member} is not one of the seven object members, so its objects cannot be read back"
        ))
    })?;
    if kind_name != format!("{kind:?}") {
        return Err(Error::msg(format!(
            "[objects] kind = {kind_name:?} but the text was loaded from {member}, which holds \
             {kind:?}; one of the two is wrong"
        )));
    }

    let levelled = war3_object::is_levelled(kind);
    let mut table = ObjectTable::default();
    let mut trailing = Vec::new();
    // Walked in order, not looked up by name: a `[mod]` belongs to the `[object]`
    // above it, and an `[object]` to the `[original]`/`[custom]` above that.
    let mut current_table: Option<bool> = None; // true = custom
    let mut current_object: Option<Object> = None;

    let flush = |table: &mut ObjectTable, which: Option<bool>, object: Option<Object>| {
        if let (Some(custom), Some(object)) = (which, object) {
            if custom {
                table.custom.push(object);
            } else {
                table.original.push(object);
            }
        }
    };

    for section in &doc.sections {
        match section.name.as_str() {
            "original" | "custom" => {
                flush(&mut table, current_table, current_object.take());
                current_table = Some(section.name == "custom");
            }
            "object" => {
                flush(&mut table, current_table, current_object.take());
                current_object = Some(Object {
                    id: parse_fourcc(section.require("id")?)?,
                    base_id: parse_fourcc(section.require("base_id")?)?,
                    modifications: Vec::new(),
                });
            }
            "mod" => {
                let object = current_object.as_mut().ok_or_else(|| {
                    Error::msg("[mod] appears before any [object]; it belongs to an object")
                })?;
                object
                    .modifications
                    .push(read_modification(section, levelled)?);
            }
            "trailing" => trailing = section.hex("bytes")?,
            _ => {}
        }
    }
    flush(&mut table, current_table, current_object);

    Ok(ObjectFile {
        kind,
        version: head.i32("version")?,
        table,
        extra_tables: head.i32("extra_tables")? as usize,
        trailing,
        diagnostics: war3_core::diag::Diagnostics::new(),
    })
}

fn read_modification(section: &Section, levelled: bool) -> Result<Modification> {
    let field = parse_fourcc(section.require("field")?)?;
    let tag = section.require("type")?;
    let raw = section.require("value")?;
    let value = match tag {
        "integer" => FieldValue::Integer(
            raw.parse()
                .map_err(|_| Error::msg(format!("[mod] value = {raw:?} is not an integer")))?,
        ),
        "real" => FieldValue::Real(parse_float(raw)?),
        "unreal" => FieldValue::Unreal(parse_float(raw)?),
        "string" => FieldValue::String(raw.to_string()),
        other => {
            return Err(Error::msg(format!(
                "[mod] type = {other:?} is not one of integer / real / unreal / string"
            )))
        }
    };
    let level = if levelled {
        Some(Levelled {
            level: section.i32("level")?,
            data_indicator: section.i32("data")?,
        })
    } else {
        None
    };
    let tail = if section.get("tail").is_some() {
        let bytes = section.hex("tail")?;
        let tail: [u8; 4] = bytes
            .try_into()
            .map_err(|v: Vec<u8>| Error::msg(format!("[mod] tail has {} bytes, not 4", v.len())))?;
        tail
    } else {
        [0; 4]
    };
    Ok(Modification {
        field,
        value,
        level,
        tail,
    })
}

fn value_text(value: &FieldValue) -> (&'static str, String) {
    match value {
        FieldValue::Integer(v) => ("integer", v.to_string()),
        FieldValue::Real(v) => ("real", float_text(*v)),
        FieldValue::Unreal(v) => ("unreal", float_text(*v)),
        FieldValue::String(v) => ("string", v.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small but complete unit file: one modified original, one custom object.
    const SAMPLE: &str = "\
[objects]
kind = \"Unit\"
version = \"2\"
extra_tables = \"1\"

[original]
[object]
id = \"hpea\"
base_id = \"0x00000000\"
[mod]
field = \"unam\"
type = \"string\"
value = \"Custom Peasant\"
[mod]
field = \"uhpm\"
type = \"integer\"
value = \"300\"
tail = \"01020304\"

[custom]
[object]
id = \"h001\"
base_id = \"hpea\"
[mod]
field = \"urel\"
type = \"real\"
value = \"1.5\"

[trailing]
bytes = \"DEADBEEF\"
";

    #[test]
    fn the_document_round_trips_through_its_own_text() {
        let file = from_document("war3map.w3u", &Document::parse(SAMPLE).unwrap()).unwrap();
        let text = to_document(&file).render();
        let back = from_document("war3map.w3u", &Document::parse(&text).unwrap()).unwrap();
        assert_eq!(back.to_bytes(), file.to_bytes());
        assert_eq!(file.to_bytes(), back.to_bytes());
    }

    #[test]
    fn the_two_tables_stay_apart() {
        let file = from_document("war3map.w3u", &Document::parse(SAMPLE).unwrap()).unwrap();
        assert_eq!(file.table.original.len(), 1);
        assert_eq!(file.table.custom.len(), 1);
        assert_eq!(
            file.table.original[0].id.as_str().as_deref(),
            Some("hpea"),
            "original"
        );
        assert_eq!(
            file.table.custom[0].id.as_str().as_deref(),
            Some("h001"),
            "custom"
        );
        assert_eq!(
            file.table.custom[0].base_id.as_str().as_deref(),
            Some("hpea"),
            "the created object names its parent"
        );
        assert_eq!(file.extra_tables, 1);
        assert_eq!(file.trailing, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn a_tail_is_written_only_when_it_is_not_zero() {
        let file = from_document("war3map.w3u", &Document::parse(SAMPLE).unwrap()).unwrap();
        let text = to_document(&file).render();
        // Exactly one modification carried a non-zero tail, and it is the second
        // one of the original object.
        assert_eq!(text.matches("tail = ").count(), 1, "{text}");
        assert_eq!(file.table.original[0].modifications[1].tail, [1, 2, 3, 4]);
        assert_eq!(file.table.custom[0].modifications[0].tail, [0; 4]);
    }

    #[test]
    fn a_levelled_kind_carries_and_requires_its_level_block() {
        // Every modification of a levelled kind carries one, so the sample needs
        // the block on all three of its modifications.
        let text = SAMPLE
            .replace("kind = \"Unit\"", "kind = \"Ability\"")
            .replace(
                "value = \"Custom Peasant\"",
                "value = \"Custom Peasant\"\nlevel = \"2\"\ndata = \"3\"",
            )
            .replace(
                "value = \"300\"",
                "value = \"300\"\nlevel = \"1\"\ndata = \"0\"",
            )
            .replace(
                "value = \"1.5\"",
                "value = \"1.5\"\nlevel = \"1\"\ndata = \"0\"",
            );
        let file = from_document("war3map.w3a", &Document::parse(&text).unwrap()).unwrap();
        assert_eq!(
            file.table.original[0].modifications[0].level,
            Some(Levelled {
                level: 2,
                data_indicator: 3
            })
        );
        let rendered = to_document(&file).render();
        assert_eq!(rendered.matches("level = ").count(), 3, "{rendered}");

        // The block is required where the kind carries one: a text form that had
        // dropped it would be written back with invented level numbers.
        let without: String = text
            .lines()
            .filter(|line| !line.starts_with("level = ") && !line.starts_with("data = "))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(from_document("war3map.w3a", &Document::parse(&without).unwrap()).is_err());
    }

    #[test]
    fn a_kind_that_disagrees_with_the_member_is_refused() {
        let err = from_document("war3map.w3t", &Document::parse(SAMPLE).unwrap())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Unit") && err.contains("Item"), "{err}");
    }

    #[test]
    fn an_unknown_value_type_is_refused_rather_than_guessed() {
        let text = SAMPLE.replace("type = \"integer\"", "type = \"money\"");
        let err = from_document("war3map.w3u", &Document::parse(&text).unwrap())
            .unwrap_err()
            .to_string();
        assert!(err.contains("money"), "{err}");
    }

    #[test]
    fn a_modification_before_any_object_is_refused() {
        let text = "[objects]\nkind = \"Unit\"\nversion = \"2\"\nextra_tables = \"0\"\n\
                    [mod]\nfield = \"unam\"\ntype = \"string\"\nvalue = \"x\"\n";
        let err = from_document("war3map.w3u", &Document::parse(text).unwrap())
            .unwrap_err()
            .to_string();
        assert!(err.contains("before any [object]"), "{err}");
    }

    #[test]
    fn the_codec_covers_the_seven_object_members_and_nothing_else() {
        for member in [
            "war3map.w3u",
            "WAR3MAP.W3T",
            "war3map.w3a",
            "war3map.w3b",
            "war3map.w3d",
            "war3map.w3h",
            "war3map.w3q",
        ] {
            assert!(codec_for(member).is_some(), "{member}");
        }
        assert!(codec_for("war3map.w3i").is_none());
        assert!(codec_for("war3map.w3e").is_none());
    }
}

//! `war3map.doo` — the doodads placed on a map: trees, rocks, and the other
//! destructables.
//!
//! # Layout
//!
//! ```text
//! char[4] magic "W3do"
//! int32   version
//! int32   subversion
//! int32   record count
//! record[count]
//! int32   special-doodad version   (0 in every file seen)
//! int32   special-doodad count
//! special[count]                   (16 bytes each)
//! ```
//!
//! A version 7 record is a fixed 42 bytes: type, variation, position,
//! rotation, scale, flags, life, and the World Editor's doodad number. Version
//! 8 inserts an item-table pointer and an item-set count between the life byte
//! and the editor number, making the record 50 bytes when both are empty —
//! which is every real file seen here.
//!
//! # How this was verified
//!
//! Not from a document alone. Walking every record must land exactly on the
//! end of the file, and it does on 136 local maps: 5317 records in
//! `(4)LostTemple.w3m` (v7/sub9), 5273 in DotA IMBA (v7/sub11), and 3338 to
//! 8362 records each across the Frozen Throne ladder maps (v8/sub11). The
//! version-8 item sets are the one path no sample exercises, so they are
//! reported rather than guessed at.

use war3_core::diag::Diagnostics;
use war3_core::{FourCC, ParseError, Vec3};

use crate::cursor::Cursor;
use crate::units::{DropSet, ItemDrop};

/// Magic shared by `war3map.doo` and `war3mapUnits.doo`.
pub const DOO_MAGIC: [u8; 4] = *b"W3do";

/// Length of a version 7 record.
const V7_RECORD_LEN: usize = 42;
/// Length of a version 8 record whose item fields are empty.
const V8_RECORD_LEN: usize = 50;
/// Length of one special-doodad entry.
const SPECIAL_LEN: usize = 16;
/// First version whose records carry the item fields.
const ITEM_FIELD_VERSION: i32 = 8;
/// Versions this build reads.
pub const SUPPORTED_VERSIONS: &[i64] = &[7, 8];
/// Upper bound on a declared count, so a bad count fails before allocating.
const MAX_RECORDS: i32 = 1 << 20;

/// One placed doodad.
#[derive(Debug, Clone, PartialEq)]
pub struct Doodad {
    /// Doodad type, e.g. `LTlt` for a Lordaeron summer tree.
    pub kind: FourCC,
    /// Which variant of the model to use.
    pub variation: i32,
    /// World position.
    pub position: Vec3,
    /// Rotation in radians.
    pub rotation: f32,
    /// Per-axis scale.
    pub scale: Vec3,
    /// Bit field; `0x02` is "visible and solid" on every file seen.
    pub flags: u8,
    /// Life as a percentage of the type's default.
    pub life: u8,
    /// Pointer to a random item table in `war3map.w3i`, or `-1` for none.
    pub item_table: i32,
    /// Items the doodad drops, inline in the record.
    ///
    /// The same shape a placed unit uses, so it is the same type: one definition,
    /// and a reader that understands one understands both.
    pub item_sets: Vec<DropSet>,
    /// The World Editor's doodad number, unique per map.
    pub editor_id: i32,
}

/// A special doodad: the ones cliffs are made of, which the editor will not let
/// you edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecialDoodad {
    /// Doodad type.
    pub kind: FourCC,
    /// Stored z coordinate. The reader keeps it as an integer because that is
    /// how it is stored — it is not a float.
    pub z: i32,
    /// Stored x coordinate.
    pub x: i32,
    /// Stored y coordinate.
    pub y: i32,
}

/// A parsed `war3map.doo`.
#[derive(Debug, Clone)]
pub struct DoodadFile {
    /// File version: 7 for the fixed record, 8 for the item-bearing one.
    pub version: i32,
    /// Sub-version. Observed values are 9 and 11, and it does **not** select the
    /// record layout — the version field does.
    pub subversion: i32,
    /// Version word of the special-doodad block.
    pub special_version: i32,
    /// The placed doodads.
    pub doodads: Vec<Doodad>,
    /// The special doodads that follow them.
    pub special: Vec<SpecialDoodad>,
    /// Bytes after the last special doodad that the format does not place.
    ///
    /// Reported as [`war3_core::DiagnosticCode::DooTrailingBytes`] **and kept**,
    /// for the same reason `war3map.w3i` keeps its leftovers: a file this build
    /// cannot fully place must not come back quietly shortened.
    pub trailing: Vec<u8>,
    /// Diagnostics collected while parsing.
    pub diagnostics: Diagnostics,
}

/// `len` as the `i32` the format uses for count words.
fn len<T>(list: &[T]) -> i32 {
    i32::try_from(list.len()).unwrap_or(i32::MAX)
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

impl DoodadFile {
    /// Parses `war3map.doo`.
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
                format: "war3map.doo",
                found: i64::from(version),
                supported: SUPPORTED_VERSIONS,
            });
        }
        let count = cursor.i32()?;
        if !(0..=MAX_RECORDS).contains(&count) {
            return Err(ParseError::BadField {
                field: "war3map.doo record count",
                reason: format!("{count} is not a plausible number of doodads"),
            });
        }

        let record_len = if version >= ITEM_FIELD_VERSION {
            V8_RECORD_LEN
        } else {
            V7_RECORD_LEN
        };
        let mut doodads = Vec::with_capacity(count as usize);
        for index in 0..count {
            let start = cursor.position();
            let kind = cursor.fourcc()?;
            let variation = cursor.i32()?;
            let position = Vec3::new(cursor.f32()?, cursor.f32()?, cursor.f32()?);
            let rotation = cursor.f32()?;
            let scale = Vec3::new(cursor.f32()?, cursor.f32()?, cursor.f32()?);
            let flags = cursor.u8()?;
            let life = cursor.u8()?;
            let (item_table, item_sets) = if version >= ITEM_FIELD_VERSION {
                let pointer = cursor.i32()?;
                let sets = cursor.i32()?;
                if !(0..=MAX_RECORDS).contains(&sets) {
                    return Err(ParseError::BadField {
                        field: "war3map.doo item sets",
                        reason: format!(
                            "record {index} at offset {start} declares {sets} item sets, which is \
                             not a plausible number"
                        ),
                    });
                }
                let mut item_sets = Vec::with_capacity(sets as usize);
                for _ in 0..sets {
                    let items = cursor.i32()?;
                    if !(0..=MAX_RECORDS).contains(&items) {
                        return Err(ParseError::BadField {
                            field: "war3map.doo item set size",
                            reason: format!(
                                "record {index} at offset {start} declares {items} items in a set, \
                                 which is not a plausible number"
                            ),
                        });
                    }
                    let mut drops = Vec::with_capacity(items as usize);
                    for _ in 0..items {
                        drops.push(ItemDrop {
                            item: cursor.fourcc()?,
                            chance: cursor.i32()?,
                        });
                    }
                    item_sets.push(DropSet { drops });
                }
                (pointer, item_sets)
            } else {
                (-1, Vec::new())
            };
            let editor_id = cursor.i32()?;
            // The record is a fixed length only while there are no item sets: each
            // set costs a count word and each item costs an id and a chance word.
            debug_assert_eq!(
                cursor.position() - start,
                record_len
                    + 4 * item_sets.len()
                    + 8 * item_sets.iter().map(|set| set.drops.len()).sum::<usize>()
            );
            doodads.push(Doodad {
                kind,
                variation,
                position,
                rotation,
                scale,
                flags,
                life,
                item_table,
                item_sets,
                editor_id,
            });
        }

        let special_version = cursor.i32()?;
        let special_count = cursor.i32()?;
        if !(0..=MAX_RECORDS).contains(&special_count) {
            return Err(ParseError::BadField {
                field: "special doodad count",
                reason: format!("{special_count} is not plausible"),
            });
        }
        let mut special = Vec::with_capacity(special_count as usize);
        for _ in 0..special_count {
            let start = cursor.position();
            special.push(SpecialDoodad {
                kind: cursor.fourcc()?,
                z: cursor.i32()?,
                x: cursor.i32()?,
                y: cursor.i32()?,
            });
            debug_assert_eq!(cursor.position() - start, SPECIAL_LEN);
        }

        let mut diagnostics = Diagnostics::new();
        // Kept, not merely noted: a write has to reproduce the file, and bytes this
        // build cannot place are exactly the ones a normalising writer drops.
        let leftover = cursor.remaining();
        let trailing = cursor.take(leftover)?.to_vec();
        if leftover > 0 {
            diagnostics.push(war3_core::Diagnostic::warn(
                war3_core::DiagnosticCode::DooTrailingBytes,
                format!("{leftover} bytes follow the special doodads; they are kept verbatim"),
            ));
        }

        Ok(Self {
            version,
            subversion,
            special_version,
            doodads,
            special,
            trailing,
            diagnostics,
        })
    }

    /// Serialises back to `war3map.doo` bytes.
    ///
    /// The version-8 item fields are written from the model, so a record with a
    /// random item table round-trips like any other. That branch was once refused
    /// outright ("no sample has ever carried it"); a real map disproved that, and
    /// the layout was then taken from
    /// [WC3MapSpecification `Doodads/8_11.md`](https://github.com/ChiefOfGxBxL/WC3MapSpecification)
    /// rather than guessed — pointer, set count, then per set a count and
    /// `(item, chance)` pairs.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&DOO_MAGIC);
        push_i32(&mut out, self.version);
        push_i32(&mut out, self.subversion);
        push_i32(&mut out, len(&self.doodads));

        for doodad in &self.doodads {
            out.extend_from_slice(&doodad.kind.to_bytes());
            push_i32(&mut out, doodad.variation);
            push_f32(&mut out, doodad.position.x);
            push_f32(&mut out, doodad.position.y);
            push_f32(&mut out, doodad.position.z);
            push_f32(&mut out, doodad.rotation);
            push_f32(&mut out, doodad.scale.x);
            push_f32(&mut out, doodad.scale.y);
            push_f32(&mut out, doodad.scale.z);
            out.push(doodad.flags);
            out.push(doodad.life);
            if self.version >= ITEM_FIELD_VERSION {
                push_i32(&mut out, doodad.item_table);
                push_i32(&mut out, len(&doodad.item_sets));
                for set in &doodad.item_sets {
                    push_i32(&mut out, len(&set.drops));
                    for drop in &set.drops {
                        out.extend_from_slice(&drop.item.to_bytes());
                        push_i32(&mut out, drop.chance);
                    }
                }
            }
            push_i32(&mut out, doodad.editor_id);
        }

        push_i32(&mut out, self.special_version);
        push_i32(&mut out, len(&self.special));
        for special in &self.special {
            out.extend_from_slice(&special.kind.to_bytes());
            push_i32(&mut out, special.z);
            push_i32(&mut out, special.x);
            push_i32(&mut out, special.y);
        }

        out.extend_from_slice(&self.trailing);
        out
    }

    /// How the doodad flags byte is usually read.
    ///
    /// The file stores a small integer rather than independent bits, so this
    /// returns a name for the three values seen in real maps.
    #[must_use]
    pub const fn flags_name(flags: u8) -> &'static str {
        match flags {
            0 => "invisible non-solid",
            1 => "visible non-solid",
            2 => "visible solid",
            _ => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: &[u8; 4], flags: u8, life: u8, editor_id: i32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(kind);
        b.extend_from_slice(&3i32.to_le_bytes()); // variation
        for v in [100.0f32, -200.0, 50.0, 1.5] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for v in [1.0f32, 1.0, 1.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.push(flags);
        b.push(life);
        b.extend_from_slice(&editor_id.to_le_bytes());
        assert_eq!(b.len(), V7_RECORD_LEN);
        b
    }

    fn build(version: i32, subversion: i32, records: &[Vec<u8>], specials: usize) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&DOO_MAGIC);
        b.extend_from_slice(&version.to_le_bytes());
        b.extend_from_slice(&subversion.to_le_bytes());
        b.extend_from_slice(&(records.len() as i32).to_le_bytes());
        for r in records {
            if version >= ITEM_FIELD_VERSION {
                // v8 inserts the two item fields before the editor number.
                b.extend_from_slice(&r[..38]);
                b.extend_from_slice(&(-1i32).to_le_bytes());
                b.extend_from_slice(&0i32.to_le_bytes());
                b.extend_from_slice(&r[38..]);
            } else {
                b.extend_from_slice(r);
            }
        }
        b.extend_from_slice(&0i32.to_le_bytes()); // special version
        b.extend_from_slice(&(specials as i32).to_le_bytes());
        for i in 0..specials {
            b.extend_from_slice(b"YBlm");
            b.extend_from_slice(&(i as i32).to_le_bytes());
            b.extend_from_slice(&(i as i32 * 2).to_le_bytes());
            b.extend_from_slice(&(i as i32 * 3).to_le_bytes());
        }
        b
    }

    #[test]
    fn every_fixture_writes_back_byte_for_byte() {
        for version in [7, 8] {
            for specials in [0, 1, 3] {
                let bytes = build(
                    version,
                    11,
                    &[record(b"LTlt", 2, 100, 7), record(b"B000", 0, 50, 9)],
                    specials,
                );
                let parsed = DoodadFile::parse(&bytes).unwrap();
                assert_eq!(
                    parsed.to_bytes(),
                    bytes,
                    "version {version} with {specials} special doodads"
                );
            }
        }
    }

    #[test]
    fn bytes_after_the_last_special_doodad_are_kept_not_just_reported() {
        let mut bytes = build(7, 9, &[record(b"LTlt", 2, 100, 1)], 0);
        bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let parsed = DoodadFile::parse(&bytes).unwrap();
        assert_eq!(parsed.trailing, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(
            parsed.to_bytes(),
            bytes,
            "a write must not shorten a file it only half places"
        );
    }

    #[test]
    fn parses_a_version_7_file() {
        let bytes = build(7, 9, &[record(b"LTlt", 2, 100, 2244)], 1);
        let file = DoodadFile::parse(&bytes).unwrap();
        assert_eq!(file.version, 7);
        assert_eq!(file.subversion, 9);
        assert_eq!(file.doodads.len(), 1);
        let d = &file.doodads[0];
        assert_eq!(d.kind.to_bytes(), *b"LTlt");
        assert_eq!(d.variation, 3);
        assert_eq!(d.position.x, 100.0);
        assert_eq!(d.rotation, 1.5);
        assert_eq!(d.scale.z, 1.0);
        assert_eq!(d.flags, 2);
        assert_eq!(d.life, 100);
        assert_eq!(d.editor_id, 2244);
        assert_eq!(file.special.len(), 1);
        assert_eq!(file.special[0].kind.to_bytes(), *b"YBlm");
        assert!(file.diagnostics.is_empty());
    }

    #[test]
    fn parses_a_version_8_file() {
        let bytes = build(
            8,
            11,
            &[record(b"LTlt", 2, 100, 7), record(b"YBlm", 0, 1, 9)],
            0,
        );
        let file = DoodadFile::parse(&bytes).unwrap();
        assert_eq!(file.version, 8);
        assert_eq!(file.doodads.len(), 2);
        assert_eq!(file.doodads[1].editor_id, 9);
        assert_eq!(file.special_version, 0);
    }

    #[test]
    fn a_version_8_item_table_is_read_and_written_back() {
        // The fixture builder's own record, with the item sets spliced in before
        // the editor id, which is the last four bytes of a version-8 record.
        let mut base = record(b"LTlt", 2, 100, 7);
        let editor_id = base.split_off(base.len() - 4);
        base.extend_from_slice(&(-1i32).to_le_bytes());
        base.extend_from_slice(&1i32.to_le_bytes());
        base.extend_from_slice(&2i32.to_le_bytes());
        base.extend_from_slice(b"ratf");
        base.extend_from_slice(&100i32.to_le_bytes());
        base.extend_from_slice(b"ratc");
        base.extend_from_slice(&50i32.to_le_bytes());
        base.extend_from_slice(&editor_id);
        // Assembled here rather than through `build`, so the header is exactly what
        // `parse` reads: magic, version, subversion, count, records, then the
        // special-doodad block.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&DOO_MAGIC);
        bytes.extend_from_slice(&8i32.to_le_bytes());
        bytes.extend_from_slice(&11i32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&base);
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());

        let file = DoodadFile::parse(&bytes).unwrap();
        let doodad = &file.doodads[0];
        assert_eq!(doodad.item_table, -1);
        assert_eq!(doodad.item_sets.len(), 1);
        assert_eq!(doodad.item_sets[0].drops.len(), 2);
        assert_eq!(
            doodad.item_sets[0].drops[1].item.as_str().as_deref(),
            Some("ratc")
        );
        assert_eq!(doodad.item_sets[0].drops[1].chance, 50);
        assert_eq!(doodad.editor_id, 7, "the editor id follows the item sets");
        assert_eq!(
            file.to_bytes(),
            bytes,
            "an item table has to come back as well as be read"
        );
    }

    #[test]
    fn an_unsupported_version_is_named() {
        let bytes = build(6, 9, &[], 0);
        assert!(matches!(
            DoodadFile::parse(&bytes),
            Err(ParseError::UnsupportedVersion { found: 6, .. })
        ));
    }

    #[test]
    fn a_wrong_magic_is_rejected() {
        let mut bytes = build(7, 9, &[], 0);
        bytes[0] = b'X';
        assert!(matches!(
            DoodadFile::parse(&bytes),
            Err(ParseError::BadMagic { .. })
        ));
    }

    #[test]
    fn trailing_bytes_are_reported() {
        let mut bytes = build(7, 9, &[], 0);
        bytes.extend_from_slice(&[0xAB; 4]);
        let file = DoodadFile::parse(&bytes).unwrap();
        assert!(file
            .diagnostics
            .items()
            .iter()
            .any(|d| d.code == war3_core::DiagnosticCode::DooTrailingBytes));
    }
}

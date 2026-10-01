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
    /// Diagnostics collected while parsing.
    pub diagnostics: Diagnostics,
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
            if version >= ITEM_FIELD_VERSION {
                let pointer = cursor.i32()?;
                let sets = cursor.i32()?;
                // The record length past this point depends on the item sets,
                // which no sample has ever carried. Guessing would desynchronise
                // every later record, so this stops and says so.
                if pointer != -1 || sets != 0 {
                    return Err(ParseError::BadField {
                        field: "war3map.doo item sets",
                        reason: format!(
                            "record {index} at offset {start} carries a random item table \
                             (pointer {pointer}, {sets} item sets); this build does not read those"
                        ),
                    });
                }
            }
            let editor_id = cursor.i32()?;
            debug_assert_eq!(cursor.position() - start, record_len);
            doodads.push(Doodad {
                kind,
                variation,
                position,
                rotation,
                scale,
                flags,
                life,
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
        if !cursor.is_at_end() {
            diagnostics.push(war3_core::Diagnostic::warn(
                war3_core::DiagnosticCode::DooTrailingBytes,
                format!(
                    "{} bytes follow the special doodads; keeping the parse but noting them",
                    cursor.remaining()
                ),
            ));
        }

        Ok(Self {
            version,
            subversion,
            special_version,
            doodads,
            special,
            diagnostics,
        })
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
    fn a_version_8_item_table_is_reported_not_guessed() {
        let mut bytes = build(8, 11, &[record(b"LTlt", 2, 100, 7)], 0);
        // The item-table pointer sits 38 bytes into the record, at offset 16.
        bytes[16 + 8 + 12 + 4 + 12 + 2..16 + 8 + 12 + 4 + 12 + 2 + 4]
            .copy_from_slice(&5i32.to_le_bytes());
        let err = DoodadFile::parse(&bytes).unwrap_err();
        assert!(
            matches!(err, ParseError::BadField { field, .. } if field.contains("item sets")),
            "{err}"
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

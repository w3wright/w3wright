//! The object file layout and its model.
//!
//! # Layout
//!
//! ```text
//! int32 version                       // 2 in every file seen; 1 accepted too
//! int32 originalCount                 // Blizzard objects the map modified
//!   { 4cc id;      4cc 0;      int32 modCount; mod[modCount] }
//! int32 customCount                   // objects the map created
//!   { 4cc parentId; 4cc newId; int32 modCount; mod[modCount] }
//! ```
//!
//! A modification is
//!
//! ```text
//! 4cc   fieldId
//! int32 type                          // 0 int, 1 real, 2 unreal, 3 string
//! int32 level; int32 dataIndicator    // only for the levelled kinds, see below
//! value                               // 4 / 4 / NUL-terminated bytes
//! 4 bytes                             // end token, ignored by the game
//! ```
//!
//! # The table block repeats, and the number of repetitions varies
//!
//! The published layout says the `count` plus entries block appears "1 or 2
//! times (must check if EOF)", so a reader that always consumes exactly two is
//! reading a guess rather than the format. Real files do carry a third: 11
//! members across four maps in the corpus end with one more `int32`, always
//! zero — an extra table with no entries. Those bytes used to be reported as
//! unexplained trailing data.
//!
//! The count of tables is not *meaningful*, because an empty table carries
//! nothing, but it is recorded — see [`ObjectFile::extra_tables`]. A writer that
//! emits two is writing a valid file, and one that emits what the input carried
//! reproduces it. An extra table that is *not* empty is still reported, since
//! that would be data this build cannot place.
//!
//! # The two rules that are easy to miss
//!
//! 1. **Whether a modification carries `level` and `dataIndicator` is decided by
//!    the object *kind*, not by the field.** Only abilities, doodads and
//!    upgrades do. Read a units file as levelled and every record after the
//!    first desynchronises — which is exactly how this was verified: each real
//!    file walks to the exact end of the file only under the setting its kind
//!    predicts.
//! 2. **Identifier ordering is semantic when writing.** An uppercase first
//!    letter marks a hero, and the game needs uppercase-id objects before
//!    lowercase-id ones or "the hero's data comes out wrong". Reading preserves
//!    file order; [`ObjectKind::sort_rank`] is the rule a writer will need.
//!
//! # What this does not do
//!
//! Unknown field ids and type mismatches are **not** dropped silently. YDWE
//! ignores an unknown field and discards a mismatched one; here the value is
//! kept as parsed and the caller is told via diagnostics, because a silently
//! dropped modification is indistinguishable from one that was never there.
//!
//! # Writing
//!
//! [`ObjectFile::to_bytes`] reproduces the bytes `parse` was given, not a tidied
//! version of them, because the prototype's hard metric is that a member
//! survives `extract → build` byte for byte. Three things exist in the model for
//! no other reason than that: [`Modification::tail`], [`ObjectFile::extra_tables`]
//! and [`ObjectFile::trailing`]. Each is bytes the format leaves to the writer
//! or that this build cannot place — which is exactly when a write is tempted to
//! invent them.

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};
use war3_core::{FourCC, ParseError};

use war3_meta::ObjectKind;

/// The versions this build reads.
///
/// Only version 2 has been verified against real files; 1 is accepted because
/// the format's documentation uses it for the same layout.
pub const SUPPORTED_VERSIONS: &[i64] = &[1, 2];
/// Largest table or modification count accepted, so a bad count fails early.
const MAX_COUNT: i32 = 1 << 20;
/// Trailing bytes each modification carries; the game ignores them.
const MOD_TAIL_LEN: usize = 4;

/// The binary type of a modification's value.
///
/// Re-exported from `war3-meta` rather than defined here: the metadata layer
/// needs the same four codes to say what it *expects*, and two enums for one
/// on-disk field is how they drift apart.
pub use war3_meta::FieldType;

/// A modification's value.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    /// An integer value.
    Integer(i32),
    /// A real value.
    Real(f32),
    /// An unreal value, nominally within `0.0 ..= 1.0`.
    Unreal(f32),
    /// A string value, as stored: `TRIGSTR_` references are not resolved here.
    String(String),
}

impl std::fmt::Display for FieldValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Integer(v) => write!(f, "{v}"),
            Self::Real(v) => write!(f, "{v}"),
            Self::Unreal(v) => write!(f, "{v}"),
            Self::String(v) => write!(f, "{v}"),
        }
    }
}

/// One field change on an object.
#[derive(Debug, Clone, PartialEq)]
pub struct Modification {
    /// The field's four-character id, e.g. `unam` for a unit's name.
    pub field: FourCC,
    /// The value and the type it was stored as.
    pub value: FieldValue,
    /// Level and data indicator, for the object kinds that carry them.
    ///
    /// `level` starts at 1 — or is 0 for a value that applies to every level —
    /// and the data indicator selects which of the `DataA..DataI` columns a
    /// value belongs to.
    ///
    /// Which kinds carry the block is a property of the kind, not of this field:
    /// see [`is_levelled`]. A `None` here on a levelled kind is a hand-built
    /// model, not a file that omitted the block.
    pub level: Option<Levelled>,
    /// The four bytes after the value, which the game ignores.
    ///
    /// They are kept so that a write can reproduce the input. Every file seen
    /// fills them with zeros, but "the game ignores it" is not "it is absent": a
    /// file that puts something else there must not have it quietly zeroed.
    pub tail: [u8; MOD_TAIL_LEN],
}

/// The level block of a levelled modification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levelled {
    /// Which level the value applies to; 0 means "all levels".
    pub level: i32,
    /// Which data column the value belongs to.
    pub data_indicator: i32,
}

/// One object: either a modified Blizzard object or a new one.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    /// For a custom object this is its own id; for a modified original it is
    /// the Blizzard object's id.
    pub id: FourCC,
    /// The object this one inherits from.
    ///
    /// Zero for the original table, whose entries *are* the base objects.
    pub base_id: FourCC,
    /// The field changes, in file order.
    pub modifications: Vec<Modification>,
}

impl Object {
    /// Whether this is an object the map created rather than modified.
    #[must_use]
    pub const fn is_custom(&self) -> bool {
        self.id.0[0] != 0 && !self.base_id.is_zero()
    }

    /// Whether the game treats this object as a hero.
    ///
    /// The uppercase-first-letter test only means anything for units: a custom
    /// item or ability id is uppercase by convention too, and calling those
    /// heroes would be nonsense.
    #[must_use]
    pub fn is_hero(&self, kind: ObjectKind) -> bool {
        kind == ObjectKind::Unit && ObjectKind::is_hero_id(self.id)
    }
}

/// The two tables an object file holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectTable {
    /// Blizzard objects the map modified. Their `base_id` is zero: they are the
    /// base objects.
    pub original: Vec<Object>,
    /// Objects the map created, each naming its parent.
    pub custom: Vec<Object>,
}

impl ObjectTable {
    /// Every object, originals first.
    pub fn all(&self) -> impl Iterator<Item = &Object> {
        self.original.iter().chain(self.custom.iter())
    }

    /// How many objects there are in total.
    #[must_use]
    pub fn len(&self) -> usize {
        self.original.len() + self.custom.len()
    }

    /// Whether the file held no objects at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The object an id names, searching customs first.
    ///
    /// A custom object shadows any original with the same id, which is how the
    /// game resolves them.
    #[must_use]
    pub fn get(&self, id: FourCC) -> Option<&Object> {
        self.custom
            .iter()
            .find(|o| o.id == id)
            .or_else(|| self.original.iter().find(|o| o.id == id))
    }

    /// Resolves one object's modifications into a flat list.
    ///
    /// Inherited values are not copied in: the chain is followed so the caller
    /// can see both, base first, with the object's own values last so a later
    /// entry for the same field wins.
    #[must_use]
    pub fn resolve<'a>(&'a self, object: &'a Object) -> Vec<(&'a Object, &'a Modification)> {
        let mut chain = Vec::new();
        let mut seen = Vec::new();
        let mut current = Some(object);
        while let Some(obj) = current {
            if seen.contains(&obj.id) {
                // A cycle in the inheritance chain: stop rather than loop.
                break;
            }
            seen.push(obj.id);
            chain.push(obj);
            current = self.get(obj.base_id);
        }
        chain.reverse();
        chain
            .into_iter()
            .flat_map(|o| o.modifications.iter().map(move |m| (o, m)))
            .collect()
    }

    /// Adds a diagnostic for every modification the metadata does not explain.
    ///
    /// `lookup` answers "is this field id known, and what type should it be".
    /// Returning `Some(None)` means known but untyped.
    pub fn diagnose_against<F>(&self, mut lookup: F, diagnostics: &mut Diagnostics)
    where
        F: FnMut(FourCC) -> Option<Option<FieldType>>,
    {
        for object in self.all() {
            for m in &object.modifications {
                match lookup(m.field) {
                    None => diagnostics.push(Diagnostic::warn(
                        DiagnosticCode::ObjectUnknownField,
                        format!(
                            "{}: field {:?} is not in the metadata table; its value is kept as \
                             parsed but cannot be named",
                            object.id, m.field
                        ),
                    )),
                    Some(Some(expected)) => {
                        let actual = value_type(&m.value);
                        if !codes_agree(expected, actual) {
                            diagnostics.push(Diagnostic::warn(
                                DiagnosticCode::ObjectTypeMismatch,
                                format!(
                                    "{}: field {:?} is stored as {actual:?} but the metadata says \
                                     {expected:?}",
                                    object.id, m.field
                                ),
                            ));
                        }
                    }
                    Some(None) => {}
                }
            }
        }
    }
}

/// The type a parsed value was stored as.
#[must_use]
pub const fn value_type(value: &FieldValue) -> FieldType {
    match value {
        FieldValue::Integer(_) => FieldType::Integer,
        FieldValue::Real(_) => FieldType::Real,
        FieldValue::Unreal(_) => FieldType::Unreal,
        FieldValue::String(_) => FieldType::String,
    }
}

/// Whether a stored type satisfies what the metadata expects.
///
/// # Why `int` and `unreal` are not a mismatch
///
/// Both occupy four bytes and both hold a number; they differ in how those four
/// bytes are read. The metadata's `type` word describes the **editor widget**,
/// and for a handful of fields that widget is an integer while the underlying
/// SLK column is a real.
///
/// Measured: across 117 object files and 322,107 modifications, every single
/// disagreement between the vocabulary-derived code and the stored code is of
/// this one shape — `udef` (Defense), `udup` (Defense Upgrade Amount) and the
/// item `Idef` are declared `int` and stored as `unreal`, 2,041 times. Treating
/// that as damage would mean warning on correct maps.
///
/// Anything else — a number where a string is promised, a string where a number
/// is promised — is a real disagreement and is reported.
#[must_use]
pub const fn codes_agree(expected: FieldType, actual: FieldType) -> bool {
    use FieldType::{Integer, Unreal};
    match (expected, actual) {
        // Four bytes holding a number, read two ways: a widget difference, not
        // damage.
        (Integer | Unreal, Integer | Unreal) => true,
        _ => expected.code() == actual.code(),
    }
}

/// A parsed object file.
#[derive(Debug, Clone)]
pub struct ObjectFile {
    /// Which of the seven categories this file holds.
    pub kind: ObjectKind,
    /// File version; 2 in every file seen.
    pub version: i32,
    /// The two tables.
    pub table: ObjectTable,
    /// How many empty tables followed the custom table.
    ///
    /// The format allows the `count` plus entries block to repeat and real files
    /// carry a third one, always with a count of zero. An empty table has no
    /// entries to keep, so the only thing worth recording is that it was there:
    /// [`Self::to_bytes`] writes this many zero words, which is what keeps a
    /// write from normalising a three-block file to the documented two.
    pub extra_tables: usize,
    /// Bytes after the last table that are not an empty table.
    ///
    /// Reported as [`DiagnosticCode::ObjectTrailingBytes`] **and kept**, because
    /// failing to place a byte is not a licence to drop it: a file this build
    /// only half understands must not come back quietly shortened.
    pub trailing: Vec<u8>,
    /// Diagnostics collected while parsing.
    pub diagnostics: Diagnostics,
}

/// Whether the object kind's modifications carry a level block.
///
/// The rule is hardcoded in the game and in every implementation: only
/// abilities, doodads and upgrades qualify.
#[must_use]
pub const fn is_levelled(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Ability | ObjectKind::Doodad | ObjectKind::Upgrade
    )
}

impl ObjectFile {
    /// Parses one object file.
    pub fn parse(kind: ObjectKind, bytes: &[u8]) -> Result<Self, ParseError> {
        let mut reader = Reader::new(bytes);
        let version = reader.i32()?;
        if !SUPPORTED_VERSIONS.contains(&i64::from(version)) {
            return Err(ParseError::UnsupportedVersion {
                format: kind.map_file(),
                found: i64::from(version),
                supported: SUPPORTED_VERSIONS,
            });
        }
        let levelled = is_levelled(kind);
        let original = reader.table("original", levelled, false)?;
        let custom = reader.table("custom", levelled, true)?;

        let mut diagnostics = Diagnostics::new();

        // The format allows a varying number of table blocks. A trailing run of
        // zero words is that many empty tables: a zero count with no entries
        // carries no data, so nothing can be lost by accepting it. Anything
        // else is reported below.
        let left = reader.remaining();
        let tail = reader.take(left)?;
        let mut extra_tables = 0usize;
        // Bytes the format does not place. Kept verbatim rather than dropped,
        // since the report below only counts them.
        let mut trailing = Vec::new();
        if !tail.is_empty() && tail.len() % 4 == 0 && tail.iter().all(|&b| b == 0) {
            extra_tables = tail.len() / 4;
        } else {
            // Not a table block. Put it back so the report names the offset.
            reader.seek(reader.len() - tail.len());
            trailing = tail.to_vec();
        }
        if extra_tables > 0 {
            diagnostics.push(Diagnostic::info(
                DiagnosticCode::ObjectExtraTable,
                format!(
                    "{extra_tables} empty table(s) follow the custom table; the format allows a \
                     varying number of tables and these hold no entries"
                ),
            ));
        }

        if reader.remaining() != 0 {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::ObjectTrailingBytes,
                format!(
                    "{} bytes follow the last table and are not an empty table; they are not part \
                     of the format as documented, so the parse may be incomplete",
                    reader.remaining()
                ),
            ));
        }
        Ok(Self {
            kind,
            version,
            table: ObjectTable { original, custom },
            extra_tables,
            trailing,
            diagnostics,
        })
    }

    /// Serialises back to the object file's bytes.
    ///
    /// # Why this mirrors `parse` line for line
    ///
    /// The hard metric is that a member survives `extract → build` byte for
    /// byte, so this has to reproduce the *input* rather than a tidied version of
    /// it: the same version, the same ids in the same order, the same counts, and
    /// the bytes the format leaves to the writer written back as they were read
    /// ([`Modification::tail`], [`Self::extra_tables`], [`Self::trailing`]).
    ///
    /// # The level block follows the kind, not the value
    ///
    /// `parse` reads `level` and `data_indicator` for abilities, doodads and
    /// upgrades and for nothing else — that is the rule the layout is built on,
    /// not a per-record flag. The write is therefore gated on
    /// [`is_levelled`]`(self.kind)` rather than on [`Modification::level`] being
    /// `Some`, so a model built by hand writes the layout its kind promises,
    /// which is the one the game will read.
    ///
    /// # About the `unwrap_or` fallback
    ///
    /// It only fires for a model built by hand: a `Modification` that came from
    /// `parse` of a levelled kind always carries its block. A model that would
    /// serialise differently from its source shows up as a byte difference, which
    /// is what the round-trip tests and `war3 map extract`'s self-check look for.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let levelled = is_levelled(self.kind);
        let mut out = Vec::new();
        push_i32(&mut out, self.version);
        push_i32(&mut out, len(&self.table.original));
        for object in &self.table.original {
            push_object(&mut out, object, levelled, false);
        }
        push_i32(&mut out, len(&self.table.custom));
        for object in &self.table.custom {
            push_object(&mut out, object, levelled, true);
        }
        // One zero word per extra empty table: the block the format allows to
        // repeat, written as many times as the input carried it.
        for _ in 0..self.extra_tables {
            push_i32(&mut out, 0);
        }
        out.extend_from_slice(&self.trailing);
        out
    }
}

/// Writes one object: its two ids, its modification count and its modifications.
///
/// The tables disagree about which id comes first — the original table stores
/// `(id, zero)` and the custom table `(parent, new id)` — and `parse` folds both
/// into `id` and `base_id`. This is that fold undone.
fn push_object(out: &mut Vec<u8>, object: &Object, levelled: bool, custom: bool) {
    let (first, second) = if custom {
        (object.base_id, object.id)
    } else {
        (object.id, object.base_id)
    };
    out.extend_from_slice(&first.to_bytes());
    out.extend_from_slice(&second.to_bytes());
    push_i32(out, len(&object.modifications));
    for modification in &object.modifications {
        push_modification(out, modification, levelled);
    }
}

/// Writes one modification: field id, type code, level block, value, tail.
fn push_modification(out: &mut Vec<u8>, modification: &Modification, levelled: bool) {
    out.extend_from_slice(&modification.field.to_bytes());
    // `value_type` inverts the `FieldType::from_code` dispatch `parse` uses to
    // decide how many bytes to read, so the code written is the code read.
    push_i32(out, value_type(&modification.value).code());
    if levelled {
        let block = modification.level.unwrap_or(Levelled {
            level: 0,
            data_indicator: 0,
        });
        push_i32(out, block.level);
        push_i32(out, block.data_indicator);
    }
    match &modification.value {
        FieldValue::Integer(v) => push_i32(out, *v),
        FieldValue::Real(v) | FieldValue::Unreal(v) => push_f32(out, *v),
        FieldValue::String(s) => push_cstr(out, s),
    }
    out.extend_from_slice(&modification.tail);
}

/// `len` as the `i32` the format uses for count words.
fn len<T>(list: &[T]) -> i32 {
    i32::try_from(list.len()).unwrap_or(i32::MAX)
}

fn push_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Writes a C string: the bytes, then the terminator.
///
/// `parse` refuses a string that is not UTF-8, so what was decoded is exactly
/// what the file held and this cannot differ from it.
fn push_cstr(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
}

/// A minimal cursor that reports offsets, since the interesting failures here
/// are positional.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// A `&mut self` method cannot be `const` on the 1.75 MSRV, which is why
    /// this is a plain `fn` while the accessors above are not.
    fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ParseError> {
        let end = self.pos.checked_add(len).ok_or(ParseError::UnexpectedEof {
            offset: self.pos,
            needed: len,
            available: self.remaining(),
        })?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(ParseError::UnexpectedEof {
                offset: self.pos,
                needed: len,
                available: self.remaining(),
            })?;
        self.pos = end;
        Ok(slice)
    }

    fn i32(&mut self) -> Result<i32, ParseError> {
        let raw = self.take(4)?;
        Ok(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }

    fn f32(&mut self) -> Result<f32, ParseError> {
        let raw = self.take(4)?;
        Ok(f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }

    fn fourcc(&mut self) -> Result<FourCC, ParseError> {
        let raw = self.take(4)?;
        Ok(FourCC([raw[0], raw[1], raw[2], raw[3]]))
    }

    fn cstr(&mut self) -> Result<String, ParseError> {
        let rest = self
            .bytes
            .get(self.pos..)
            .ok_or(ParseError::UnexpectedEof {
                offset: self.pos,
                needed: 1,
                available: 0,
            })?;
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(ParseError::BadString { offset: self.pos })?;
        let text = String::from_utf8(rest[..end].to_vec())
            .map_err(|_| ParseError::BadString { offset: self.pos })?;
        self.pos += end + 1;
        Ok(text)
    }

    /// Reads one of the two tables.
    fn table(
        &mut self,
        which: &'static str,
        levelled: bool,
        custom: bool,
    ) -> Result<Vec<Object>, ParseError> {
        let count_at = self.pos;
        let count = self.i32()?;
        if !(0..=MAX_COUNT).contains(&count) {
            return Err(ParseError::BadField {
                field: "object table count",
                reason: format!(
                    "the {which} table at offset {count_at} declares {count} objects, which is \
                     not a plausible count; the file is either not this format or damaged"
                ),
            });
        }
        let mut objects = Vec::with_capacity(count as usize);
        for index in 0..count {
            let start = self.pos;
            let id = self.fourcc()?;
            let second = self.fourcc()?;
            // The original table's second id is zero by design; the custom
            // table's is the new object's id, and its first id is the parent.
            let (id, base_id) = if custom { (second, id) } else { (id, second) };
            if id.is_zero() {
                return Err(ParseError::BadField {
                    field: "object id",
                    reason: format!("{which} object {index} at offset {start} has an all-zero id"),
                });
            }
            let mods = self.modifications(levelled, &id)?;
            objects.push(Object {
                id,
                base_id,
                modifications: mods,
            });
        }
        Ok(objects)
    }

    /// Reads one object's modifications.
    fn modifications(
        &mut self,
        levelled: bool,
        object: &FourCC,
    ) -> Result<Vec<Modification>, ParseError> {
        let count_at = self.pos;
        let count = self.i32()?;
        if !(0..=MAX_COUNT).contains(&count) {
            return Err(ParseError::BadField {
                field: "modification count",
                reason: format!(
                    "object {object} at offset {count_at} declares {count} modifications, which \
                     is not plausible"
                ),
            });
        }
        let mut mods = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let start = self.pos;
            let field = self.fourcc()?;
            let code = self.i32()?;
            let level = if levelled {
                Some(Levelled {
                    level: self.i32()?,
                    data_indicator: self.i32()?,
                })
            } else {
                None
            };
            let value = match FieldType::from_code(code) {
                FieldType::Integer => FieldValue::Integer(self.i32()?),
                FieldType::Real => FieldValue::Real(self.f32()?),
                FieldType::Unreal => FieldValue::Unreal(self.f32()?),
                FieldType::String => FieldValue::String(self.cstr()?),
                FieldType::Unknown(code) => {
                    return Err(ParseError::BadField {
                        field: "modification type",
                        reason: format!(
                            "object {object}, field {field:?} at offset {start} has type code \
                             {code}; only 0, 1, 2 and 3 are known"
                        ),
                    });
                }
            };
            let tail = self.take(MOD_TAIL_LEN)?;
            let mut kept_tail = [0u8; MOD_TAIL_LEN];
            kept_tail.copy_from_slice(tail);
            mods.push(Modification {
                field,
                value,
                level,
                tail: kept_tail,
            });
        }
        Ok(mods)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mod_int(field: &[u8; 4], value: i32, levelled: Option<(i32, i32)>) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(field);
        b.extend_from_slice(&0i32.to_le_bytes());
        if let Some((level, data)) = levelled {
            b.extend_from_slice(&level.to_le_bytes());
            b.extend_from_slice(&data.to_le_bytes());
        }
        b.extend_from_slice(&value.to_le_bytes());
        b.extend_from_slice(&[0u8; MOD_TAIL_LEN]);
        b
    }

    fn mod_str(field: &[u8; 4], value: &str) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(field);
        b.extend_from_slice(&3i32.to_le_bytes());
        b.extend_from_slice(value.as_bytes());
        b.push(0);
        b.extend_from_slice(&[0u8; MOD_TAIL_LEN]);
        b
    }

    /// A modification with an explicit stored type code and raw value bytes.
    fn mod_raw(field: &[u8; 4], code: i32, value: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(field);
        b.extend_from_slice(&code.to_le_bytes());
        b.extend_from_slice(value);
        b.extend_from_slice(&[0u8; MOD_TAIL_LEN]);
        b
    }

    fn object(id: &[u8; 4], second: &[u8; 4], mods: &[Vec<u8>]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(id);
        b.extend_from_slice(second);
        b.extend_from_slice(&(mods.len() as i32).to_le_bytes());
        for m in mods {
            b.extend_from_slice(m);
        }
        b
    }

    fn file(version: i32, original: &[Vec<u8>], custom: &[Vec<u8>]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&version.to_le_bytes());
        b.extend_from_slice(&(original.len() as i32).to_le_bytes());
        for o in original {
            b.extend_from_slice(o);
        }
        b.extend_from_slice(&(custom.len() as i32).to_le_bytes());
        for o in custom {
            b.extend_from_slice(o);
        }
        b
    }

    /// Asserts that a fixture comes back out of `to_bytes` exactly as it went in.
    ///
    /// Every fixture built in this module is fed through this, because a
    /// serialiser is only as good as the bytes it reproduces: an assertion on the
    /// model alone would pass just as happily on a write that drops a table, a
    /// level block or a tail.
    fn assert_round_trips(kind: ObjectKind, bytes: &[u8]) {
        let parsed = ObjectFile::parse(kind, bytes)
            .unwrap_or_else(|e| panic!("the fixture has to parse before it can be written: {e}"));
        assert_eq!(
            parsed.to_bytes(),
            bytes,
            "the fixture has to serialise back to the bytes it was parsed from"
        );
    }

    #[test]
    fn parses_a_units_file() {
        let bytes = file(
            2,
            &[object(b"Hmkg", b"\0\0\0\0", &[mod_str(b"uabi", "AInv")])],
            &[object(b"hpea", b"h000", &[mod_int(b"uhpm", 420, None)])],
        );
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.table.original.len(), 1);
        assert_eq!(parsed.table.custom.len(), 1);
        let original = &parsed.table.original[0];
        assert_eq!(original.id.to_bytes(), *b"Hmkg");
        assert!(original.base_id.is_zero());
        assert!(!original.is_custom());
        let custom = &parsed.table.custom[0];
        assert_eq!(custom.id.to_bytes(), *b"h000");
        assert_eq!(custom.base_id.to_bytes(), *b"hpea");
        assert!(custom.is_custom());
        assert_eq!(custom.modifications[0].value, FieldValue::Integer(420));
        assert_eq!(custom.modifications[0].level, None);
        assert!(parsed.diagnostics.is_empty());
    }

    #[test]
    fn a_levelled_kind_carries_the_level_block() {
        let bytes = file(
            2,
            &[],
            &[object(
                b"AIlf",
                b"A001",
                &[
                    mod_int(b"Ilif", 100, Some((1, 0))),
                    mod_int(b"Ilif", 200, Some((2, 0))),
                ],
            )],
        );
        let parsed = ObjectFile::parse(ObjectKind::Ability, &bytes).unwrap();
        assert_round_trips(ObjectKind::Ability, &bytes);
        let custom = &parsed.table.custom[0];
        assert_eq!(
            custom.modifications[0].level,
            Some(Levelled {
                level: 1,
                data_indicator: 0
            })
        );
        assert_eq!(custom.modifications[1].level.unwrap().level, 2);
    }

    #[test]
    fn the_levelled_rule_follows_the_kind() {
        assert!(is_levelled(ObjectKind::Ability));
        assert!(is_levelled(ObjectKind::Doodad));
        assert!(is_levelled(ObjectKind::Upgrade));
        assert!(!is_levelled(ObjectKind::Unit));
        assert!(!is_levelled(ObjectKind::Item));
        assert!(!is_levelled(ObjectKind::Destructable));
        assert!(!is_levelled(ObjectKind::Buff));
    }

    #[test]
    fn the_wrong_levelled_setting_desynchronises_and_is_reported() {
        // A units file read as levelled: the level block eats value bytes and
        // the walk runs off the end. It must be an error, not a short list.
        let bytes = file(
            2,
            &[],
            &[object(b"hpea", b"h000", &[mod_int(b"uhpm", 420, None)])],
        );
        let err = ObjectFile::parse(ObjectKind::Ability, &bytes).unwrap_err();
        assert!(
            matches!(
                err,
                ParseError::BadField { .. } | ParseError::UnexpectedEof { .. }
            ),
            "{err}"
        );
        // The same bytes as a unit file are fine.
        assert!(ObjectFile::parse(ObjectKind::Unit, &bytes).is_ok());
        assert_round_trips(ObjectKind::Unit, &bytes);
    }

    #[test]
    fn an_unknown_type_code_is_reported_with_its_offset() {
        let mut m = mod_int(b"uhpm", 1, None);
        m[4..8].copy_from_slice(&7i32.to_le_bytes());
        let bytes = file(2, &[], &[object(b"hpea", b"h000", &[m])]);
        let err = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap_err();
        assert!(
            matches!(&err, ParseError::BadField { reason, .. } if reason.contains("type code 7")),
            "{err}"
        );
    }

    #[test]
    fn an_implausible_count_is_rejected() {
        let mut bytes = file(2, &[], &[]);
        bytes[0..4].copy_from_slice(&2i32.to_le_bytes());
        bytes[4..8].copy_from_slice(&99_999_999i32.to_le_bytes());
        let err = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap_err();
        assert!(
            matches!(&err, ParseError::BadField { field, .. } if field.contains("table count")),
            "{err}"
        );
    }

    #[test]
    fn an_all_zero_object_id_is_rejected() {
        let bytes = file(2, &[], &[object(b"hpea", b"\0\0\0\0", &[])]);
        let err = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap_err();
        assert!(matches!(err, ParseError::BadField { .. }), "{err}");

        // The original table's zero second id is not the object's id, so the
        // same shape is fine there.
        let bytes = file(2, &[object(b"hpea", b"\0\0\0\0", &[])], &[]);
        assert!(ObjectFile::parse(ObjectKind::Unit, &bytes).is_ok());
        assert_round_trips(ObjectKind::Unit, &bytes);
    }

    #[test]
    fn inheritance_resolves_base_first_and_the_object_last() {
        let bytes = file(
            2,
            &[object(b"hfoo", b"\0\0\0\0", &[mod_int(b"uhpm", 420, None)])],
            &[object(
                b"hfoo",
                b"h001",
                &[mod_int(b"uhpm", 500, None), mod_str(b"unam", "Mine")],
            )],
        );
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        let custom = parsed.table.get(FourCC(*b"h001")).unwrap();
        let resolved = parsed.table.resolve(custom);
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].0.id.to_bytes(), *b"hfoo");
        assert_eq!(resolved[0].1.value, FieldValue::Integer(420));
        assert_eq!(resolved[2].1.value, FieldValue::String("Mine".to_string()));
    }

    #[test]
    fn a_custom_object_shadows_an_original_with_the_same_id() {
        let bytes = file(
            2,
            &[object(b"h000", b"\0\0\0\0", &[mod_int(b"uhpm", 1, None)])],
            &[object(b"hfoo", b"h000", &[mod_int(b"uhpm", 2, None)])],
        );
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        let found = parsed.table.get(FourCC(*b"h000")).unwrap();
        assert!(found.is_custom(), "the custom entry must win");
    }

    #[test]
    fn only_units_can_be_heroes() {
        let files = [
            (ObjectKind::Unit, b"U000"),
            (ObjectKind::Item, b"I000"),
            (ObjectKind::Ability, b"A001"),
        ];
        for (kind, id) in files {
            // The custom table stores (parent, new id) in that order.
            let bytes = file(2, &[], &[object(b"hfoo", id, &[])]);
            let parsed = ObjectFile::parse(kind, &bytes).unwrap();
            assert_round_trips(kind, &bytes);
            let object = &parsed.table.custom[0];
            assert_eq!(object.id.to_bytes(), *id, "{kind:?}");
            assert_eq!(
                object.is_hero(kind),
                kind == ObjectKind::Unit,
                "{kind:?} {id:?}"
            );
        }
    }

    #[test]
    fn an_unknown_version_is_named() {
        let bytes = file(3, &[], &[]);
        assert!(matches!(
            ObjectFile::parse(ObjectKind::Unit, &bytes),
            Err(ParseError::UnsupportedVersion { found: 3, .. })
        ));
    }

    #[test]
    fn unknown_fields_and_type_mismatches_are_diagnosed() {
        let bytes = file(
            2,
            &[],
            &[object(
                b"hpea",
                b"h000",
                &[mod_int(b"zzzz", 1, None), mod_str(b"uhpm", "text")],
            )],
        );
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        let mut diagnostics = Diagnostics::new();
        parsed.table.diagnose_against(
            |id| match &id.0 {
                b"zzzz" => None,
                b"uhpm" => Some(Some(FieldType::Integer)),
                _ => Some(None),
            },
            &mut diagnostics,
        );
        let codes: Vec<_> = diagnostics.items().iter().map(|d| d.code).collect();
        assert!(codes.contains(&DiagnosticCode::ObjectUnknownField));
        assert!(codes.contains(&DiagnosticCode::ObjectTypeMismatch));
    }

    #[test]
    fn a_number_where_a_string_is_declared_is_a_mismatch() {
        let bytes = file(
            2,
            &[],
            &[object(
                b"hpea",
                b"h000",
                &[mod_raw(b"unam", 0, &7i32.to_le_bytes())],
            )],
        );
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        let mut diagnostics = Diagnostics::new();
        parsed
            .table
            .diagnose_against(|_| Some(Some(FieldType::String)), &mut diagnostics);
        assert!(diagnostics
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::ObjectTypeMismatch));
    }

    #[test]
    fn an_integer_where_unreal_is_declared_is_not_a_mismatch() {
        // Measured: `udef`/`udup`/`Idef` are declared `int` in the metadata and
        // stored as `unreal` in 2,041 modifiers across the corpus. Warning on
        // that would mean warning on correct maps.
        assert!(codes_agree(FieldType::Integer, FieldType::Unreal));
        assert!(codes_agree(FieldType::Unreal, FieldType::Integer));
        assert!(codes_agree(FieldType::Integer, FieldType::Integer));

        let bytes = file(
            2,
            &[],
            &[object(
                b"hpea",
                b"h000",
                &[mod_raw(b"udef", 2, &2.5f32.to_le_bytes())],
            )],
        );
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        let mut diagnostics = Diagnostics::new();
        parsed
            .table
            .diagnose_against(|_| Some(Some(FieldType::Integer)), &mut diagnostics);
        assert!(
            diagnostics.is_empty(),
            "int versus unreal is a widget difference, not damage: {:?}",
            diagnostics.items()
        );
    }

    #[test]
    fn a_real_is_not_tolerated_against_a_string() {
        assert!(!codes_agree(FieldType::Real, FieldType::String));
        assert!(!codes_agree(FieldType::Real, FieldType::Unreal));
        assert!(!codes_agree(FieldType::Unknown(9), FieldType::Integer));
    }

    #[test]
    fn trailing_bytes_are_reported() {
        let mut bytes = file(2, &[], &[]);
        bytes.extend_from_slice(&[0xAB; 4]);
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.trailing, vec![0xAB; 4]);
        assert!(parsed
            .diagnostics
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::ObjectTrailingBytes));
    }

    #[test]
    fn an_extra_empty_table_is_accepted_and_not_a_warning() {
        // Four maps in the corpus end with one more `int32`, always zero: a
        // third table with no entries. It is valid, so it must not be reported
        // as unexplained data.
        let mut bytes = file(2, &[], &[]);
        bytes.extend_from_slice(&0i32.to_le_bytes());

        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        assert!(parsed.table.is_empty());
        assert_eq!(
            parsed.extra_tables, 1,
            "the third block is recorded, not assumed"
        );
        let codes: Vec<_> = parsed.diagnostics.items().iter().map(|d| d.code).collect();
        assert!(
            !codes.contains(&DiagnosticCode::ObjectTrailingBytes),
            "an empty extra table is part of the format: {codes:?}"
        );
        assert!(codes.contains(&DiagnosticCode::ObjectExtraTable));
        assert!(!parsed.diagnostics.has_problems(), "info is not a problem");
    }

    #[test]
    fn two_extra_empty_tables_are_both_counted() {
        let mut bytes = file(2, &[], &[]);
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        assert_eq!(parsed.extra_tables, 2);
        let info = parsed
            .diagnostics
            .items()
            .iter()
            .find(|d| d.code == DiagnosticCode::ObjectExtraTable)
            .expect("the extra tables are reported");
        assert!(info.message.contains('2'), "{}", info.message);
    }

    #[test]
    fn an_extra_table_that_is_not_empty_is_still_a_warning() {
        // A non-zero count means entries this build cannot place, so the bytes
        // must be reported rather than swallowed as an empty table.
        let mut bytes = file(2, &[], &[]);
        bytes.extend_from_slice(&1i32.to_le_bytes());
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_round_trips(ObjectKind::Unit, &bytes);
        assert_eq!(
            parsed.trailing,
            vec![1, 0, 0, 0],
            "a block that is not empty is kept, not swallowed as an empty table"
        );
        let codes: Vec<_> = parsed.diagnostics.items().iter().map(|d| d.code).collect();
        assert!(
            codes.contains(&DiagnosticCode::ObjectTrailingBytes),
            "{codes:?}"
        );
        assert!(!codes.contains(&DiagnosticCode::ObjectExtraTable));
    }

    #[test]
    fn a_level_zero_modification_means_all_levels() {
        let bytes = file(
            2,
            &[],
            &[object(
                b"AIlf",
                b"A001",
                &[mod_int(b"anam", 7, Some((0, 0)))],
            )],
        );
        let parsed = ObjectFile::parse(ObjectKind::Ability, &bytes).unwrap();
        assert_round_trips(ObjectKind::Ability, &bytes);
        assert_eq!(
            parsed.table.custom[0].modifications[0].level.unwrap().level,
            0
        );
    }

    #[test]
    fn a_modification_tail_that_is_not_zero_is_kept() {
        // The four bytes after each value are ignored by the game, but ignored is
        // not absent: a file that carries something else there has to come back
        // unchanged rather than with its tail zeroed.
        let mut m = mod_int(b"uhpm", 420, None);
        let tail = m.len() - MOD_TAIL_LEN;
        m[tail..].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let bytes = file(2, &[], &[object(b"hpea", b"h000", &[m])]);
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_eq!(
            parsed.table.custom[0].modifications[0].tail,
            [0xDE, 0xAD, 0xBE, 0xEF]
        );
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn an_original_object_whose_second_id_is_not_zero_is_kept() {
        // The original table's second id is zero by design, but a file that put
        // something else in that word must not come back with it zeroed: it is
        // stored as `base_id` and written back in the same position.
        let bytes = file(2, &[object(b"hpea", b"XXXX", &[])], &[]);
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_eq!(parsed.table.original[0].base_id.to_bytes(), *b"XXXX");
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn the_version_is_written_back_as_it_was_read() {
        // Version 1 shares this layout, so a version 1 file parses — and writing
        // a constant 2 would upgrade a file that was not this build's to upgrade.
        let bytes = file(
            1,
            &[],
            &[object(b"hpea", b"h000", &[mod_int(b"uhpm", 420, None)])],
        );
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn a_trailing_run_that_is_not_a_whole_number_of_words_is_kept() {
        // Five zero bytes are neither an empty table nor nothing: the empty-table
        // test is applied to the whole run, and a run that fails it is kept.
        let mut bytes = file(2, &[], &[]);
        bytes.extend_from_slice(&[0u8; 5]);
        let parsed = ObjectFile::parse(ObjectKind::Unit, &bytes).unwrap();
        assert_eq!(parsed.extra_tables, 0);
        assert_eq!(parsed.trailing, vec![0u8; 5]);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn a_levelled_kind_writes_the_level_block_even_when_the_model_omits_it() {
        // Which layout a file has is a property of its kind, which is why
        // `to_bytes` asks the kind rather than the modification. A caller that
        // builds an ability without a level would otherwise write bytes the game
        // reads as a units file.
        let model = ObjectFile {
            kind: ObjectKind::Ability,
            version: 2,
            table: ObjectTable {
                original: Vec::new(),
                custom: vec![Object {
                    id: FourCC(*b"A001"),
                    base_id: FourCC(*b"AIlf"),
                    modifications: vec![Modification {
                        field: FourCC(*b"Ilif"),
                        value: FieldValue::Integer(100),
                        level: None,
                        tail: [0; MOD_TAIL_LEN],
                    }],
                }],
            },
            extra_tables: 0,
            trailing: Vec::new(),
            diagnostics: Diagnostics::new(),
        };
        let expected = file(
            2,
            &[],
            &[object(
                b"AIlf",
                b"A001",
                &[mod_int(b"Ilif", 100, Some((0, 0)))],
            )],
        );
        assert_eq!(model.to_bytes(), expected);
    }
}

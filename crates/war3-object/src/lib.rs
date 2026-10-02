//! Warcraft III object data.
//!
//! A map's object files record the changes the author made in the Object
//! Editor: modifications to Blizzard's own objects, and the objects they
//! created. All seven categories share one binary layout, differing only in
//! which file they live in and whether their modifications carry a level.
//!
//! ```text
//! ObjectKind::Unit        war3map.w3u
//! ObjectKind::Item        war3map.w3t
//! ObjectKind::Destructable war3map.w3b
//! ObjectKind::Doodad      war3map.w3d
//! ObjectKind::Ability     war3map.w3a
//! ObjectKind::Buff        war3map.w3h
//! ObjectKind::Upgrade     war3map.w3q
//! ```
//!
//! The format is documented in `docs/reference/YDWE调研结论.md` §2, and the
//! layout here was additionally verified against real files by walking both
//! tables and requiring the cursor to land exactly on the end of the file:
//! `war3map.w3u` (2 original + 5 custom), `war3map.w3t` (0 + 1) and
//! `war3map.w3a` (6 + 4) from YDWE's sample map all close exactly, and each
//! accepts only the one "levelled" setting the rules below predict.
//!
//! Object data on its own is just field ids and raw values; what it *means*
//! comes from Blizzard's `*MetaData.slk` tables, which live in [`war3_meta`].
//! This crate deliberately does not require them: parsing is independent of
//! metadata, and naming is a separate step.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod object;

pub use object::{
    codes_agree, is_levelled, FieldType, FieldValue, Levelled, Modification, Object, ObjectFile,
    ObjectTable, SUPPORTED_VERSIONS,
};
pub use war3_meta::ObjectKind;

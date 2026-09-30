//! Object field metadata and trigger definitions.
//!
//! # Two layers
//!
//! | Layer | Content | Handling |
//! | --- | --- | --- |
//! | mechanical | field identifiers, type codes, `index`/`repeat`/`data`, ranges, `WESTRING_*` **keys** | prebuilt, shipped with the repository |
//! | expressive | the actual text behind those keys | read at runtime from the user's installation, never shipped |
//!
//! The boundary is a file boundary already: `displayname` in `metadata.ini` holds
//! a key such as `WESTRING_AEVAL_AARE`, and the text lives in
//! `UI\WorldEditStrings.txt`.
//!
//! When adding metadata to this repository, add keys and structure only, never
//! display text.
//!
//! # Where the data comes from
//!
//! Field metadata comes from the game's `*MetaData.slk` files, which are SYLK
//! rather than tab-separated text. Trigger definitions come from
//! `UI\TriggerData.txt` and `UI\TriggerStrings.txt`, which exist only in the
//! user's installation.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod field;
pub mod object_meta;
pub mod trigger;

pub use field::{has_level_in_binary, FieldMeta, FieldType, ObjectKind};
pub use object_meta::{MetaTable, MetaTableSet};
pub use trigger::{TriggerArg, TriggerData, TriggerFunction, TriggerParam, TriggerType};

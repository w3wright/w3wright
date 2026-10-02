//! Object field metadata and trigger definitions.
//!
//! # Two layers
//!
//! | Layer | Content | Handling |
//! | --- | --- | --- |
//! | mechanical | field identifiers, type codes, `index`/`repeat`/`data`, ranges, `WESTRING_*` **keys** | read at runtime from the user's installation |
//! | expressive | the actual text behind those keys | read at runtime from the user's installation, never shipped |
//!
//! Nothing from either layer ships with this repository: there is no prebuilt
//! metadata table and no Blizzard-derived data file in the tree. A user without
//! the game installed still parses and edits maps, but sees FourCCs where field
//! names would be. Whether the mechanical layer should instead be prebuilt and
//! shipped is an open design question (ADR-0011, Q19 in the `docs/` set).
//!
//! The boundary is a file boundary already: the `displayName` column of a
//! `*MetaData.slk` holds a key such as `WESTRING_UEVAL_UNAM`, and the text lives
//! in `UI\WorldEditStrings.txt`.
//!
//! # Where the data comes from
//!
//! Field metadata comes from the game's `*MetaData.slk` files, which are SYLK
//! rather than tab-separated text. Trigger definitions come from
//! `UI\TriggerData.txt` and `UI\TriggerStrings.txt`, which exist only in the
//! user's installation.
//!
//! The `type` column of a metadata file holds a **word**, not a binary code;
//! `UI\UnitEditorData.txt` is the registry that turns one into the other, and
//! [`TypeRegistry`] derives it. See [`editordata`] for why it is derived rather
//! than listed.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod editordata;
pub mod field;
pub mod object_meta;
pub mod trigger;

pub use editordata::{EditorData, TypeRegistry, EDITOR_DATA_PATH};
pub use field::{has_level_in_binary, FieldMeta, FieldType, ObjectKind};
pub use object_meta::{MetaTable, MetaTableSet};
pub use trigger::{TriggerArg, TriggerData, TriggerFunction, TriggerParam, TriggerType};

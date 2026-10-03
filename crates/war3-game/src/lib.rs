//! The user's Warcraft III installation: its data, and the names of the objects in it.
//!
//! # What this crate is for
//!
//! Answering "what is `hfoo` called" without the caller having to know any of the following, all
//! of which are true and none of which is guessable:
//!
//! - The game's data is **inside MPQ archives**, not on disk. A directory scan reports almost
//!   everything missing on a normal installation.
//! - Which archive wins matters. `war3.mpq` holds RoC-era metadata and `War3Patch.mpq` overrides
//!   it; reading them in the wrong order silently serves 2003.
//! - Names live in **three different places** depending on the kind of object, and two of the
//!   three need a second lookup through `UI\WorldEditStrings.txt`.
//! - A map can override any of it, and the override is the name that matters.
//!
//! # Caching, and where it belongs
//!
//! Two levels, deliberately:
//!
//! 1. **[`cached::Cached`]** wraps any [`war3_core::AssetSource`] and remembers each file it has been
//!    asked for. This is for the caller that resolves names while a map is open: reading a 400 KB
//!    `.slk` out of an MPQ means locating, decompressing and parsing it, and a lookup that repeats
//!    per table row would pay for that per row.
//! 2. **The caller keeps one [`GameAssets`] per game directory.** Opening it reads the hash and
//!    block tables of four archives — 17,660 enumerable names on this machine — and that is the
//!    expensive part, not the file reads.
//!
//! ⚠️ **This crate holds no cache of its own.** A library with a global mutable cache is a library
//! that cannot be used twice, cannot be tested and cannot be reasoned about; the caller knows when
//! the game directory changed and this crate does not. See [`resolver::Resolver`] for the parsed
//! result, which is what a caller should keep for the life of one open map.
//!
//! # Nothing Blizzard's ships here
//!
//! No table, no string, no prebuilt index. Everything is read from the installation at runtime, so
//! a user without the game installed still parses and edits maps — they see IDs where names would
//! be, which is also what they see for an object the game has never heard of. Whether the
//! mechanical layer *should* be shipped prebuilt is [ADR-0011]'s open question, and this crate
//! takes the same position the rest of the workspace does.
//!
//! [ADR-0011]: https://github.com/w3wright/docs

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod cached;
pub mod fields;
pub mod names;
pub mod resolver;
pub mod strings;
pub mod sylk;

#[cfg(feature = "mpq")]
pub mod mpq;

pub use cached::Cached;
pub use fields::{list_element_kind, name_field, value_shape, FieldFact, FieldFacts, ValueShape};
pub use names::{NameStats, Names};
pub use resolver::{NameSource, Resolved, Resolver};
pub use strings::{parse_sections, WorldStrings};
pub use sylk::{SylkRow, SylkTable};

#[cfg(feature = "mpq")]
pub use mpq::{GameAssets, LayeredSource, MpqAssetSource, ARCHIVE_LOAD_ORDER};

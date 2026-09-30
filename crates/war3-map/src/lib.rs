//! Map composition and the metadata-class file formats.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod cursor;
pub mod imports;
pub mod map;
pub mod w3i;
pub mod wts;

pub use cursor::Cursor;
pub use imports::{Import, ImportFlags, ImportList};
pub use map::{Map, MapFileEntry, MapSource, MemoryMapSource};
pub use w3i::{MapFlags, MapInfo, Player, Tileset, Upgrade, UpgradeState};
pub use wts::StringTable;

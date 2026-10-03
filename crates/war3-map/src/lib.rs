//! Map composition and the metadata-class file formats.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

mod bytes;
pub mod cursor;
pub mod doodads;
pub mod imports;
pub mod map;
pub mod summary;
pub mod text;
pub mod units;
pub mod w3i;
pub mod wts;

pub use cursor::Cursor;
pub use doodads::{Doodad, DoodadFile, SpecialDoodad};
pub use imports::{Import, ImportFlags, ImportList};
pub use map::{is_map_file_name, Map, MapFileEntry, MapSource, MemoryMapSource};
pub use summary::{CountEntry, DoodadSummary, Histogram, UnitSummary};
pub use text::{plain, spans, Markup};
pub use units::{
    AbilityModification, DropSet, HeroAttributes, InventoryItem, ItemDrop, RandomChoice,
    RandomPayload, Unit, UnitFile,
};
pub use w3i::{MapFlags, MapInfo, Player, Tileset, Upgrade, UpgradeState};
pub use wts::StringTable;
// ⚠️ Exported so a caller can ask **whether** a value is a reference without owning a table to resolve
// it against. That is exactly the case in the editor, where a field's value arrives before the map it
// belongs to and a reference has to be recognised rather than resolved.
pub use wts::parse_trigstr;

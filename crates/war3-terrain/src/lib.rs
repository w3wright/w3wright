//! Terrain (`.w3e`) parsing and serialisation, plus the SLK reader it depends on.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod slk;
pub mod w3e;

pub use slk::{parse_sylk, SylkTable};
pub use w3e::{
    Terrain, TerrainTile, TileFlags, Tileset, Variation, W3eVersion, WaterAndFlags,
    HEADER_FIXED_SIZE, RECORD_SIZE_V11, RECORD_SIZE_V12, WORLD_UNITS_PER_TILE,
};

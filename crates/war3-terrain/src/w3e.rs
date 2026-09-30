//! Terrain (`.w3e`): header, tile-point records, and the two on-disk layouts.
//!
//! # Two layouts, and why the version field is not enough
//!
//! | Game version | `.w3e` version | Record | Ground texture bits | Addressable textures |
//! | --- | --- | --- | --- | --- |
//! | RoC 1.0x → TFT 1.2x → 1.31 → Reforged 1.32.x | **11** | 7 bytes | 4 | 16 |
//! | WC3 **2.0.3+** | **12** | 8 bytes | 6 | 64 |
//!
//! Nothing changed across RoC, TFT, 1.31 or Reforged 1.32; only 2.0.3 changed
//! the layout. Reading a v12 file with a v11 parser corrupts it silently, so the
//! record size is derived from the file geometry and cross-checked against the
//! version field rather than trusted from it.
//!
//! # Other rules that are easy to get wrong
//!
//! - **Read the water word as `u16`, then mask.** Reading it as `i16` and masking
//!   afterwards hits sign extension and yields a wrong water level.
//! - **Preserve bits whose meaning is unknown.** Bit 15 of the water word and bits
//!   2-7 of the v12 second flag byte have no documented meaning, so they are kept
//!   verbatim across a round trip.
//! - **Report, never drop.** Unknown bits, out-of-range texture indices and
//!   version disagreements all produce a diagnostic.

use std::fmt;

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};
use war3_core::{Error, FourCC, ParseError, Result};

use crate::slk::WaterTable;

/// `.w3e` magic bytes.
pub const MAGIC: [u8; 4] = *b"W3E!";
/// Fixed part of the header, excluding the two texture lists.
///
/// Total header size is `HEADER_FIXED_SIZE + 4a + 4b`.
pub const HEADER_FIXED_SIZE: usize = 37;
/// Size of a v11 tile-point record.
pub const RECORD_SIZE_V11: usize = 7;
/// Size of a v12 tile-point record.
pub const RECORD_SIZE_V12: usize = 8;

/// World units per tile.
pub const WORLD_UNITS_PER_TILE: f32 = 128.0;

/// Zero offset of the stored ground height (`0x2000`).
///
/// This is an offset, not a divisor.
pub const GROUND_HEIGHT_ZERO: i32 = 0x2000;
/// Divisor converting a stored height to tiles.
pub const HEIGHT_DIVISOR_TILE: f32 = 512.0;
/// Divisor converting a stored height to world units.
pub const HEIGHT_DIVISOR_WORLD: f32 = 4.0;

/// World units per cliff layer.
pub const LAYER_HEIGHT_STEP_WORLD: f32 = 128.0;
/// Layer height that represents zero.
pub const LAYER_HEIGHT_ZERO: i32 = 2;

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// On-disk layout version of a `.w3e` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum W3eVersion {
    /// Version 11: 7-byte records, 4-bit ground texture index (16 textures).
    ///
    /// Covers RoC 1.0x through Reforged 1.32.x.
    V11,
    /// Version 12: 8-byte records, 6-bit ground texture index (64 textures).
    ///
    /// Introduced by WC3 2.0.3.
    V12,
}

impl W3eVersion {
    /// The version number stored in the file.
    #[must_use]
    pub const fn number(self) -> i32 {
        match self {
            Self::V11 => 11,
            Self::V12 => 12,
        }
    }

    /// Record size in bytes.
    #[must_use]
    pub const fn record_size(self) -> usize {
        match self {
            Self::V11 => RECORD_SIZE_V11,
            Self::V12 => RECORD_SIZE_V12,
        }
    }

    /// Width of the ground texture field, in bits.
    #[must_use]
    pub const fn texture_bits(self) -> u32 {
        match self {
            Self::V11 => 4,
            Self::V12 => 6,
        }
    }

    /// How many ground textures this version can address.
    #[must_use]
    pub const fn max_textures(self) -> u32 {
        1 << self.texture_bits()
    }

    /// Parses a version number.
    #[must_use]
    pub const fn from_number(n: i32) -> Option<Self> {
        match n {
            11 => Some(Self::V11),
            12 => Some(Self::V12),
            _ => None,
        }
    }

    /// Reverses the record size into a version, for the geometry check.
    #[must_use]
    pub const fn from_record_size(size: usize) -> Option<Self> {
        match size {
            RECORD_SIZE_V11 => Some(Self::V11),
            RECORD_SIZE_V12 => Some(Self::V12),
            _ => None,
        }
    }
}

impl fmt::Display for W3eVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.number())
    }
}

// ---------------------------------------------------------------------------
// Bit fields
// ---------------------------------------------------------------------------

/// The water word at `+2`: a `u16` carrying a level and two flags.
///
/// Must be read as unsigned and then masked; reading as `i16` first hits sign
/// extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WaterAndFlags(pub u16);

impl WaterAndFlags {
    /// Water level mask: 14 bits, not 15.
    pub const WATER_MASK: u16 = 0x3FFF;
    /// Map edge, as marked by the World Editor's boundary tool.
    pub const BOUNDARY_1: u16 = 0x4000;
    /// `0x8000`, whose meaning is not documented anywhere.
    ///
    /// Kept verbatim so that a round trip does not lose it.
    pub const UNKNOWN: u16 = 0x8000;

    /// The raw water level, 14 bits.
    #[must_use]
    pub const fn water_raw(self) -> u16 {
        self.0 & Self::WATER_MASK
    }

    /// Whether the map edge flag is set.
    #[must_use]
    pub const fn is_boundary(self) -> bool {
        self.0 & Self::BOUNDARY_1 != 0
    }

    /// Whether the undocumented bit is set.
    #[must_use]
    pub const fn has_unknown_bit(self) -> bool {
        self.0 & Self::UNKNOWN != 0
    }

    /// Water level in world units.
    ///
    /// `water_zero_offset` comes from the `height` column of `Water.slk`
    /// multiplied by 128, and varies by tileset. The frequently quoted `-89.6`
    /// corresponds to a `height` of `-0.7` and is not universal.
    #[must_use]
    pub fn water_world_height(self, water_zero_offset: f32) -> f32 {
        (f32::from(self.water_raw()) - GROUND_HEIGHT_ZERO as f32) / HEIGHT_DIVISOR_WORLD
            - water_zero_offset
    }
}

/// The variation byte, whose layout differs between versions only in position.
///
/// # A three-way disagreement in the sources
///
/// | Source | Ground | Cliff | Widths |
/// | --- | --- | --- | --- |
/// | Two rendering implementations | `v & 0x1F` | `(v & 0xE0) >> 5` | 5 / 3 |
/// | A community specification | `byte & 0xf8` | `byte & 0x7` | 5 / 3, bit order inverted |
/// | A .NET library's accessors | `& 0x0F` | `(& 0xF0) >> 4` | **4 / 4** |
///
/// This implementation uses **5 / 3 with ground in the low bits**. The 5-bit
/// width is forced independently: the ground variation's sentinel value is 16,
/// which does not fit in 4 bits, ruling out the 4/4 split. Both implementations
/// that actually draw terrain read ground from the low bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Variation(pub u8);

impl Variation {
    /// Ground variation mask, low 5 bits.
    pub const GROUND_MASK: u8 = 0x1F;
    /// Cliff variation mask, high 3 bits.
    pub const CLIFF_MASK: u8 = 0xE0;
    /// The sentinel ground variation; its value is why the field needs 5 bits.
    pub const GROUND_SENTINEL: u8 = 16;

    /// Ground variation, 0 to 31.
    #[must_use]
    pub const fn ground(self) -> u8 {
        self.0 & Self::GROUND_MASK
    }

    /// Cliff variation, 0 to 7.
    #[must_use]
    pub const fn cliff(self) -> u8 {
        (self.0 & Self::CLIFF_MASK) >> 5
    }

    /// Packs the two components.
    #[must_use]
    pub const fn from_parts(ground: u8, cliff: u8) -> Self {
        Self((ground & Self::GROUND_MASK) | ((cliff << 5) & Self::CLIFF_MASK))
    }
}

/// Per-tile-point flags.
///
/// The same logical flags sit at **different bit positions** in v11 and v12:
///
/// | Flag | v11 `+4` | v12 `+4` | v12 `+5` |
/// | --- | --- | --- | --- |
/// | ramp | `0x10` | `0x40` | — |
/// | blight | `0x20` | `0x80` | — |
/// | water | `0x40` | — | `0x01` |
/// | boundary | `0x80` | — | `0x02` |
///
/// Mask constants must not be shared between the two versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TileFlags {
    /// This point ramps between two layers.
    pub ramp: bool,
    /// Ground renders as undead blight.
    pub blight: bool,
    /// The water plane is enabled at this point.
    pub water: bool,
    /// Camera boundary area.
    pub boundary: bool,
}

impl TileFlags {
    // ---- v11 masks ----
    /// v11 ramp bit.
    pub const V11_RAMP: u8 = 0x10;
    /// v11 blight bit.
    pub const V11_BLIGHT: u8 = 0x20;
    /// v11 water bit.
    pub const V11_WATER: u8 = 0x40;
    /// v11 boundary bit.
    pub const V11_BOUNDARY: u8 = 0x80;

    // ---- v12 masks ----
    /// v12 `+4` ramp bit.
    pub const V12_RAMP: u8 = 0x40;
    /// v12 `+4` blight bit.
    pub const V12_BLIGHT: u8 = 0x80;
    /// v12 `+5` water bit.
    pub const V12_WATER: u8 = 0x01;
    /// v12 `+5` boundary bit.
    pub const V12_BOUNDARY: u8 = 0x02;

    /// Decodes the v11 `+4` byte.
    #[must_use]
    pub const fn from_v11_byte(b: u8) -> Self {
        Self {
            ramp: b & Self::V11_RAMP != 0,
            blight: b & Self::V11_BLIGHT != 0,
            water: b & Self::V11_WATER != 0,
            boundary: b & Self::V11_BOUNDARY != 0,
        }
    }

    /// Decodes the v12 `+4` and `+5` bytes, also returning the six reserved bits.
    #[must_use]
    pub const fn from_v12_bytes(b4: u8, b5: u8) -> (Self, u8) {
        (
            Self {
                ramp: b4 & Self::V12_RAMP != 0,
                blight: b4 & Self::V12_BLIGHT != 0,
                water: b5 & Self::V12_WATER != 0,
                boundary: b5 & Self::V12_BOUNDARY != 0,
            },
            b5 & 0xFC,
        )
    }

    /// Encodes the v11 `+4` byte.
    #[must_use]
    pub const fn to_v11_byte(self) -> u8 {
        let mut b = 0u8;
        if self.ramp {
            b |= Self::V11_RAMP;
        }
        if self.blight {
            b |= Self::V11_BLIGHT;
        }
        if self.water {
            b |= Self::V11_WATER;
        }
        if self.boundary {
            b |= Self::V11_BOUNDARY;
        }
        b
    }

    /// Encodes the v12 `+4` byte, excluding the texture index.
    #[must_use]
    pub const fn to_v12_byte4(self) -> u8 {
        let mut b = 0u8;
        if self.ramp {
            b |= Self::V12_RAMP;
        }
        if self.blight {
            b |= Self::V12_BLIGHT;
        }
        b
    }

    /// Encodes the v12 `+5` byte, preserving the reserved bits.
    #[must_use]
    pub const fn to_v12_byte5(self, unknown_bits: u8) -> u8 {
        let mut b = unknown_bits & 0xFC;
        if self.water {
            b |= Self::V12_WATER;
        }
        if self.boundary {
            b |= Self::V12_BOUNDARY;
        }
        b
    }
}

// ---------------------------------------------------------------------------
// Tile points
// ---------------------------------------------------------------------------

/// One terrain tile point: a corner shared by up to four tiles.
///
/// Raw bytes are stored rather than decoded fields, so bits with no known
/// meaning survive a round trip untouched.
///
/// # Byte meaning per version
///
/// | Field | v11 | v12 |
/// | --- | --- | --- |
/// | `byte4` | `+4`: texture in the low 4 bits, four flags above | `+4`: texture in the low 6 bits, ramp and blight above |
/// | `byte5` | `+5`: **the variation byte** | `+5`: **flag byte 2** |
/// | `byte6` | `+6`: layer height and cliff texture | `+6`: **the variation byte** |
/// | `byte7` | unused, always 0 | `+7`: layer height and cliff texture |
///
/// `byte5` therefore carries different meanings in the two versions, which is
/// why every accessor takes a version and why cross-version conversion goes
/// through [`TerrainTile::to_bytes_for`] with an explicit source version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerrainTile {
    /// Ground height at `+0`, signed.
    pub ground_height: i16,
    /// Water word at `+2`.
    pub water_and_flags: WaterAndFlags,
    /// The `+4` byte.
    pub byte4: u8,
    /// The `+5` byte.
    pub byte5: u8,
    /// The `+6` byte.
    pub byte6: u8,
    /// The `+7` byte; only v12 uses it.
    pub byte7: u8,
}

impl TerrainTile {
    /// The stored ground height.
    #[must_use]
    pub const fn ground_height_raw(self) -> i16 {
        self.ground_height
    }

    /// The byte that carries layer height and cliff texture.
    #[must_use]
    pub const fn layer_height_byte(self, version: W3eVersion) -> u8 {
        match version {
            W3eVersion::V11 => self.byte6,
            W3eVersion::V12 => self.byte7,
        }
    }

    /// Layer height, 0 to 15.
    #[must_use]
    pub const fn layer_height(self, version: W3eVersion) -> u8 {
        self.layer_height_byte(version) & 0x0F
    }

    /// Cliff texture index, 0 to 15. The value 15 is reserved.
    #[must_use]
    pub const fn cliff_texture(self, version: W3eVersion) -> u8 {
        self.layer_height_byte(version) >> 4
    }

    /// Ground texture index.
    ///
    /// v11 addresses only the first 16 textures even when the file's texture
    /// list is longer; v12 addresses 64.
    #[must_use]
    pub const fn ground_texture(self, version: W3eVersion) -> u8 {
        self.byte4 & ((1u8 << version.texture_bits()) - 1)
    }

    /// The variation byte.
    #[must_use]
    pub const fn variation(self, version: W3eVersion) -> Variation {
        match version {
            W3eVersion::V11 => Variation(self.byte5),
            W3eVersion::V12 => Variation(self.byte6),
        }
    }

    /// The decoded flags.
    #[must_use]
    pub const fn flags(self, version: W3eVersion) -> TileFlags {
        match version {
            W3eVersion::V11 => TileFlags::from_v11_byte(self.byte4),
            W3eVersion::V12 => TileFlags::from_v12_bytes(self.byte4, self.byte5).0,
        }
    }

    /// The six reserved bits of the v12 second flag byte; zero for v11.
    #[must_use]
    pub const fn v12_unknown_flag_bits(self, version: W3eVersion) -> u8 {
        match version {
            W3eVersion::V11 => 0,
            W3eVersion::V12 => TileFlags::from_v12_bytes(self.byte4, self.byte5).1,
        }
    }

    /// Height in world units, including the cliff layer.
    #[must_use]
    pub fn world_height(self, version: W3eVersion) -> f32 {
        (f32::from(self.ground_height) - GROUND_HEIGHT_ZERO as f32) / HEIGHT_DIVISOR_WORLD
            + (f32::from(self.layer_height(version)) - LAYER_HEIGHT_ZERO as f32)
                * LAYER_HEIGHT_STEP_WORLD
    }

    /// Height in tiles, including the cliff layer.
    #[must_use]
    pub fn tile_height(self, version: W3eVersion) -> f32 {
        (f32::from(self.ground_height) - GROUND_HEIGHT_ZERO as f32) / HEIGHT_DIVISOR_TILE
            + (f32::from(self.layer_height(version)) - LAYER_HEIGHT_ZERO as f32)
    }

    /// Water level in world units.
    #[must_use]
    pub fn water_world_height(self, water_zero_offset: f32) -> f32 {
        self.water_and_flags.water_world_height(water_zero_offset)
    }

    /// Builds a v11-shaped tile point, mainly for tests and editing.
    #[must_use]
    pub const fn new(
        ground_height: i16,
        water_and_flags: WaterAndFlags,
        byte4: u8,
        byte5: u8,
        byte6: u8,
    ) -> Self {
        Self { ground_height, water_and_flags, byte4, byte5, byte6, byte7: 0 }
    }

    /// Builds a v12-shaped tile point.
    #[must_use]
    pub const fn new_v12(
        ground_height: i16,
        water_and_flags: WaterAndFlags,
        byte4: u8,
        byte5: u8,
        byte6: u8,
        byte7: u8,
    ) -> Self {
        Self { ground_height, water_and_flags, byte4, byte5, byte6, byte7 }
    }

    /// Decodes a record for a given version.
    #[must_use]
    pub fn from_bytes(version: W3eVersion, raw: &[u8]) -> Self {
        let ground_height = i16::from_le_bytes([raw[0], raw[1]]);
        let water_and_flags = WaterAndFlags(u16::from_le_bytes([raw[2], raw[3]]));
        match version {
            W3eVersion::V11 => Self {
                ground_height,
                water_and_flags,
                byte4: raw[4],
                byte5: raw[5],
                byte6: raw[6],
                byte7: 0,
            },
            W3eVersion::V12 => Self {
                ground_height,
                water_and_flags,
                byte4: raw[4],
                byte5: raw[5],
                byte6: raw[6],
                byte7: raw[7],
            },
        }
    }

    /// Encodes a record for `target`, interpreting the stored bytes as `source`.
    ///
    /// The source version is needed because `byte5` and `byte6` swap roles
    /// between the two layouts. Decoding to semantic values first and re-encoding
    /// afterwards is what makes v11 ↔ v12 conversion correct rather than
    /// coincidental.
    #[must_use]
    pub fn to_bytes_for(self, target: W3eVersion, source: W3eVersion) -> Vec<u8> {
        let variation = self.variation(source);
        let flags = self.flags(source);
        let unknown_v12 = self.v12_unknown_flag_bits(source);
        let layer_byte = self.layer_height_byte(source);

        let mut out = Vec::with_capacity(target.record_size());
        out.extend_from_slice(&self.ground_height.to_le_bytes());
        out.extend_from_slice(&self.water_and_flags.0.to_le_bytes());
        match target {
            W3eVersion::V11 => {
                // The texture is narrowed to 4 bits; `Terrain::write` has already
                // refused any conversion where that would lose information.
                out.push((self.ground_texture(source) & 0x0F) | flags.to_v11_byte());
                out.push(variation.0);
                out.push(layer_byte);
            }
            W3eVersion::V12 => {
                out.push((self.ground_texture(source) & 0x3F) | flags.to_v12_byte4());
                out.push(flags.to_v12_byte5(unknown_v12));
                out.push(variation.0);
                out.push(layer_byte);
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------

/// The terrain of one map.
#[derive(Debug, Clone, PartialEq)]
pub struct Terrain {
    /// On-disk layout version.
    pub version: W3eVersion,
    /// Main tileset letter.
    pub tileset: Tileset,
    /// Whether a custom or mixed tileset is in use.
    pub uses_custom_tileset: bool,
    /// Tile points along X, which is tiles plus one.
    pub width: u32,
    /// Tile points along Y, which is tiles plus one.
    pub height: u32,
    /// Centre offset along X, in world units.
    pub center_offset_x: f32,
    /// Centre offset along Y, in world units.
    pub center_offset_y: f32,
    /// Ground texture list, in file order.
    pub ground_textures: Vec<FourCC>,
    /// Cliff texture list, in file order.
    pub cliff_textures: Vec<FourCC>,
    /// Tile points, indexed as `y * width + x`, with `(0, 0)` at the south-west
    /// corner and +Y pointing north.
    pub tiles: Vec<TerrainTile>,
    /// Diagnostics from parsing.
    pub diagnostics: Diagnostics,
}

impl Terrain {
    /// Parses a `.w3e` file.
    ///
    /// # Determining the version
    ///
    /// The record size is derived from the geometry first and only then compared
    /// with the version field:
    ///
    /// 1. read the header to get `a`, `b`, `width` and `height`;
    /// 2. compute `data_len = len - 37 - 4a - 4b`;
    /// 3. require `data_len % (width * height) == 0` with a quotient of 7 or 8;
    /// 4. compare that against the version field and follow the geometry if they
    ///    disagree, reporting the disagreement.
    ///
    /// Step 3 is what keeps a v12 file from being parsed as v11 and corrupted
    /// without a word.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut diagnostics = Diagnostics::new();

        // ---- magic ----
        if bytes.len() < HEADER_FIXED_SIZE {
            return Err(Error::from(ParseError::UnexpectedEof {
                offset: 0,
                needed: HEADER_FIXED_SIZE,
                available: bytes.len(),
            }));
        }
        let magic: [u8; 4] = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if magic != MAGIC {
            return Err(Error::from(ParseError::BadMagic { expected: "W3E!", found: magic }));
        }

        // ---- header ----
        let declared_version = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let tileset = Tileset::from_byte(bytes[8]);
        let uses_custom_tileset =
            i32::from_le_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]) != 0;
        let ground_count =
            i32::from_le_bytes([bytes[13], bytes[14], bytes[15], bytes[16]]).max(0) as usize;

        // The two texture lists sit *between* the fixed fields:
        //
        //   magic(4) version(4) tileset(1) custom(4) a(4)
        //   ground[a*4]  b(4)  cliff[b*4]
        //   width(4) height(4) offsetX(4) offsetY(4)
        //
        // The total is `37 + 4a + 4b`, but reading has to follow that order:
        // `width` comes after the lists, so `a` must be known to find it.
        let mut cursor = 17usize;
        let ground_textures = read_fourcc_list(bytes, &mut cursor, ground_count)?;
        // `read_i32` rather than an unchecked conversion, so a truncated file
        // yields an error with a position instead of a panic.
        let cliff_count = read_i32(bytes, cursor)?.max(0) as usize;
        cursor += 4;
        let cliff_textures = read_fourcc_list(bytes, &mut cursor, cliff_count)?;

        let textures_end = cursor;
        // Four more fixed fields follow the lists, so the header ends 16 bytes
        // later. Forgetting those 16 bytes makes the computed data length 16 too
        // large and the geometry check fails.
        let header_end = textures_end + 16;

        let width = read_i32(bytes, textures_end)?;
        let height = read_i32(bytes, textures_end + 4)?;
        let center_offset_x = f32::from_bits(read_i32(bytes, textures_end + 8)? as u32);
        let center_offset_y = f32::from_bits(read_i32(bytes, textures_end + 12)? as u32);

        if width <= 0 || height <= 0 {
            return Err(Error::from(ParseError::Validation {
                check: "w3e.dimensions",
                detail: format!("tile point dimensions must be positive, got {width}x{height}"),
            }));
        }
        let width = width as u32;
        let height = height as u32;

        // ---- geometry check, which is what actually decides the version ----
        let point_count = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| {
                Error::from(ParseError::Validation {
                    check: "w3e.dimensions",
                    detail: format!("{width}x{height} tile points overflows"),
                })
            })?;
        let data_len = bytes.len().saturating_sub(header_end);

        if point_count == 0 {
            return Err(Error::from(ParseError::Validation {
                check: "w3e.dimensions",
                detail: "zero tile points".to_string(),
            }));
        }
        if data_len % point_count != 0 {
            return Err(Error::from(ParseError::Validation {
                check: "w3e.tile_record_size",
                detail: format!(
                    "data section of {data_len} bytes is not divisible by {width}x{height} = \
                     {point_count} tile points; header is {header_end} bytes ({ground_count} \
                     ground and {cliff_count} cliff textures) -- the file is truncated, or the \
                     texture counts are wrong"
                ),
            }));
        }
        let record_size = data_len / point_count;
        let geometric_version = W3eVersion::from_record_size(record_size).ok_or_else(|| {
            Error::from(ParseError::Validation {
                check: "w3e.tile_record_size",
                detail: format!(
                    "geometry implies {record_size}-byte records, which is neither 7 (v11) nor \
                     8 (v12) -- the file is damaged, or a layout is missing from this build"
                ),
            })
        })?;

        // ---- cross-check against the version field ----
        let version = match W3eVersion::from_number(declared_version) {
            Some(declared) if declared == geometric_version => declared,
            Some(declared) => {
                // This is the case where trusting the version field alone would
                // silently corrupt a v12 file. The geometry wins and the
                // disagreement is reported.
                diagnostics.push(Diagnostic::warn(
                    DiagnosticCode::W3eRecordSizeMismatch,
                    format!(
                        "version field says v{} ({} bytes per record) but the geometry implies \
                         {record_size} bytes -- following the geometry (v{}); this is how a v12 \
                         file gets silently corrupted by a v11-only parser",
                        declared.number(),
                        declared.record_size(),
                        geometric_version.number()
                    ),
                ));
                geometric_version
            }
            None => {
                diagnostics.push(Diagnostic::warn(
                    DiagnosticCode::W3eRecordSizeMismatch,
                    format!(
                        "version field {declared_version} is outside the known range {{11, 12}} but \
                         the geometry implies {record_size}-byte records -- following the geometry \
                         (v{})",
                        geometric_version.number()
                    ),
                ));
                geometric_version
            }
        };

        // ---- tile points ----
        let mut tiles = Vec::with_capacity(point_count);
        let texture_limit = version.max_textures();
        let mut out_of_range_textures = 0usize;
        let mut unknown_water_bits = 0usize;
        let mut unknown_flag_bits = 0usize;

        for index in 0..point_count {
            let start = header_end + index * record_size;
            let raw = &bytes[start..start + record_size];
            let tile = TerrainTile::from_bytes(version, raw);
            if u32::from(tile.ground_texture(version)) >= texture_limit {
                out_of_range_textures += 1;
            }
            if tile.water_and_flags.has_unknown_bit() {
                unknown_water_bits += 1;
            }
            if tile.v12_unknown_flag_bits(version) != 0 {
                unknown_flag_bits += 1;
            }
            tiles.push(tile);
        }

        if out_of_range_textures > 0 {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::W3eTextureIndexOutOfRange,
                format!(
                    "{out_of_range_textures} tile points reference a ground texture at or above \
                     {texture_limit}, the addressable limit for {version}; preserved as is rather \
                     than normalised"
                ),
            ));
        }
        if unknown_water_bits > 0 {
            diagnostics.push(Diagnostic::info(
                DiagnosticCode::W3eUnknownWaterBit,
                format!(
                    "{unknown_water_bits} tile points have bit 0x8000 set in the water word; its \
                     meaning is unknown, so it is preserved for a lossless round trip"
                ),
            ));
        }
        if unknown_flag_bits > 0 {
            diagnostics.push(Diagnostic::info(
                DiagnosticCode::W3eUnknownFlagBits,
                format!(
                    "{unknown_flag_bits} tile points have reserved bits set in the v12 second flag \
                     byte; preserved as is"
                ),
            ));
        }

        Ok(Self {
            version,
            tileset,
            uses_custom_tileset,
            width,
            height,
            center_offset_x,
            center_offset_y,
            ground_textures,
            cliff_textures,
            tiles,
            diagnostics,
        })
    }

    /// Width in tiles, which is the number of tile points minus one.
    #[must_use]
    pub const fn tile_width(&self) -> u32 {
        self.width.saturating_sub(1)
    }

    /// Height in tiles.
    #[must_use]
    pub const fn tile_height(&self) -> u32 {
        self.height.saturating_sub(1)
    }

    /// A tile point by coordinate, with `(0, 0)` at the south-west corner.
    #[must_use]
    pub fn tile_at(&self, x: u32, y: u32) -> Option<&TerrainTile> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.tiles.get((y * self.width + x) as usize)
    }

    /// World position of the south-west corner.
    ///
    /// A 128x128-tile map spans -8192 to +8192.
    #[must_use]
    pub fn world_origin(&self) -> (f32, f32) {
        (
            -(self.tile_width() as f32) * WORLD_UNITS_PER_TILE / 2.0,
            -(self.tile_height() as f32) * WORLD_UNITS_PER_TILE / 2.0,
        )
    }

    /// World position of a tile point, excluding height.
    #[must_use]
    pub fn world_position(&self, x: u32, y: u32) -> (f32, f32) {
        let (ox, oy) = self.world_origin();
        (
            ox + x as f32 * WORLD_UNITS_PER_TILE,
            oy + y as f32 * WORLD_UNITS_PER_TILE,
        )
    }

    /// The range of world heights, for framing a viewport.
    #[must_use]
    pub fn world_height_range(&self) -> (f32, f32) {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for tile in &self.tiles {
            let h = tile.world_height(self.version);
            if h < min {
                min = h;
            }
            if h > max {
                max = h;
            }
        }
        if self.tiles.is_empty() {
            (0.0, 0.0)
        } else {
            (min, max)
        }
    }

    /// How many tile points carry each flag.
    #[must_use]
    pub fn flag_counts(&self) -> FlagCounts {
        let mut counts = FlagCounts::default();
        for tile in &self.tiles {
            let f = tile.flags(self.version);
            if f.ramp {
                counts.ramp += 1;
            }
            if f.blight {
                counts.blight += 1;
            }
            if f.water {
                counts.water += 1;
            }
            if f.boundary {
                counts.boundary += 1;
            }
            if tile.water_and_flags.is_boundary() {
                counts.boundary_1 += 1;
            }
        }
        counts
    }

    /// How many tile points have each layer height, indexed 0 to 15.
    #[must_use]
    pub fn layer_histogram(&self) -> [usize; 16] {
        let mut hist = [0usize; 16];
        for tile in &self.tiles {
            hist[tile.layer_height(self.version) as usize] += 1;
        }
        hist
    }

    /// Which ground texture indices are actually referenced.
    #[must_use]
    pub fn used_ground_textures(&self) -> Vec<u8> {
        let mut seen = [false; 64];
        for tile in &self.tiles {
            seen[tile.ground_texture(self.version) as usize & 0x3F] = true;
        }
        (0..64u8).filter(|&i| seen[i as usize]).collect()
    }

    /// Serialises back to `.w3e`.
    ///
    /// The target version is explicit rather than defaulting to the source,
    /// because downgrading from v12 to v11 discards the top two bits of the
    /// ground texture index and that should be a deliberate choice.
    ///
    /// If a downgrade would actually lose a texture index, this fails instead of
    /// truncating silently.
    pub fn write(&self, target: W3eVersion) -> Result<Vec<u8>> {
        if target == W3eVersion::V11 {
            let limit = W3eVersion::V11.max_textures();
            let bad = self
                .tiles
                .iter()
                .filter(|t| u32::from(t.ground_texture(self.version)) >= limit)
                .count();
            if bad > 0 {
                return Err(Error::from(ParseError::Validation {
                    check: "w3e.downgrade_texture_index",
                    detail: format!(
                        "downgrading to v11 would overflow the 4-bit ground texture index for {bad} \
                         tile points (v12 has 6 bits); refusing to truncate -- merge the texture \
                         list or stay on v12"
                    ),
                }));
            }
        }

        let mut out = Vec::with_capacity(
            HEADER_FIXED_SIZE
                + 4 * self.ground_textures.len()
                + 4 * self.cliff_textures.len()
                + self.tiles.len() * target.record_size(),
        );
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&target.number().to_le_bytes());
        out.push(self.tileset.to_byte());
        out.extend_from_slice(&i32::from(self.uses_custom_tileset).to_le_bytes());
        out.extend_from_slice(&(self.ground_textures.len() as i32).to_le_bytes());
        for id in &self.ground_textures {
            out.extend_from_slice(&id.to_bytes());
        }
        out.extend_from_slice(&(self.cliff_textures.len() as i32).to_le_bytes());
        for id in &self.cliff_textures {
            out.extend_from_slice(&id.to_bytes());
        }
        out.extend_from_slice(&(self.width as i32).to_le_bytes());
        out.extend_from_slice(&(self.height as i32).to_le_bytes());
        out.extend_from_slice(&self.center_offset_x.to_bits().to_le_bytes());
        out.extend_from_slice(&self.center_offset_y.to_bits().to_le_bytes());

        for tile in &self.tiles {
            out.extend_from_slice(&tile.to_bytes_for(target, self.version));
        }
        Ok(out)
    }

    /// The water zero offset in world units, from `Water.slk`.
    ///
    /// `None` means unknown; callers should not fall back to a constant.
    #[must_use]
    pub fn water_zero_offset(&self, water: &WaterTable) -> Option<f32> {
        water.height_for_tileset(self.tileset).map(|h| h * WORLD_UNITS_PER_TILE)
    }
}

/// How many tile points carry each flag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlagCounts {
    /// Ramp points.
    pub ramp: usize,
    /// Blighted points.
    pub blight: usize,
    /// Points with the water plane enabled.
    pub water: usize,
    /// Camera boundary points.
    pub boundary: usize,
    /// Map edge points, from the water word.
    pub boundary_1: usize,
}

/// Main tileset letter.
///
/// Deliberately a separate type from the one in the map-info module: this crate
/// does not depend on that one, and the caller converts between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tileset {
    /// `A`
    Ashenvale,
    /// `B`
    Barrens,
    /// `C`
    Felwood,
    /// `D`
    Dungeon,
    /// `F`
    LordaeronFall,
    /// `G`
    Underground,
    /// `I`
    Icecrown,
    /// `J`
    DalaranRuins,
    /// `K`
    BlackCitadel,
    /// `L`
    LordaeronSummer,
    /// `N`
    Northrend,
    /// `O`
    Outland,
    /// `Q`
    VillageFall,
    /// `V`
    Village,
    /// `W`
    LordaeronWinter,
    /// `X`
    Dalaran,
    /// `Y`
    Cityscape,
    /// `Z`
    SunkenRuins,
    /// A letter outside the known table, preserved as is.
    Unknown(u8),
}

impl Tileset {
    /// Decodes the stored byte, case-insensitively.
    #[must_use]
    pub const fn from_byte(b: u8) -> Self {
        match b.to_ascii_uppercase() {
            b'A' => Self::Ashenvale,
            b'B' => Self::Barrens,
            b'C' => Self::Felwood,
            b'D' => Self::Dungeon,
            b'F' => Self::LordaeronFall,
            b'G' => Self::Underground,
            b'I' => Self::Icecrown,
            b'J' => Self::DalaranRuins,
            b'K' => Self::BlackCitadel,
            b'L' => Self::LordaeronSummer,
            b'N' => Self::Northrend,
            b'O' => Self::Outland,
            b'Q' => Self::VillageFall,
            b'V' => Self::Village,
            b'W' => Self::LordaeronWinter,
            b'X' => Self::Dalaran,
            b'Y' => Self::Cityscape,
            b'Z' => Self::SunkenRuins,
            other => Self::Unknown(other),
        }
    }

    /// The stored byte.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Ashenvale => b'A',
            Self::Barrens => b'B',
            Self::Felwood => b'C',
            Self::Dungeon => b'D',
            Self::LordaeronFall => b'F',
            Self::Underground => b'G',
            Self::Icecrown => b'I',
            Self::DalaranRuins => b'J',
            Self::BlackCitadel => b'K',
            Self::LordaeronSummer => b'L',
            Self::Northrend => b'N',
            Self::Outland => b'O',
            Self::VillageFall => b'Q',
            Self::Village => b'V',
            Self::LordaeronWinter => b'W',
            Self::Dalaran => b'X',
            Self::Cityscape => b'Y',
            Self::SunkenRuins => b'Z',
            Self::Unknown(b) => b,
        }
    }

    /// English name, used for display.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ashenvale => "Ashenvale",
            Self::Barrens => "Barrens",
            Self::Felwood => "Felwood",
            Self::Dungeon => "Dungeon",
            Self::LordaeronFall => "Lordaeron Fall",
            Self::Underground => "Underground",
            Self::Icecrown => "Icecrown",
            Self::DalaranRuins => "Dalaran Ruins",
            Self::BlackCitadel => "Black Citadel",
            Self::LordaeronSummer => "Lordaeron Summer",
            Self::Northrend => "Northrend",
            Self::Outland => "Outland",
            Self::VillageFall => "Village Fall",
            Self::Village => "Village",
            Self::LordaeronWinter => "Lordaeron Winter",
            Self::Dalaran => "Dalaran",
            Self::Cityscape => "Cityscape",
            Self::SunkenRuins => "Sunken Ruins",
            Self::Unknown(_) => "Unknown",
        }
    }

    /// The letter used as a prefix in `Water.slk` row keys.
    #[must_use]
    pub const fn water_key(self) -> u8 {
        self.to_byte()
    }
}

impl fmt::Display for Tileset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(b) => write!(f, "Unknown({:?})", *b as char),
            other => write!(f, "{}", other.name()),
        }
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn read_exact(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    bytes.get(offset..offset + len).ok_or_else(|| {
        Error::from(ParseError::UnexpectedEof {
            offset,
            needed: len,
            available: bytes.len().saturating_sub(offset),
        })
    })
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32> {
    let raw = read_exact(bytes, offset, 4)?;
    Ok(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn read_fourcc_list(
    bytes: &[u8],
    cursor: &mut usize,
    count: usize,
) -> Result<Vec<war3_core::FourCC>> {
    let raw = read_exact(bytes, *cursor, count * 4)?;
    *cursor += count * 4;
    Ok(raw
        .chunks_exact(4)
        .map(|c| war3_core::FourCC([c[0], c[1], c[2], c[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Header length the test builder produces: `37 + 4a + 4b` with a=2, b=1.
    ///
    /// Written from the counts rather than as a literal so that changing the
    /// builder's texture lists updates it automatically.
    const GROUND_TEXTURE_COUNT: usize = 2;
    /// Cliff texture count the test builder writes.
    const CLIFF_TEXTURE_COUNT: usize = 1;

    /// Total header length the builder produces.
    const TEST_HEADER_LEN: usize =
        HEADER_FIXED_SIZE + 4 * GROUND_TEXTURE_COUNT + 4 * CLIFF_TEXTURE_COUNT;

    /// Offset of `width`, so tests can patch it without hard-coding 37.
    const WIDTH_OFFSET: usize =
        17 + 4 * GROUND_TEXTURE_COUNT + 4 + 4 * CLIFF_TEXTURE_COUNT;
    /// `height` follows `width`.
    const HEIGHT_OFFSET: usize = WIDTH_OFFSET + 4;

    /// Builds a terrain file of `width` x `height` tile points.
    fn build(version: W3eVersion, width: u32, height: u32, fill: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC);
        b.extend_from_slice(&version.number().to_le_bytes());
        b.push(b'L');
        b.extend_from_slice(&0i32.to_le_bytes()); // usesCustomTileset
        b.extend_from_slice(&2i32.to_le_bytes()); // a = 2
        b.extend_from_slice(b"Ldrt");
        b.extend_from_slice(b"Lgrs");
        b.extend_from_slice(&1i32.to_le_bytes()); // b = 1
        b.extend_from_slice(b"CLdi");
        assert_eq!(b.len(), WIDTH_OFFSET, "width must land here");
        b.extend_from_slice(&(width as i32).to_le_bytes());
        b.extend_from_slice(&(height as i32).to_le_bytes());
        b.extend_from_slice(&(-8128.0f32).to_bits().to_le_bytes());
        b.extend_from_slice(&(-8128.0f32).to_bits().to_le_bytes());
        assert_eq!(b.len(), TEST_HEADER_LEN, "header must be 37 + 4a + 4b");
        for _ in 0..(width * height) {
            b.extend_from_slice(&0x2000i16.to_le_bytes()); // ground height 0
            b.extend_from_slice(&0u16.to_le_bytes()); // water word
            b.push(fill); // +4
            b.push(0); // +5
            if version == W3eVersion::V12 {
                b.push(0); // +6 variation
            }
            b.push(0x22); // +6 / +7: layer height 2, cliff texture 0
        }
        b
    }

    #[test]
    fn parses_v11_header_and_tile_points() {
        let bytes = build(W3eVersion::V11, 3, 4, 0x01);
        let t = Terrain::parse(&bytes).unwrap();
        assert_eq!(t.version, W3eVersion::V11);
        assert_eq!(t.width, 3);
        assert_eq!(t.height, 4);
        assert_eq!(t.tiles.len(), 12);
        assert_eq!(t.tile_width(), 2);
        assert_eq!(t.tile_height(), 3);
        assert_eq!(t.ground_textures.len(), 2);
        assert_eq!(t.ground_textures[0].to_string(), "Ldrt");
        assert_eq!(t.cliff_textures[0].to_string(), "CLdi");
        assert_eq!(t.tileset, Tileset::LordaeronSummer);
    }

    #[test]
    fn header_fixed_size_matches_the_field_offsets() {
        // With a = b = 0 the header is 8 + 1 + 4 + 4 + 4 + 4 + 4 + 4 + 4 = 37.
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC);
        b.extend_from_slice(&11i32.to_le_bytes());
        b.push(b'L');
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&0f32.to_bits().to_le_bytes());
        b.extend_from_slice(&0f32.to_bits().to_le_bytes());
        assert_eq!(b.len(), HEADER_FIXED_SIZE);
    }

    #[test]
    fn parses_v12_with_eight_byte_records() {
        let bytes = build(W3eVersion::V12, 2, 2, 0x01);
        let t = Terrain::parse(&bytes).unwrap();
        assert_eq!(t.version, W3eVersion::V12);
        assert_eq!(t.tiles.len(), 4);
    }

    #[test]
    fn geometry_wins_over_a_wrong_version_field() {
        // A v12 layout whose version field claims v11. A v11-only parser would
        // corrupt it silently; this must follow the geometry and report it.
        let mut bytes = build(W3eVersion::V12, 4, 4, 0x01);
        bytes[4..8].copy_from_slice(&11i32.to_le_bytes());

        let t = Terrain::parse(&bytes).unwrap();
        assert_eq!(t.version, W3eVersion::V12, "geometry must decide the version");
        assert_eq!(t.tiles.len(), 16);
        assert!(
            t.diagnostics
                .items()
                .iter()
                .any(|d| d.code == DiagnosticCode::W3eRecordSizeMismatch),
            "the disagreement must be reported"
        );
    }

    #[test]
    fn truncated_file_is_rejected_with_the_arithmetic() {
        let mut bytes = build(W3eVersion::V11, 4, 4, 0);
        bytes.truncate(bytes.len() - 3);
        let err = Terrain::parse(&bytes).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("not divisible"), "{text}");
        assert!(text.contains("16 tile points"), "{text}");
    }

    #[test]
    fn impossible_record_size_is_rejected() {
        let mut bytes = build(W3eVersion::V11, 1, 1, 0);
        bytes.truncate(bytes.len() - RECORD_SIZE_V11);
        bytes.extend_from_slice(&[0u8; 9]);
        let err = Terrain::parse(&bytes).unwrap_err();
        assert!(err.to_string().contains("neither 7 (v11) nor 8 (v12)"), "{err}");
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut bytes = build(W3eVersion::V11, 2, 2, 0);
        bytes[0] = b'X';
        assert!(matches!(
            Terrain::parse(&bytes),
            Err(e) if e.to_string().contains("magic")
        ));
    }

    #[test]
    fn zero_dimensions_are_rejected() {
        let mut bytes = build(W3eVersion::V11, 2, 2, 0);
        bytes[WIDTH_OFFSET..WIDTH_OFFSET + 4].copy_from_slice(&0i32.to_le_bytes());
        assert_eq!(
            i32::from_le_bytes(bytes[WIDTH_OFFSET..WIDTH_OFFSET + 4].try_into().unwrap()),
            0,
            "the patch must land on width"
        );
        let err = Terrain::parse(&bytes).unwrap_err();
        assert!(err.to_string().contains("must be positive"), "{err}");
    }

    #[test]
    fn negative_dimensions_are_rejected() {
        let mut bytes = build(W3eVersion::V11, 2, 2, 0);
        bytes[HEIGHT_OFFSET..HEIGHT_OFFSET + 4].copy_from_slice(&(-5i32).to_le_bytes());
        assert!(Terrain::parse(&bytes).is_err());
    }

    #[test]
    fn zero_dimensions_do_not_panic_with_a_division_by_zero() {
        // The dimension check has to run before the modulo, which would divide by
        // the zero tile count.
        let mut bytes = build(W3eVersion::V11, 1, 1, 0);
        bytes[WIDTH_OFFSET..WIDTH_OFFSET + 4].copy_from_slice(&0i32.to_le_bytes());
        let result = std::panic::catch_unwind(|| Terrain::parse(&bytes).is_err());
        assert_eq!(result.ok(), Some(true), "must return an error rather than panic");
    }

    #[test]
    fn flag_masks_differ_between_versions_and_are_not_interchanged() {
        // The same byte is blight in v11 and ramp in v12.
        let tile = TerrainTile::new(0x2000, WaterAndFlags(0), 0x20, 0, 0x22);
        assert!(tile.flags(W3eVersion::V11).blight);
        assert!(!tile.flags(W3eVersion::V11).ramp);
        assert!(!tile.flags(W3eVersion::V12).blight);
    }

    #[test]
    fn v12_water_flag_lives_in_the_second_flag_byte() {
        let tile = TerrainTile::new(0x2000, WaterAndFlags(0), 0x00, TileFlags::V12_WATER, 0x22);
        assert!(tile.flags(W3eVersion::V12).water);
        // In v11 the same byte is the variation byte and must not read as water.
        assert!(!tile.flags(W3eVersion::V11).water);
    }

    #[test]
    fn unknown_water_bit_is_preserved_and_reported() {
        let mut bytes = build(W3eVersion::V11, 2, 2, 0);
        let first_tile = TEST_HEADER_LEN;
        let w = 0x8000u16.to_le_bytes();
        bytes[first_tile + 2] = w[0];
        bytes[first_tile + 3] = w[1];

        let t = Terrain::parse(&bytes).unwrap();
        assert!(t.tiles[0].water_and_flags.has_unknown_bit());
        assert!(t
            .diagnostics
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::W3eUnknownWaterBit));

        let out = t.write(W3eVersion::V11).unwrap();
        let t2 = Terrain::parse(&out).unwrap();
        assert!(t2.tiles[0].water_and_flags.has_unknown_bit());
    }

    #[test]
    fn water_word_is_read_as_unsigned_then_masked() {
        // Read as i16, 0xC000 is negative and masking would give a wrong level.
        let w = WaterAndFlags(0xC123);
        assert_eq!(w.water_raw(), 0x0123);
        assert!(w.is_boundary());
        assert!(w.has_unknown_bit());

        let only_boundary = WaterAndFlags(0x4123);
        assert_eq!(only_boundary.water_raw(), 0x0123);
        assert!(only_boundary.is_boundary());
        assert!(!only_boundary.has_unknown_bit());
    }

    #[test]
    fn v11_round_trip_is_byte_exact() {
        let bytes = build(W3eVersion::V11, 5, 7, 0x0A);
        let t = Terrain::parse(&bytes).unwrap();
        let out = t.write(W3eVersion::V11).unwrap();
        assert_eq!(out, bytes, "v11 round trip must be byte-exact");
    }

    #[test]
    fn v12_round_trip_is_byte_exact() {
        let bytes = build(W3eVersion::V12, 5, 7, 0x0A);
        let t = Terrain::parse(&bytes).unwrap();
        let out = t.write(W3eVersion::V12).unwrap();
        assert_eq!(out, bytes, "v12 round trip must be byte-exact");
    }

    #[test]
    fn v12_flag_and_variation_bytes_survive_round_trip() {
        let mut bytes = build(W3eVersion::V12, 1, 1, 0);
        let header_len = TEST_HEADER_LEN;
        bytes[header_len + 4] = TileFlags::V12_RAMP | TileFlags::V12_BLIGHT | 0x07;
        bytes[header_len + 5] = TileFlags::V12_WATER | TileFlags::V12_BOUNDARY | 0xFC;
        bytes[header_len + 6] = Variation::from_parts(21, 5).0;

        let t = Terrain::parse(&bytes).unwrap();
        let tile = t.tiles[0];
        assert!(tile.flags(W3eVersion::V12).ramp);
        assert!(tile.flags(W3eVersion::V12).blight);
        assert!(tile.flags(W3eVersion::V12).water);
        assert!(tile.flags(W3eVersion::V12).boundary);
        assert_eq!(tile.ground_texture(W3eVersion::V12), 7);
        assert_eq!(tile.variation(W3eVersion::V12).ground(), 21);
        assert_eq!(tile.variation(W3eVersion::V12).cliff(), 5);
        assert!(
            t.diagnostics
                .items()
                .iter()
                .any(|d| d.code == DiagnosticCode::W3eUnknownFlagBits),
            "reserved bits must be reported"
        );

        let out = t.write(W3eVersion::V12).unwrap();
        assert_eq!(out, bytes, "flags and reserved bits must survive the round trip");
    }

    #[test]
    fn downgrade_with_wide_texture_index_is_refused_not_truncated() {
        let mut bytes = build(W3eVersion::V12, 1, 1, 0);
        let header_len = TEST_HEADER_LEN;
        bytes[header_len + 4] = 0x3F; // texture 63, which v11 cannot hold
        let t = Terrain::parse(&bytes).unwrap();
        let err = t.write(W3eVersion::V11).unwrap_err();
        assert!(err.to_string().contains("refusing to truncate"), "{err}");
    }

    #[test]
    fn height_arithmetic_matches_the_documented_formula() {
        let flat = TerrainTile::new(GROUND_HEIGHT_ZERO as i16, WaterAndFlags(0), 0, 0, 0x22);
        assert!((flat.world_height(W3eVersion::V11) - 0.0).abs() < f32::EPSILON);
        assert!((flat.tile_height(W3eVersion::V11) - 0.0).abs() < f32::EPSILON);

        // raw = 0x2200 is +512, exactly one tile at layer height 2.
        let one_tile_up = TerrainTile::new(0x2200, WaterAndFlags(0), 0, 0, 0x22);
        assert!((one_tile_up.tile_height(W3eVersion::V11) - 1.0).abs() < f32::EPSILON);
        assert!(
            (one_tile_up.world_height(W3eVersion::V11) - WORLD_UNITS_PER_TILE).abs()
                < f32::EPSILON
        );

        // Layer height 3 is also +1 tile.
        let layer_up = TerrainTile::new(0x2000, WaterAndFlags(0), 0, 0, 0x23);
        assert!((layer_up.tile_height(W3eVersion::V11) - 1.0).abs() < f32::EPSILON);

        // The same struct read as v12 takes its layer height from byte7, so the
        // V11-shaped value reads as layer 0. That is exactly why the version has
        // to be passed explicitly.
        assert!((layer_up.tile_height(W3eVersion::V12) - (-2.0)).abs() < f32::EPSILON);
        let v12_layer_up = TerrainTile::new_v12(0x2000, WaterAndFlags(0), 0, 0, 0, 0x23);
        assert!((v12_layer_up.tile_height(W3eVersion::V12) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn water_height_uses_the_tileset_specific_offset() {
        let tile = TerrainTile::new(0x2000, WaterAndFlags(0x2000), 0, 0, 0x22);
        // slk height -0.7 gives an offset of -89.6, the commonly quoted number.
        let h = tile.water_world_height(-0.7 * WORLD_UNITS_PER_TILE);
        assert!((h - 89.6).abs() < 1e-4, "got {h}");
        // A different tileset gives a different world height for the same water
        // word, which is the point: it is not a constant.
        let h2 = tile.water_world_height(0.0);
        assert!((h2 - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn variation_split_is_five_three_and_ground_sentinel_needs_five_bits() {
        let v = Variation(0xFF);
        assert_eq!(v.ground(), 31);
        assert_eq!(v.cliff(), 7);
        // The sentinel 16 does not fit in 4 bits, which rules out a 4/4 split.
        // Pinned as a literal rather than compared against itself, so changing
        // the sentinel fails here instead of silently weakening the argument.
        assert_eq!(Variation::GROUND_SENTINEL, 16);
        assert_eq!(Variation::from_parts(16, 0).ground(), 16);
    }

    #[test]
    fn tile_index_is_south_west_origin() {
        let mut bytes = build(W3eVersion::V11, 3, 2, 0);
        let header_len = TEST_HEADER_LEN;
        // The first record, the south-west corner, gets a height of +512.
        bytes[header_len] = 0x00;
        bytes[header_len + 1] = 0x22;
        let t = Terrain::parse(&bytes).unwrap();
        let sw = t.tile_at(0, 0).unwrap();
        assert!((sw.tile_height(W3eVersion::V11) - 1.0).abs() < f32::EPSILON);
        assert_eq!(t.tiles[0], *sw);
    }

    #[test]
    fn world_origin_matches_minus_half_the_tile_span() {
        let bytes = build(W3eVersion::V11, 129, 129, 0);
        let t = Terrain::parse(&bytes).unwrap();
        assert_eq!(t.tile_width(), 128);
        assert_eq!(t.world_origin(), (-8192.0, -8192.0));
    }

    #[test]
    fn flag_and_layer_statistics_add_up() {
        let mut bytes = build(W3eVersion::V11, 2, 2, 0);
        let header_len = TEST_HEADER_LEN;
        bytes[header_len + 4] = TileFlags::V11_BLIGHT;
        bytes[header_len + 7 + 4] = TileFlags::V11_WATER;
        let t = Terrain::parse(&bytes).unwrap();
        let counts = t.flag_counts();
        assert_eq!(counts.blight, 1);
        assert_eq!(counts.water, 1);
        assert_eq!(t.layer_histogram()[2], 4);
    }

    #[test]
    fn output_is_deterministic() {
        let bytes = build(W3eVersion::V11, 4, 4, 3);
        let a = Terrain::parse(&bytes).unwrap().write(W3eVersion::V11).unwrap();
        let b = Terrain::parse(&bytes).unwrap().write(W3eVersion::V11).unwrap();
        assert_eq!(a, b);
    }
}

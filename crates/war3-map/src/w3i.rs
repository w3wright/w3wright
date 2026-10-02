//! `war3map.w3i`: map information.
//!
//! # Notes on the layout
//!
//! - Most fields are version-gated, so parsing follows the version branches
//!   exactly; reading a field that a version does not have shifts everything
//!   after it.
//! - The trailing sections (players, forces, upgrades, tech, random tables) have
//!   no count field at the top level. Their presence is decided by whether bytes
//!   remain, not by the version number.
//! - An unsupported version is an error. Returning an empty result instead would
//!   make the failure appear far from its cause.
//! - String fields may hold `TRIGSTR_nnn` references into `war3map.wts`. They are
//!   kept as stored here; resolving them is the caller's decision.

use std::fmt;

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};
use war3_core::{Error, FourCC, ParseError, Result, Vec3};

use crate::cursor::Cursor;

/// Highest `.w3i` version this build understands.
pub const MAX_SUPPORTED_VERSION: i32 = 33;

/// Version written by the 1.27 target release.
pub const VERSION_TFT: i32 = 25;
/// Version written by Reign of Chaos.
pub const VERSION_ROC: i32 = 18;
/// Version written by Reforged 1.32.
pub const VERSION_REFORGED: i32 = 31;

/// Main terrain tileset.
///
/// The 18 letters are not contiguous: `E`, `H`, `M`, `P`, `R`, `S`, `T` and `U`
/// are unused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tileset {
    /// `A` Ashenvale
    Ashenvale,
    /// `B` Barrens
    Barrens,
    /// `C` Felwood
    Felwood,
    /// `D` Dungeon
    Dungeon,
    /// `F` Lordaeron Fall
    LordaeronFall,
    /// `G` Underground
    Underground,
    /// `I` Icecrown
    Icecrown,
    /// `J` Dalaran Ruins
    DalaranRuins,
    /// `K` Black Citadel
    BlackCitadel,
    /// `L` Lordaeron Summer
    LordaeronSummer,
    /// `N` Northrend
    Northrend,
    /// `O` Outland
    Outland,
    /// `Q` Village Fall
    VillageFall,
    /// `V` Village
    Village,
    /// `W` Lordaeron Winter
    LordaeronWinter,
    /// `X` Dalaran
    Dalaran,
    /// `Y` Cityscape
    Cityscape,
    /// `Z` Sunken Ruins
    SunkenRuins,
    /// A letter outside the known table.
    ///
    /// Preserved rather than rejected: a future release could add a tileset
    /// without changing the file version.
    Unknown(u8),
}

impl Tileset {
    /// Decodes the stored byte.
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

    /// English name, for display.
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
}

impl fmt::Display for Tileset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(b) => write!(f, "Unknown({:?})", *b as char),
            other => write!(f, "{}", other.name()),
        }
    }
}

/// The map flags word.
///
/// Stored verbatim, with named accessors for the bits that are understood. Two
/// bits have no known meaning and are preserved by keeping the raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapFlags(pub u32);

impl MapFlags {
    /// `0x000001` hide the minimap on preview screens.
    pub const HIDE_MINIMAP: u32 = 0x0000_0001;
    /// `0x000002` change ally priorities.
    pub const CHANGE_ALLY_PRIORITIES: u32 = 0x0000_0002;
    /// `0x000004` suitable for melee play.
    pub const MELEE: u32 = 0x0000_0004;
    /// `0x000008` a non-default tileset is in use.
    pub const CUSTOM_TILESET: u32 = 0x0000_0008;
    /// `0x000010` unexplored areas are partly visible.
    pub const PARTIAL_FOG: u32 = 0x0000_0010;
    /// `0x000040` custom teams.
    pub const CUSTOM_TEAMS: u32 = 0x0000_0040;
    /// `0x000080` custom tech tree.
    pub const CUSTOM_TECH: u32 = 0x0000_0080;
    /// `0x000100` custom abilities.
    pub const CUSTOM_ABILITIES: u32 = 0x0000_0100;
    /// `0x000200` custom upgrades.
    pub const CUSTOM_UPGRADES: u32 = 0x0000_0200;
    /// `0x000800` water waves on cliff shores.
    pub const WATER_WAVES_CLIFF: u32 = 0x0000_0800;
    /// `0x001000` water waves on rolling shores.
    pub const WATER_WAVES_ROLLING: u32 = 0x0000_1000;
    /// `0x002000` terrain fog is in use.
    pub const TERRAIN_FOG: u32 = 0x0000_2000;
    /// `0x004000` the expansion is required.
    pub const EXPANSION_REQUIRED: u32 = 0x0000_4000;
    /// `0x008000` item classification.
    pub const ITEM_CLASSIFICATION: u32 = 0x0000_8000;
    /// `0x010000` custom water tint.
    pub const CUSTOM_WATER_TINT: u32 = 0x0001_0000;

    /// Whether a bit is set.
    #[must_use]
    pub const fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }

    /// Whether this is a melee map.
    ///
    /// Melee maps take their base objects from the standard library rather than
    /// the custom one.
    #[must_use]
    pub const fn is_melee(self) -> bool {
        self.has(Self::MELEE)
    }
}

/// One player, from the first trailing section.
#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    /// Slot number, zero-based.
    pub slot: i32,
    /// 1 = player, 2 = computer, 3 = neutral, 4 = reserved.
    pub player_type: i32,
    /// 0 = random, 1 = human, 2 = orc, 3 = undead, 4 = night elf.
    pub race: i32,
    /// Fixed start position.
    /// The raw "fixed start position" word. Non-zero means fixed.
    ///
    /// Kept as the word rather than as a `bool`, because real maps write more than
    /// one non-zero spelling (`1` and `2` both occur) and a write has to reproduce
    /// the one that was there. Normalising `2` to `1` changes the file, which is
    /// exactly what the round-trip check exists to catch.
    pub fixed_start_position: i32,
    /// Player name.
    pub name: String,
    /// Start position X.
    pub start_x: f32,
    /// Start position Y.
    pub start_y: f32,
    /// Ally low priority bitmap (version 5 and above).
    pub ally_low_priority: u32,
    /// Ally high priority bitmap (version 5 and above).
    pub ally_high_priority: u32,
    /// Enemy low priority bitmap (version 31 and above).
    pub enemy_low_priority: Option<u32>,
    /// Enemy high priority bitmap (version 31 and above).
    pub enemy_high_priority: Option<u32>,
}

impl Player {
    /// The start position.
    #[must_use]
    pub const fn start_position(&self) -> Vec3 {
        Vec3::new_xy(self.start_x, self.start_y)
    }
}

/// One custom upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upgrade {
    /// Bitmap of the players it applies to.
    pub players: u32,
    /// The upgrade's identifier.
    pub id: FourCC,
    /// Level.
    pub level: i32,
    /// Availability.
    pub state: UpgradeState,
}

/// Availability of a custom upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeState {
    /// 0: unavailable.
    NotAvailable,
    /// 1: available.
    Available,
    /// 2: researched.
    Researched,
    /// Any other value, preserved.
    Unknown(i32),
}

impl From<i32> for UpgradeState {
    fn from(v: i32) -> Self {
        match v {
            0 => Self::NotAvailable,
            1 => Self::Available,
            2 => Self::Researched,
            other => Self::Unknown(other),
        }
    }
}

impl UpgradeState {
    /// The value the file stores, so a write reproduces the read.
    #[must_use]
    pub const fn to_i32(self) -> i32 {
        match self {
            Self::NotAvailable => 0,
            Self::Available => 1,
            Self::Researched => 2,
            Self::Unknown(v) => v,
        }
    }
}

/// One force, from the second trailing section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Force {
    /// Force flags.
    pub flags: u32,
    /// Bitmap of member players.
    pub players: u32,
    /// Force name.
    pub name: String,
}

/// One random unit table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomUnitTable {
    /// Group number.
    pub group: i32,
    /// Group name.
    pub name: String,
    /// Per-column type: 0 = unit, 1 = building, 2 = item.
    pub column_types: Vec<i32>,
    /// Rows: a chance plus one identifier per column.
    pub rows: Vec<RandomUnitRow>,
}

/// One row of a random unit table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomUnitRow {
    /// Chance, as a percentage.
    pub chance: i32,
    /// One identifier per column.
    pub ids: Vec<FourCC>,
}

/// One random item table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomItemTable {
    /// Table number.
    pub number: i32,
    /// Table name.
    pub name: String,
    /// The item sets.
    pub sets: Vec<RandomItemSet>,
}

/// One item set of a random item table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomItemSet {
    /// The items in this set.
    pub items: Vec<RandomItemEntry>,
}

/// One item of an item set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomItemEntry {
    /// Chance, as a percentage.
    pub chance: i32,
    /// Item identifier.
    pub id: FourCC,
}

/// Ambient fog settings (version 19 and above).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    /// 0 = linear, 1 = exponential 1, 2 = exponential 2.
    pub style: i32,
    /// Start height along Z.
    pub start_height: f32,
    /// End height along Z.
    pub end_height: f32,
    /// Density.
    pub density: f32,
    /// RGBA.
    pub color: [u8; 4],
}

/// Water tint (version 25 and above).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterTint {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha.
    pub a: u8,
}

/// Which of the trailing sections a file actually carried.
///
/// Their presence is decided by "are there bytes left?", not by the version, so
/// **a section with a count of zero and an absent section are different files**.
/// Recording which ones were there is what lets a write tell them apart, the same
/// way `terrain: Option<Terrain>` separates "no `.w3e`" from "zero tiles".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrailingSections {
    /// The player section.
    pub players: bool,
    /// The force section (version 3 and above).
    pub forces: bool,
    /// The upgrade section.
    pub upgrades: bool,
    /// The tech section (version 7 and above).
    pub tech: bool,
    /// The random unit table section (version 12 and above).
    pub random_units: bool,
    /// The random item table section (version 24 and above).
    pub random_items: bool,
}

/// Everything `war3map.w3i` holds.
#[derive(Debug, Clone, PartialEq)]
pub struct MapInfo {
    /// Format version: 18 for RoC, 25 for TFT, 31 for 1.32, 32 and 33 for 2.0.3.
    pub format_version: i32,
    /// Save count (version 16 and above).
    pub save_count: Option<i32>,
    /// Editor version (version 16 and above).
    pub editor_version: Option<i32>,
    /// Game version `A.B.C.D` (version 27 and above); `None` for 1.27.
    ///
    /// This is what decides whether camera records carry the three floats added
    /// in 1.32, since the camera file's own version was not incremented.
    pub game_version: Option<[i32; 4]>,
    /// Map name.
    pub name: String,
    /// Author.
    pub author: String,
    /// Description.
    pub description: String,
    /// Recommended players (version 8 and above).
    pub recommended_players: Option<String>,
    /// Camera bounds, all eight floats kept **in file order**.
    ///
    /// The semantics of positions 2 and 3 are disputed — whether they are
    /// `bottom, right` or `right, bottom`. Keeping the order and deriving a
    /// rectangle with min/max is safe under either reading.
    pub camera_bounds: [f32; 8],
    /// The four edges of the unplayable area (version 14 and above); map width
    /// is `A + E + B`.
    pub unplayable: Option<[i32; 4]>,
    /// Playable width in tiles.
    pub playable_width: i32,
    /// Playable height in tiles.
    pub playable_height: i32,
    /// Map flags.
    pub flags: MapFlags,
    /// Main tileset (version 8 and above).
    pub tileset: Option<Tileset>,
    /// Campaign loading screen index (version 17 and above); -1 for none.
    pub loading_screen_number: Option<i32>,
    /// Imported loading screen path (version 10 and above, except 18 and 19).
    pub loading_screen_path: Option<String>,
    /// Loading screen text (version 10 and above).
    pub loading_screen_text: Option<String>,
    /// Loading screen title (version 11 and above).
    pub loading_screen_title: Option<String>,
    /// Loading screen subtitle (version 11 and above).
    pub loading_screen_subtitle: Option<String>,
    /// Game data set version (version 17 and above).
    pub game_data_set: Option<i32>,
    /// Prologue screen path (version 13 and above, except 18 and 19).
    pub prologue_screen_path: Option<String>,
    /// Prologue screen text (version 13 and above).
    pub prologue_screen_text: Option<String>,
    /// Prologue screen title (version 13 and above).
    pub prologue_screen_title: Option<String>,
    /// Prologue screen subtitle (version 13 and above).
    pub prologue_screen_subtitle: Option<String>,
    /// Ambient fog (version 19 and above).
    pub fog: Option<Fog>,
    /// Global weather identifier (version 21 and above); all zero means none.
    pub global_weather: Option<FourCC>,
    /// Custom sound environment (version 22 and above).
    pub sound_environment: Option<String>,
    /// Custom light environment letter (version 23 and above).
    pub light_environment: Option<u8>,
    /// Water tint (version 25 and above).
    pub water_tint: Option<WaterTint>,
    /// Script language, at the position versions 26 and 27 use.
    pub script_language: Option<i32>,
    /// Supported graphics modes (version 29 and above): 1 = SD, 2 = HD, 3 = both.
    pub graphics_modes: Option<i32>,
    /// Game data version (version 30 and above): 0 = RoC, 1 = TFT.
    pub game_data_version: Option<i32>,
    /// Bytes read before the camera bounds whose meaning is not decoded
    /// (versions 8 and below).
    ///
    /// Kept verbatim rather than dropped: a file this build no longer sees is
    /// still a file it has to be able to write back, and "not decoded" is not a
    /// licence to lose bytes.
    pub legacy_pre_camera: Vec<u8>,
    /// The four bytes versions 2 to 8 carry between the playable size and the
    /// flags. Same reasoning as [`Self::legacy_pre_camera`].
    pub legacy_post_size: Vec<u8>,
    /// Bytes read after the game data version (versions 32 and above).
    pub legacy_post_data_version: Vec<u8>,
    /// Bytes left over after the trailing sections.
    ///
    /// Non-empty means a field of that version is missing from this build. It is
    /// reported as [`DiagnosticCode::W3iTrailingBytesLeft`] **and kept**, so
    /// writing the file back reproduces what was there.
    pub trailing: Vec<u8>,
    /// Which trailing sections the file carried, including empty ones.
    pub trailing_sections: TrailingSections,
    /// Players.
    pub players: Vec<Player>,
    /// Forces.
    pub forces: Vec<Force>,
    /// Custom upgrades.
    pub upgrades: Vec<Upgrade>,
    /// Custom tech.
    pub tech: Vec<FourCC>,
    /// Random unit tables.
    pub random_units: Vec<RandomUnitTable>,
    /// Random item tables (version 24 and above).
    pub random_items: Vec<RandomItemTable>,
    /// Diagnostics from parsing.
    pub diagnostics: Diagnostics,
}

impl MapInfo {
    /// Total width in tiles, `A + E + B`.
    ///
    /// Below version 14 there is no unplayable area, so this equals the playable
    /// width.
    #[must_use]
    pub fn width(&self) -> i32 {
        match self.unplayable {
            Some([a, b, _c, _d]) => a + self.playable_width + b,
            None => self.playable_width,
        }
    }

    /// Total height in tiles, `C + F + D`.
    #[must_use]
    pub fn height(&self) -> i32 {
        match self.unplayable {
            Some([_a, _b, c, d]) => c + self.playable_height + d,
            None => self.playable_height,
        }
    }

    /// Parses raw bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let mut diagnostics = Diagnostics::new();

        let format_version = cursor.i32()?;
        if !(0..=MAX_SUPPORTED_VERSION).contains(&format_version) {
            // A hard error rather than an empty result: an unrecognised version
            // means the fields read below would be meaningless.
            return Err(Error::from(ParseError::UnsupportedVersion {
                format: "war3map.w3i",
                found: i64::from(format_version),
                supported: &[
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33,
                ],
            }));
        }

        let (save_count, editor_version) = if format_version >= 16 {
            (Some(cursor.i32()?), Some(cursor.i32()?))
        } else {
            (None, None)
        };

        let game_version = if format_version >= 27 {
            Some([cursor.i32()?, cursor.i32()?, cursor.i32()?, cursor.i32()?])
        } else {
            None
        };

        let name = cursor.cstr()?;
        let author = cursor.cstr()?;
        let description = cursor.cstr()?;
        let recommended_players = if format_version >= 8 {
            Some(cursor.cstr()?)
        } else {
            None
        };

        // Historical fields for versions up to 8: their meaning is not decoded,
        // but they are kept byte for byte so the file can be written back
        // unchanged (see `legacy_pre_camera`).
        let mut legacy_pre_camera = Vec::new();
        if format_version <= 3 {
            legacy_pre_camera.extend_from_slice(cursor.take(4)?);
            legacy_pre_camera.extend_from_slice(cursor.take(4)?);
        } else if format_version <= 8 {
            for _ in 0..6 {
                legacy_pre_camera.extend_from_slice(cursor.take(4)?);
            }
        }

        let mut camera_bounds = [0f32; 8];
        for slot in &mut camera_bounds {
            *slot = cursor.f32()?;
        }

        let unplayable = if format_version >= 14 {
            Some([cursor.i32()?, cursor.i32()?, cursor.i32()?, cursor.i32()?])
        } else {
            None
        };

        let playable_width = cursor.i32()?;
        let playable_height = cursor.i32()?;

        let legacy_post_size = if (2..=8).contains(&format_version) {
            cursor.take(4)?.to_vec()
        } else {
            Vec::new()
        };
        let flags = MapFlags(cursor.u32()?);

        let tileset = if format_version >= 8 {
            Some(Tileset::from_byte(cursor.u8()?))
        } else {
            None
        };

        let loading_screen_number = if format_version >= 17 {
            Some(cursor.i32()?)
        } else {
            None
        };

        let loading_screen_path =
            if format_version >= 10 && format_version != 18 && format_version != 19 {
                Some(cursor.cstr()?)
            } else {
                None
            };

        let (loading_screen_text, loading_screen_title, loading_screen_subtitle) =
            if format_version >= 10 {
                let text = cursor.cstr()?;
                let (title, subtitle) = if format_version >= 11 {
                    (Some(cursor.cstr()?), Some(cursor.cstr()?))
                } else {
                    (None, None)
                };
                (Some(text), title, subtitle)
            } else {
                (None, None, None)
            };

        let game_data_set = if format_version >= 17 {
            Some(cursor.i32()?)
        } else {
            None
        };

        let prologue_screen_path =
            if format_version >= 13 && format_version != 18 && format_version != 19 {
                Some(cursor.cstr()?)
            } else {
                None
            };

        let (prologue_screen_text, prologue_screen_title, prologue_screen_subtitle) =
            if format_version >= 13 {
                (
                    Some(cursor.cstr()?),
                    Some(cursor.cstr()?),
                    Some(cursor.cstr()?),
                )
            } else {
                (None, None, None)
            };

        let fog = if format_version >= 19 {
            Some(Fog {
                style: cursor.i32()?,
                start_height: cursor.f32()?,
                end_height: cursor.f32()?,
                density: cursor.f32()?,
                color: [cursor.u8()?, cursor.u8()?, cursor.u8()?, cursor.u8()?],
            })
        } else {
            None
        };

        let global_weather = if format_version >= 21 {
            Some(cursor.fourcc()?)
        } else {
            None
        };
        let sound_environment = if format_version >= 22 {
            Some(cursor.cstr()?)
        } else {
            None
        };
        let light_environment = if format_version >= 23 {
            Some(cursor.u8()?)
        } else {
            None
        };
        let water_tint = if format_version >= 25 {
            Some(WaterTint {
                r: cursor.u8()?,
                g: cursor.u8()?,
                b: cursor.u8()?,
                a: cursor.u8()?,
            })
        } else {
            None
        };

        // The script language appears in two places: in the main block for
        // versions 26 and 27, and in the trailing section from 28 onwards. Only
        // the former is read here.
        let script_language = if format_version == 26 || format_version == 27 {
            Some(cursor.i32()?)
        } else {
            None
        };
        if script_language.is_none() && (26..=27).contains(&format_version) {
            diagnostics.push(Diagnostic::info(
                DiagnosticCode::W3iScriptLanguageUnknown,
                "this version has no script language flag",
            ));
        }

        let graphics_modes = if format_version >= 29 {
            Some(cursor.i32()?)
        } else {
            None
        };
        let game_data_version = if format_version >= 30 {
            Some(cursor.i32()?)
        } else {
            None
        };

        let mut legacy_post_data_version = Vec::new();
        if format_version >= 32 {
            // Forced camera zoom values. The semantics are not documented, but
            // they must be read or the trailing sections shift.
            legacy_post_data_version.extend_from_slice(cursor.take(4)?);
            legacy_post_data_version.extend_from_slice(cursor.take(4)?);
        }
        if format_version >= 33 {
            legacy_post_data_version.extend_from_slice(cursor.take(4)?);
        }

        // ---- trailing sections ----
        // Their presence is decided by whether bytes remain, not by the version.
        let mut players = Vec::new();
        let mut forces = Vec::new();
        let mut upgrades = Vec::new();
        let mut tech = Vec::new();
        let mut random_units = Vec::new();
        let mut random_items = Vec::new();

        let mut sections = TrailingSections::default();

        // A section that fails to parse must not cost bytes. The cursor is rewound
        // to that section's first byte and everything from there to the end is
        // kept verbatim in `trailing`, so a file this build only half understands
        // still writes back unchanged.
        let mut opaque_tail_from: Option<usize> = None;

        if !cursor.is_at_end() {
            let start = cursor.position();
            match read_players(&mut cursor, format_version) {
                Ok(list) => {
                    players = list;
                    sections.players = true;
                }
                Err(e) => {
                    diagnostics.push(Diagnostic::warn(
                        DiagnosticCode::W3iMissingTrailingData,
                        format!(
                            "the player section failed to parse, so the tail is kept as-is: {e}"
                        ),
                    ));
                    opaque_tail_from = Some(start);
                }
            }
        } else {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::W3iMissingTrailingData,
                "the file ends before the trailing sections (no players or forces)",
            ));
        }

        if opaque_tail_from.is_none() && !cursor.is_at_end() && format_version >= 3 {
            let start = cursor.position();
            match read_forces(&mut cursor) {
                Ok(list) => {
                    forces = list;
                    sections.forces = true;
                }
                Err(_) => opaque_tail_from = Some(start),
            }
        }
        if opaque_tail_from.is_none() && !cursor.is_at_end() {
            let start = cursor.position();
            match read_upgrades(&mut cursor) {
                Ok(list) => {
                    upgrades = list;
                    sections.upgrades = true;
                }
                Err(_) => opaque_tail_from = Some(start),
            }
        }
        if opaque_tail_from.is_none() && !cursor.is_at_end() && format_version >= 7 {
            let start = cursor.position();
            match cursor.i32() {
                Ok(count) => {
                    sections.tech = true;
                    let mut short = false;
                    for _ in 0..count.max(0) {
                        match cursor.fourcc() {
                            Ok(id) => tech.push(id),
                            Err(_) => {
                                short = true;
                                break;
                            }
                        }
                    }
                    if short {
                        opaque_tail_from = Some(start);
                    }
                }
                Err(_) => opaque_tail_from = Some(start),
            }
        }
        if opaque_tail_from.is_none() && !cursor.is_at_end() && format_version >= 12 {
            let start = cursor.position();
            match read_random_units(&mut cursor) {
                Ok(list) => {
                    random_units = list;
                    sections.random_units = true;
                }
                Err(_) => opaque_tail_from = Some(start),
            }
        }
        if opaque_tail_from.is_none() && !cursor.is_at_end() && format_version >= 24 {
            let start = cursor.position();
            match read_random_items(&mut cursor) {
                Ok(list) => {
                    random_items = list;
                    sections.random_items = true;
                }
                Err(_) => opaque_tail_from = Some(start),
            }
        }

        if let Some(start) = opaque_tail_from {
            // The sections read before the failure stay; the rest is opaque.
            cursor.seek(start)?;
            diagnostics.push(Diagnostic::info(
                DiagnosticCode::W3iMissingTrailingData,
                format!(
                    "{} bytes from offset {start} are kept verbatim: a trailing section could not \
                     be parsed",
                    cursor.remaining()
                ),
            ));
        }

        // Leftover bytes are reported: they are the only signal that a version's
        // field is missing from this implementation. They are also **kept**, so
        // that writing the file back does not quietly shorten it.
        let leftover_at = cursor.position();
        let remaining = cursor.remaining();
        let trailing = cursor.take(remaining)?.to_vec();
        if remaining > 0 {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::W3iTrailingBytesLeft,
                format!(
                    "{remaining} bytes remain after parsing (offset {leftover_at} of {}) -- a field \
                     of version {format_version} may be missing from this build",
                    cursor.len()
                ),
            ));
        }

        Ok(Self {
            format_version,
            save_count,
            editor_version,
            game_version,
            name,
            author,
            description,
            recommended_players,
            camera_bounds,
            unplayable,
            playable_width,
            playable_height,
            flags,
            tileset,
            loading_screen_number,
            loading_screen_path,
            loading_screen_text,
            loading_screen_title,
            loading_screen_subtitle,
            game_data_set,
            prologue_screen_path,
            prologue_screen_text,
            prologue_screen_title,
            prologue_screen_subtitle,
            fog,
            global_weather,
            sound_environment,
            light_environment,
            water_tint,
            script_language,
            graphics_modes,
            game_data_version,
            legacy_pre_camera,
            legacy_post_size,
            legacy_post_data_version,
            trailing,
            trailing_sections: sections,
            players,
            forces,
            upgrades,
            tech,
            random_units,
            random_items,
            diagnostics,
        })
    }

    /// Serialises back to `war3map.w3i` bytes.
    ///
    /// # Why this mirrors `parse` line for line
    ///
    /// The prototype's hard metric is that a member survives `extract → build`
    /// byte for byte, so this has to reproduce the *input* rather than a
    /// normalised version of it: the same version gates, the same field order, and
    /// the same bytes for the parts whose meaning this build does not decode
    /// ([`Self::legacy_pre_camera`], [`Self::legacy_post_size`],
    /// [`Self::legacy_post_data_version`], [`Self::trailing`]).
    ///
    /// # About the `unwrap_or` fallbacks
    ///
    /// They only fire for a model built by hand rather than by `parse`: a file
    /// that came from `parse` always carries the fields its version requires.
    /// A model that would serialise differently from its source shows up as a byte
    /// difference, which is exactly what the round-trip tests and `war3 map
    /// extract`'s self-check look for.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let v = self.format_version;
        let mut out = Vec::new();

        out.extend_from_slice(&v.to_le_bytes());
        if v >= 16 {
            push_i32(&mut out, self.save_count.unwrap_or(0));
            push_i32(&mut out, self.editor_version.unwrap_or(0));
        }
        if v >= 27 {
            for part in self.game_version.unwrap_or([0; 4]) {
                push_i32(&mut out, part);
            }
        }
        push_cstr(&mut out, &self.name);
        push_cstr(&mut out, &self.author);
        push_cstr(&mut out, &self.description);
        if v >= 8 {
            push_cstr(&mut out, self.recommended_players.as_deref().unwrap_or(""));
        }

        out.extend_from_slice(&self.legacy_pre_camera);
        for bound in self.camera_bounds {
            out.extend_from_slice(&bound.to_le_bytes());
        }
        if v >= 14 {
            for edge in self.unplayable.unwrap_or([0; 4]) {
                push_i32(&mut out, edge);
            }
        }
        push_i32(&mut out, self.playable_width);
        push_i32(&mut out, self.playable_height);
        out.extend_from_slice(&self.legacy_post_size);
        push_u32(&mut out, self.flags.0);

        if v >= 8 {
            out.push(self.tileset.unwrap_or(Tileset::Unknown(0)).to_byte());
        }
        if v >= 17 {
            push_i32(&mut out, self.loading_screen_number.unwrap_or(-1));
        }
        if v >= 10 && v != 18 && v != 19 {
            push_cstr(&mut out, self.loading_screen_path.as_deref().unwrap_or(""));
        }
        if v >= 10 {
            push_cstr(&mut out, self.loading_screen_text.as_deref().unwrap_or(""));
            if v >= 11 {
                push_cstr(&mut out, self.loading_screen_title.as_deref().unwrap_or(""));
                push_cstr(
                    &mut out,
                    self.loading_screen_subtitle.as_deref().unwrap_or(""),
                );
            }
        }
        if v >= 17 {
            push_i32(&mut out, self.game_data_set.unwrap_or(0));
        }
        if v >= 13 && v != 18 && v != 19 {
            push_cstr(&mut out, self.prologue_screen_path.as_deref().unwrap_or(""));
        }
        if v >= 13 {
            push_cstr(&mut out, self.prologue_screen_text.as_deref().unwrap_or(""));
            push_cstr(
                &mut out,
                self.prologue_screen_title.as_deref().unwrap_or(""),
            );
            push_cstr(
                &mut out,
                self.prologue_screen_subtitle.as_deref().unwrap_or(""),
            );
        }

        if v >= 19 {
            let fog = self.fog.as_ref();
            push_i32(&mut out, fog.map_or(0, |f| f.style));
            push_f32(&mut out, fog.map_or(0.0, |f| f.start_height));
            push_f32(&mut out, fog.map_or(0.0, |f| f.end_height));
            push_f32(&mut out, fog.map_or(0.0, |f| f.density));
            out.extend_from_slice(&fog.map_or([0; 4], |f| f.color));
        }
        if v >= 21 {
            out.extend_from_slice(
                &self
                    .global_weather
                    .unwrap_or(FourCC::new([0; 4]))
                    .to_bytes(),
            );
        }
        if v >= 22 {
            push_cstr(&mut out, self.sound_environment.as_deref().unwrap_or(""));
        }
        if v >= 23 {
            out.push(self.light_environment.unwrap_or(0));
        }
        if v >= 25 {
            let tint = self.water_tint.as_ref();
            for byte in tint.map_or([0; 4], |t| [t.r, t.g, t.b, t.a]) {
                out.push(byte);
            }
        }
        // The script language has two homes; this is the main-block one, which
        // only versions 26 and 27 use.
        if v == 26 || v == 27 {
            push_i32(&mut out, self.script_language.unwrap_or(-1));
        }
        if v >= 29 {
            push_i32(&mut out, self.graphics_modes.unwrap_or(0));
        }
        if v >= 30 {
            push_i32(&mut out, self.game_data_version.unwrap_or(0));
        }
        out.extend_from_slice(&self.legacy_post_data_version);

        // ---- trailing sections ----
        // Written exactly where `parse` found them: presence is a property of the
        // file, recorded in `trailing_sections`, not of whether a list is empty.
        let sections = self.trailing_sections;

        if sections.players {
            push_i32(&mut out, len(&self.players));
            for player in &self.players {
                push_i32(&mut out, player.slot);
                push_i32(&mut out, player.player_type);
                push_i32(&mut out, player.race);
                push_i32(&mut out, player.fixed_start_position);
                push_cstr(&mut out, &player.name);
                push_f32(&mut out, player.start_x);
                push_f32(&mut out, player.start_y);
                if v >= 5 {
                    push_u32(&mut out, player.ally_low_priority);
                    push_u32(&mut out, player.ally_high_priority);
                }
                if v >= 31 {
                    push_u32(&mut out, player.enemy_low_priority.unwrap_or(0));
                    push_u32(&mut out, player.enemy_high_priority.unwrap_or(0));
                }
            }
        }

        if sections.forces {
            push_i32(&mut out, len(&self.forces));
            for force in &self.forces {
                push_u32(&mut out, force.flags);
                push_u32(&mut out, force.players);
                push_cstr(&mut out, &force.name);
            }
        }

        if sections.upgrades {
            push_i32(&mut out, len(&self.upgrades));
            for upgrade in &self.upgrades {
                push_u32(&mut out, upgrade.players);
                out.extend_from_slice(&upgrade.id.to_bytes());
                push_i32(&mut out, upgrade.level);
                push_i32(&mut out, upgrade.state.to_i32());
            }
        }

        if sections.tech {
            push_i32(&mut out, len(&self.tech));
            for id in &self.tech {
                out.extend_from_slice(&id.to_bytes());
            }
        }

        if sections.random_units {
            push_i32(&mut out, len(&self.random_units));
            for table in &self.random_units {
                push_i32(&mut out, table.group);
                push_cstr(&mut out, &table.name);
                push_i32(&mut out, len(&table.column_types));
                for column in &table.column_types {
                    push_i32(&mut out, *column);
                }
                push_i32(&mut out, len(&table.rows));
                for row in &table.rows {
                    push_i32(&mut out, row.chance);
                    for id in &row.ids {
                        out.extend_from_slice(&id.to_bytes());
                    }
                }
            }
        }

        if sections.random_items {
            push_i32(&mut out, len(&self.random_items));
            for table in &self.random_items {
                push_i32(&mut out, table.number);
                push_cstr(&mut out, &table.name);
                push_i32(&mut out, len(&table.sets));
                for set in &table.sets {
                    push_i32(&mut out, len(&set.items));
                    for item in &set.items {
                        push_i32(&mut out, item.chance);
                        out.extend_from_slice(&item.id.to_bytes());
                    }
                }
            }
        }

        out.extend_from_slice(&self.trailing);
        out
    }
}

/// Reads a count word.
///
/// A negative count is refused rather than clamped to zero: it means the cursor is
/// not where this build thinks it is, so what follows is not the structure it
/// expects. Refusing lets the caller keep those bytes verbatim instead of writing
/// a normalised — and wrong — file.
fn read_count(cursor: &mut Cursor<'_>, what: &'static str) -> Result<i32> {
    let count = cursor.i32()?;
    if count < 0 {
        return Err(Error::from(ParseError::Validation {
            check: "non-negative count",
            detail: format!("the {what} section starts with a count of {count}"),
        }));
    }
    Ok(count)
}

/// `len` as the `i32` the format uses for count words.
fn len<T>(list: &[T]) -> i32 {
    i32::try_from(list.len()).unwrap_or(i32::MAX)
}

fn push_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Writes a C string: the bytes, then the terminator.
///
/// The reader decodes these lossily, so a file whose strings were **not** UTF-8
/// does not round-trip byte for byte. That is not guessed at here: the caller's
/// self-check compares the result with the original and falls back to storing the
/// member as binary when they differ.
fn push_cstr(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
}

fn read_players(cursor: &mut Cursor<'_>, format_version: i32) -> Result<Vec<Player>> {
    let count = read_count(cursor, "player")?;
    let mut out = Vec::with_capacity(count.clamp(0, 1024) as usize);
    for _ in 0..count {
        let slot = cursor.i32()?;
        let player_type = cursor.i32()?;
        let race = cursor.i32()?;
        let fixed_start_position = cursor.i32()?;
        let name = cursor.cstr()?;
        let start_x = cursor.f32()?;
        let start_y = cursor.f32()?;
        let (ally_low_priority, ally_high_priority) = if format_version >= 5 {
            (cursor.u32()?, cursor.u32()?)
        } else {
            (0, 0)
        };
        let (enemy_low_priority, enemy_high_priority) = if format_version >= 31 {
            (Some(cursor.u32()?), Some(cursor.u32()?))
        } else {
            (None, None)
        };
        out.push(Player {
            slot,
            player_type,
            race,
            fixed_start_position,
            name,
            start_x,
            start_y,
            ally_low_priority,
            ally_high_priority,
            enemy_low_priority,
            enemy_high_priority,
        });
    }
    Ok(out)
}

fn read_forces(cursor: &mut Cursor<'_>) -> Result<Vec<Force>> {
    let count = read_count(cursor, "force")?;
    let mut out = Vec::with_capacity(count.clamp(0, 1024) as usize);
    for _ in 0..count {
        let flags = cursor.u32()?;
        let players = cursor.u32()?;
        let name = cursor.cstr()?;
        out.push(Force {
            flags,
            players,
            name,
        });
    }
    Ok(out)
}

fn read_upgrades(cursor: &mut Cursor<'_>) -> Result<Vec<Upgrade>> {
    let count = read_count(cursor, "upgrade")?;
    let mut out = Vec::with_capacity(count.clamp(0, 4096) as usize);
    for _ in 0..count {
        let players = cursor.u32()?;
        let id = cursor.fourcc()?;
        let level = cursor.i32()?;
        let state = UpgradeState::from(cursor.i32()?);
        out.push(Upgrade {
            players,
            id,
            level,
            state,
        });
    }
    Ok(out)
}

fn read_random_units(cursor: &mut Cursor<'_>) -> Result<Vec<RandomUnitTable>> {
    let count = read_count(cursor, "random unit")?;
    let mut out = Vec::with_capacity(count.clamp(0, 1024) as usize);
    for _ in 0..count {
        let group = cursor.i32()?;
        let name = cursor.cstr()?;
        let columns = read_count(cursor, "random unit column")?.clamp(0, 1024) as usize;
        let column_types = cursor
            .u32s(columns)?
            .into_iter()
            .map(|v| v as i32)
            .collect();
        let rows = read_count(cursor, "random unit row")?.clamp(0, 65536) as usize;
        let mut row_list = Vec::with_capacity(rows);
        for _ in 0..rows {
            let chance = cursor.i32()?;
            let mut ids = Vec::with_capacity(columns);
            for _ in 0..columns {
                ids.push(cursor.fourcc()?);
            }
            row_list.push(RandomUnitRow { chance, ids });
        }
        out.push(RandomUnitTable {
            group,
            name,
            column_types,
            rows: row_list,
        });
    }
    Ok(out)
}

fn read_random_items(cursor: &mut Cursor<'_>) -> Result<Vec<RandomItemTable>> {
    let count = read_count(cursor, "random item")?;
    let mut out = Vec::with_capacity(count.clamp(0, 1024) as usize);
    for _ in 0..count {
        let number = cursor.i32()?;
        let name = cursor.cstr()?;
        let sets = read_count(cursor, "random item set")?.clamp(0, 65536) as usize;
        let mut set_list = Vec::with_capacity(sets);
        for _ in 0..sets {
            let items = read_count(cursor, "random item")?.clamp(0, 65536) as usize;
            let mut item_list = Vec::with_capacity(items);
            for _ in 0..items {
                let chance = cursor.i32()?;
                let id = cursor.fourcc()?;
                item_list.push(RandomItemEntry { chance, id });
            }
            set_list.push(RandomItemSet { items: item_list });
        }
        out.push(RandomItemTable {
            number,
            name,
            sets: set_list,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_file_round_trips_byte_for_byte() {
        let bytes = minimal_v25();
        let parsed = MapInfo::parse(&bytes).unwrap();
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn a_tail_that_cannot_be_parsed_is_reported_and_kept() {
        // `minimal_v25` carries no trailing sections at all, so four extra bytes
        // look like a player count: the section read fails. Reporting that is not
        // enough — the bytes have to survive the round trip, or a file this build
        // only half understands would be quietly shortened.
        let mut bytes = minimal_v25();
        bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let parsed = MapInfo::parse(&bytes).unwrap();
        assert!(
            parsed.diagnostics.warning_count() > 0,
            "a tail that cannot be parsed has to be reported"
        );
        assert_eq!(
            parsed.to_bytes(),
            bytes,
            "an unparsable tail is kept, not dropped"
        );
    }

    /// Builds a minimal version 25 `.w3i` with no trailing sections.
    fn minimal_v25() -> Vec<u8> {
        let mut b = Vec::new();
        let push_i32 = |b: &mut Vec<u8>, v: i32| b.extend_from_slice(&v.to_le_bytes());
        let push_str = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        };

        push_i32(&mut b, 25);
        push_i32(&mut b, 1); // save count
        push_i32(&mut b, 6059); // editor version
        push_str(&mut b, "Test Map");
        push_str(&mut b, "Author");
        push_str(&mut b, "Desc");
        push_str(&mut b, "2v2");
        for _ in 0..8 {
            b.extend_from_slice(&0f32.to_le_bytes()); // camera bounds
        }
        push_i32(&mut b, 16); // unplayable A
        push_i32(&mut b, 16); // unplayable B
        push_i32(&mut b, 16); // unplayable C
        push_i32(&mut b, 16); // unplayable D
        push_i32(&mut b, 96); // playable width
        push_i32(&mut b, 96); // playable height
        push_i32(&mut b, 0x0040); // flags
        b.push(b'L'); // tileset
        push_i32(&mut b, -1); // loading screen number
        push_str(&mut b, ""); // loading screen path
        push_str(&mut b, ""); // loading screen text
        push_str(&mut b, ""); // loading screen title
        push_str(&mut b, ""); // loading screen subtitle
        push_i32(&mut b, 1); // game data set
        push_str(&mut b, ""); // prologue path
        push_str(&mut b, ""); // prologue text
        push_str(&mut b, ""); // prologue title
        push_str(&mut b, ""); // prologue subtitle
        push_i32(&mut b, 0); // fog style
        b.extend_from_slice(&0f32.to_le_bytes()); // fog start
        b.extend_from_slice(&0f32.to_le_bytes()); // fog end
        b.extend_from_slice(&0f32.to_le_bytes()); // fog density
        b.extend_from_slice(&[0, 0, 0, 255]); // fog colour
        b.extend_from_slice(b"\0\0\0\0"); // global weather
        push_str(&mut b, ""); // sound environment
        b.push(b'L'); // light environment
        b.extend_from_slice(&[255, 255, 255, 255]); // water tint
        b
    }

    #[test]
    fn parses_minimal_v25_main_block() {
        let info = MapInfo::parse(&minimal_v25()).unwrap();
        assert_eq!(info.format_version, 25);
        assert_eq!(info.save_count, Some(1));
        assert_eq!(info.editor_version, Some(6059));
        assert_eq!(info.name, "Test Map");
        assert_eq!(info.author, "Author");
        assert_eq!(info.description, "Desc");
        assert_eq!(info.recommended_players.as_deref(), Some("2v2"));
        assert_eq!(info.tileset, Some(Tileset::LordaeronSummer));
        assert_eq!(info.playable_width, 96);
        assert_eq!(info.playable_height, 96);
        // A + E + B = 16 + 96 + 16
        assert_eq!(info.width(), 128);
        assert_eq!(info.height(), 128);
        assert!(!info.flags.is_melee());
        assert!(info.flags.has(MapFlags::CUSTOM_TEAMS));
        assert_eq!(
            info.water_tint,
            Some(WaterTint {
                r: 255,
                g: 255,
                b: 255,
                a: 255
            })
        );
    }

    #[test]
    fn missing_trailing_data_is_reported_not_silent() {
        let info = MapInfo::parse(&minimal_v25()).unwrap();
        assert!(
            info.diagnostics
                .items()
                .iter()
                .any(|d| d.code == DiagnosticCode::W3iMissingTrailingData),
            "absent trailing sections must be reported"
        );
    }

    #[test]
    fn unsupported_version_is_a_hard_error_not_an_empty_map() {
        let mut bytes = 99i32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0u8; 64]);
        let err = MapInfo::parse(&bytes).unwrap_err();
        assert!(err.to_string().contains("99"), "{err}");
    }

    #[test]
    fn tileset_round_trips_every_known_letter() {
        let letters = b"ABCDFGIJKLNOQVWXYZ";
        for &letter in letters {
            let ts = Tileset::from_byte(letter);
            assert_eq!(ts.to_byte(), letter);
            assert_ne!(ts.name(), "Unknown");
        }
    }

    #[test]
    fn unknown_tileset_letter_is_preserved() {
        let ts = Tileset::from_byte(b'E');
        assert_eq!(ts, Tileset::Unknown(b'E'));
        assert_eq!(ts.to_byte(), b'E');
        assert!(ts.to_string().contains('E'));
    }

    #[test]
    fn tileset_is_case_insensitive_on_read() {
        assert_eq!(Tileset::from_byte(b'l'), Tileset::LordaeronSummer);
    }

    #[test]
    fn width_without_unplayable_equals_playable() {
        let mut info = MapInfo::parse(&minimal_v25()).unwrap();
        info.unplayable = None;
        assert_eq!(info.width(), 96);
    }
}

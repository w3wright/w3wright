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
    pub fixed_start_position: bool,
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

        // Historical fields for versions up to 8. The values are discarded, but
        // they must still be read or everything after them shifts.
        if format_version <= 3 {
            cursor.take(4)?;
            cursor.take(4)?;
        } else if format_version <= 8 {
            cursor.f32()?;
            cursor.i32()?;
            cursor.f32()?;
            cursor.f32()?;
            cursor.f32()?;
            cursor.i32()?;
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

        if (2..=8).contains(&format_version) {
            cursor.i32()?;
        }
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

        if format_version >= 32 {
            // Forced camera zoom values. The semantics are not documented, but
            // they must be read or the trailing sections shift.
            cursor.i32()?;
            cursor.i32()?;
        }
        if format_version >= 33 {
            cursor.i32()?;
        }

        // ---- trailing sections ----
        // Their presence is decided by whether bytes remain, not by the version.
        let mut players = Vec::new();
        let mut forces = Vec::new();
        let mut upgrades = Vec::new();
        let mut tech = Vec::new();
        let mut random_units = Vec::new();
        let mut random_items = Vec::new();

        if !cursor.is_at_end() {
            match read_players(&mut cursor, format_version) {
                Ok(list) => players = list,
                Err(e) => diagnostics.push(Diagnostic::warn(
                    DiagnosticCode::W3iMissingTrailingData,
                    format!(
                        "the player section failed to parse; the rest of the tail was skipped: {e}"
                    ),
                )),
            }
        } else {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::W3iMissingTrailingData,
                "the file ends before the trailing sections (no players or forces)",
            ));
        }

        if !cursor.is_at_end() && format_version >= 3 {
            if let Ok(list) = read_forces(&mut cursor) {
                forces = list;
            }
        }
        if !cursor.is_at_end() {
            if let Ok(list) = read_upgrades(&mut cursor) {
                upgrades = list;
            }
        }
        if !cursor.is_at_end() && format_version >= 7 {
            if let Ok(count) = cursor.i32() {
                for _ in 0..count.max(0) {
                    match cursor.fourcc() {
                        Ok(id) => tech.push(id),
                        Err(_) => break,
                    }
                }
            }
        }
        if !cursor.is_at_end() && format_version >= 12 {
            if let Ok(list) = read_random_units(&mut cursor) {
                random_units = list;
            }
        }
        if !cursor.is_at_end() && format_version >= 24 {
            if let Ok(list) = read_random_items(&mut cursor) {
                random_items = list;
            }
        }

        // Leftover bytes are reported: they are the only signal that a version's
        // field is missing from this implementation.
        if cursor.remaining() > 0 {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::W3iTrailingBytesLeft,
                format!(
                    "{} bytes remain after parsing (offset {} of {}) -- a field of version {} may \
                     be missing from this build",
                    cursor.remaining(),
                    cursor.position(),
                    cursor.len(),
                    format_version
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
            players,
            forces,
            upgrades,
            tech,
            random_units,
            random_items,
            diagnostics,
        })
    }
}

fn read_players(cursor: &mut Cursor<'_>, format_version: i32) -> Result<Vec<Player>> {
    let count = cursor.i32()?;
    let mut out = Vec::with_capacity(count.clamp(0, 1024) as usize);
    for _ in 0..count.max(0) {
        let slot = cursor.i32()?;
        let player_type = cursor.i32()?;
        let race = cursor.i32()?;
        let fixed_start_position = cursor.i32()? != 0;
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
    let count = cursor.i32()?;
    let mut out = Vec::with_capacity(count.clamp(0, 1024) as usize);
    for _ in 0..count.max(0) {
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
    let count = cursor.i32()?;
    let mut out = Vec::with_capacity(count.clamp(0, 4096) as usize);
    for _ in 0..count.max(0) {
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
    let count = cursor.i32()?;
    let mut out = Vec::with_capacity(count.clamp(0, 1024) as usize);
    for _ in 0..count.max(0) {
        let group = cursor.i32()?;
        let name = cursor.cstr()?;
        let columns = cursor.i32()?.clamp(0, 1024) as usize;
        let column_types = cursor
            .u32s(columns)?
            .into_iter()
            .map(|v| v as i32)
            .collect();
        let rows = cursor.i32()?.clamp(0, 65536) as usize;
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
    let count = cursor.i32()?;
    let mut out = Vec::with_capacity(count.clamp(0, 1024) as usize);
    for _ in 0..count.max(0) {
        let number = cursor.i32()?;
        let name = cursor.cstr()?;
        let sets = cursor.i32()?.clamp(0, 65536) as usize;
        let mut set_list = Vec::with_capacity(sets);
        for _ in 0..sets {
            let items = cursor.i32()?.clamp(0, 65536) as usize;
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

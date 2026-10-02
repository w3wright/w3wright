//! The text form of `war3map.w3i`.
//!
//! # Why a text form at all
//!
//! A source project exists so a map can be reviewed and edited as text. `w3i` is
//! the first member to get one: it holds the map name, author, description, the
//! player and force setup, and the small settings no other file carries.
//!
//! # The three rules this follows
//!
//! - **Nothing is dropped.** Parts of the format this build does not decode —
//!   the pre-8 legacy words, the version-32 extra words, the bytes after the
//!   trailing sections — are written out as hex, not omitted.
//! - **Absent and empty are different.** A key that is not there means the model
//!   field is `None`; a key present with an empty value means `Some("")`. The
//!   writer relies on that distinction, and so does a reader checking its work.
//! - **Ids are hex when they have to be.** A FourCC prints as its four characters
//!   when they are printable (`A000`) and as `0x…` when they are not, so an id
//!   that is not text still survives.
//!
//! # What does not round-trip
//!
//! A file whose strings are not UTF-8: the reader decodes them lossily, so the
//! bytes cannot come back. That is not guessed at here — `extract` compares the
//! result with the original and keeps the member as binary when they differ.
//!
//! ```text
//! [info]
//! version = "25"
//! name = "TRIGSTR_010"
//! ...
//! [players]
//! [player]
//! slot = "0"
//! ```

use war3_core::{Error, Result};
use war3_map::w3i::{
    Fog, Force, MapFlags, MapInfo, Player, RandomItemEntry, RandomItemSet, RandomItemTable,
    RandomUnitRow, RandomUnitTable, Tileset, TrailingSections, Upgrade, UpgradeState, WaterTint,
};

use crate::codecs::{fourcc_text, hex, parse_fourcc, Codec};
use crate::ini::{float_text, parse_float, Document, Section};

/// The codec for a member, if it has a text form yet.
#[must_use]
pub fn codec_for(member: &str) -> Option<Codec> {
    match member.to_ascii_lowercase().as_str() {
        "war3map.w3i" => Some(Codec {
            to_text: w3i_to_text,
            from_text: w3i_from_text,
        }),
        _ => None,
    }
}

fn w3i_to_text(_member: &str, bytes: &[u8]) -> Result<String> {
    Ok(to_document(&MapInfo::parse(bytes)?).render())
}

fn w3i_from_text(_member: &str, text: &str) -> Result<Vec<u8>> {
    Ok(from_document(&Document::parse(text)?)?.to_bytes())
}

/// Renders a map-info model as a document.
#[must_use]
pub fn to_document(info: &MapInfo) -> Document {
    let mut doc = Document::new();
    {
        let s = doc.push("info");
        s.set("version", info.format_version.to_string());
        if let Some(v) = info.save_count {
            s.set("save_count", v.to_string());
        }
        if let Some(v) = info.editor_version {
            s.set("editor_version", v.to_string());
        }
        if let Some(v) = info.game_version {
            s.set("game_version", numbers(&v));
        }
        s.set("name", &info.name);
        s.set("author", &info.author);
        s.set("description", &info.description);
        if let Some(v) = &info.recommended_players {
            s.set("recommended_players", v);
        }
        s.set("camera_bounds", floats(&info.camera_bounds));
        if let Some(v) = info.unplayable {
            s.set("unplayable", numbers(&v));
        }
        s.set("playable_width", info.playable_width.to_string());
        s.set("playable_height", info.playable_height.to_string());
        s.set("flags", info.flags.0.to_string());
        if let Some(v) = info.tileset {
            s.set("tileset", byte_text(v.to_byte()));
        }
        if let Some(v) = info.loading_screen_number {
            s.set("loading_screen_number", v.to_string());
        }
        if let Some(v) = &info.loading_screen_path {
            s.set("loading_screen_path", v);
        }
        if let Some(v) = &info.loading_screen_text {
            s.set("loading_screen_text", v);
        }
        if let Some(v) = &info.loading_screen_title {
            s.set("loading_screen_title", v);
        }
        if let Some(v) = &info.loading_screen_subtitle {
            s.set("loading_screen_subtitle", v);
        }
        if let Some(v) = info.game_data_set {
            s.set("game_data_set", v.to_string());
        }
        if let Some(v) = &info.prologue_screen_path {
            s.set("prologue_screen_path", v);
        }
        if let Some(v) = &info.prologue_screen_text {
            s.set("prologue_screen_text", v);
        }
        if let Some(v) = &info.prologue_screen_title {
            s.set("prologue_screen_title", v);
        }
        if let Some(v) = &info.prologue_screen_subtitle {
            s.set("prologue_screen_subtitle", v);
        }
        if let Some(v) = info.global_weather {
            s.set("global_weather", fourcc_text(v));
        }
        if let Some(v) = &info.sound_environment {
            s.set("sound_environment", v);
        }
        if let Some(v) = info.light_environment {
            s.set("light_environment", v.to_string());
        }
        if let Some(v) = info.script_language {
            s.set("script_language", v.to_string());
        }
        if let Some(v) = info.graphics_modes {
            s.set("graphics_modes", v.to_string());
        }
        if let Some(v) = info.game_data_version {
            s.set("game_data_version", v.to_string());
        }
    }

    if let Some(fog) = &info.fog {
        let s = doc.push("fog");
        s.set("style", fog.style.to_string());
        s.set("start_height", float_text(fog.start_height));
        s.set("end_height", float_text(fog.end_height));
        s.set("density", float_text(fog.density));
        s.set("color", byte_list(&fog.color));
    }
    if let Some(tint) = &info.water_tint {
        let s = doc.push("water_tint");
        s.set("r", tint.r.to_string());
        s.set("g", tint.g.to_string());
        s.set("b", tint.b.to_string());
        s.set("a", tint.a.to_string());
    }

    // Bytes this build does not decode, written out rather than dropped.
    if !info.legacy_pre_camera.is_empty()
        || !info.legacy_post_size.is_empty()
        || !info.legacy_post_data_version.is_empty()
    {
        let s = doc.push("legacy");
        if !info.legacy_pre_camera.is_empty() {
            s.set("pre_camera", hex(&info.legacy_pre_camera));
        }
        if !info.legacy_post_size.is_empty() {
            s.set("post_size", hex(&info.legacy_post_size));
        }
        if !info.legacy_post_data_version.is_empty() {
            s.set("post_data_version", hex(&info.legacy_post_data_version));
        }
    }

    let sections = info.trailing_sections;
    // A marker section means "the file carried this one", which an empty list
    // cannot express on its own.
    if sections.players {
        doc.push("players");
        for p in &info.players {
            let s = doc.push("player");
            s.set("slot", p.slot.to_string());
            s.set("player_type", p.player_type.to_string());
            s.set("race", p.race.to_string());
            s.set("fixed_start_position", p.fixed_start_position.to_string());
            s.set("name", &p.name);
            s.set("start_x", float_text(p.start_x));
            s.set("start_y", float_text(p.start_y));
            s.set("ally_low_priority", p.ally_low_priority.to_string());
            s.set("ally_high_priority", p.ally_high_priority.to_string());
            if let Some(v) = p.enemy_low_priority {
                s.set("enemy_low_priority", v.to_string());
            }
            if let Some(v) = p.enemy_high_priority {
                s.set("enemy_high_priority", v.to_string());
            }
        }
    }
    if sections.forces {
        doc.push("forces");
        for f in &info.forces {
            let s = doc.push("force");
            s.set("flags", f.flags.to_string());
            s.set("players", f.players.to_string());
            s.set("name", &f.name);
        }
    }
    if sections.upgrades {
        doc.push("upgrades");
        for u in &info.upgrades {
            let s = doc.push("upgrade");
            s.set("players", u.players.to_string());
            s.set("id", fourcc_text(u.id));
            s.set("level", u.level.to_string());
            s.set("state", u.state.to_i32().to_string());
        }
    }
    if sections.tech {
        let s = doc.push("tech");
        let ids: Vec<String> = info.tech.iter().map(|id| fourcc_text(*id)).collect();
        s.set("ids", ids.join(" "));
    }
    if sections.random_units {
        doc.push("random_units");
        for table in &info.random_units {
            let s = doc.push("random_unit");
            s.set("group", table.group.to_string());
            s.set("name", &table.name);
            s.set("columns", numbers(&table.column_types));
            for (index, row) in table.rows.iter().enumerate() {
                let mut line = vec![row.chance.to_string()];
                line.extend(row.ids.iter().map(|id| fourcc_text(*id)));
                s.set(format!("row{index}"), line.join(" "));
            }
        }
    }
    if sections.random_items {
        doc.push("random_items");
        for table in &info.random_items {
            let s = doc.push("random_item");
            s.set("number", table.number.to_string());
            s.set("name", &table.name);
            for (index, set) in table.sets.iter().enumerate() {
                let mut line = Vec::new();
                for item in &set.items {
                    line.push(item.chance.to_string());
                    line.push(fourcc_text(item.id));
                }
                s.set(format!("set{index}"), line.join(" "));
            }
        }
    }

    if !info.trailing.is_empty() {
        let s = doc.push("trailing");
        s.set("bytes", hex(&info.trailing));
    }
    doc
}

/// Reads a map-info model back from a document.
///
/// # Errors
///
/// A missing `[info]` section or a missing key inside it, an unparsable number, a
/// malformed id or hex blob. Everything else a document could get wrong — a
/// mistyped section name, a row with the wrong number of tokens — is reported by
/// [`MapInfo::to_bytes`]'s caller instead, because that is where the bytes are
/// compared with the original.
pub fn from_document(doc: &Document) -> Result<MapInfo> {
    let s = doc.require_section("info")?;
    let fog = doc.section("fog").map(read_fog).transpose()?;
    let water_tint = doc.section("water_tint").map(read_water_tint).transpose()?;
    let (legacy_pre_camera, legacy_post_size, legacy_post_data_version) =
        match doc.section("legacy") {
            Some(legacy) => (
                optional_hex(legacy, "pre_camera")?,
                optional_hex(legacy, "post_size")?,
                optional_hex(legacy, "post_data_version")?,
            ),
            None => (Vec::new(), Vec::new(), Vec::new()),
        };

    let mut sections = TrailingSections::default();
    let mut players = Vec::new();
    for section in doc.sections_named("player") {
        players.push(read_player(section)?);
    }
    if doc.section("players").is_some() {
        sections.players = true;
    }

    let mut forces = Vec::new();
    for section in doc.sections_named("force") {
        forces.push(Force {
            flags: section.u32("flags")?,
            players: section.u32("players")?,
            name: section.require("name")?.to_string(),
        });
    }
    if doc.section("forces").is_some() {
        sections.forces = true;
    }

    let mut upgrades = Vec::new();
    for section in doc.sections_named("upgrade") {
        upgrades.push(Upgrade {
            players: section.u32("players")?,
            id: parse_fourcc(section.require("id")?)?,
            level: section.i32("level")?,
            state: UpgradeState::from(section.i32("state")?),
        });
    }
    if doc.section("upgrades").is_some() {
        sections.upgrades = true;
    }

    let mut tech = Vec::new();
    if let Some(section) = doc.section("tech") {
        for token in section.require("ids")?.split_whitespace() {
            tech.push(parse_fourcc(token)?);
        }
        sections.tech = true;
    }

    let mut random_units = Vec::new();
    for section in doc.sections_named("random_unit") {
        let mut rows = Vec::new();
        for (key, value) in &section.entries {
            if !key.starts_with("row") {
                continue;
            }
            let mut tokens = value.split_whitespace();
            let chance = parse_i32(tokens.next(), "row chance")?;
            let mut ids = Vec::new();
            for token in tokens {
                ids.push(parse_fourcc(token)?);
            }
            rows.push(RandomUnitRow { chance, ids });
        }
        random_units.push(RandomUnitTable {
            group: section.i32("group")?,
            name: section.require("name")?.to_string(),
            column_types: parse_numbers(section.require("columns")?)?,
            rows,
        });
    }
    if doc.section("random_units").is_some() {
        sections.random_units = true;
    }

    let mut random_items = Vec::new();
    for section in doc.sections_named("random_item") {
        let mut sets = Vec::new();
        for (key, value) in &section.entries {
            if !key.starts_with("set") {
                continue;
            }
            let tokens: Vec<&str> = value.split_whitespace().collect();
            if tokens.len() % 2 != 0 {
                return Err(Error::msg(format!(
                    "[random_item] {key} has {} tokens; item entries come in chance/id pairs",
                    tokens.len()
                )));
            }
            let mut items = Vec::new();
            for pair in tokens.chunks(2) {
                items.push(RandomItemEntry {
                    chance: parse_i32(Some(pair[0]), "item chance")?,
                    id: parse_fourcc(pair[1])?,
                });
            }
            sets.push(RandomItemSet { items });
        }
        random_items.push(RandomItemTable {
            number: section.i32("number")?,
            name: section.require("name")?.to_string(),
            sets,
        });
    }
    if doc.section("random_items").is_some() {
        sections.random_items = true;
    }

    let trailing = match doc.section("trailing") {
        Some(section) => section.hex("bytes")?,
        None => Vec::new(),
    };

    let bounds = parse_floats(s.require("camera_bounds")?)?;
    let camera_bounds: [f32; 8] = bounds.try_into().map_err(|v: Vec<f32>| {
        Error::msg(format!("camera_bounds has {} values, not 8", v.len()))
    })?;

    Ok(MapInfo {
        format_version: s.i32("version")?,
        save_count: s.opt_i32("save_count")?,
        editor_version: s.opt_i32("editor_version")?,
        game_version: match s.get("game_version") {
            Some(_) => Some(parse_fixed::<i32, 4>(
                s.require("game_version")?,
                "game_version",
            )?),
            None => None,
        },
        name: s.require("name")?.to_string(),
        author: s.require("author")?.to_string(),
        description: s.require("description")?.to_string(),
        recommended_players: s.opt_string("recommended_players"),
        camera_bounds,
        unplayable: match s.get("unplayable") {
            Some(_) => Some(parse_fixed::<i32, 4>(
                s.require("unplayable")?,
                "unplayable",
            )?),
            None => None,
        },
        playable_width: s.i32("playable_width")?,
        playable_height: s.i32("playable_height")?,
        flags: MapFlags(s.u32("flags")?),
        tileset: match s.get("tileset") {
            Some(text) => Some(Tileset::from_byte(parse_byte(text)?)),
            None => None,
        },
        loading_screen_number: s.opt_i32("loading_screen_number")?,
        loading_screen_path: s.opt_string("loading_screen_path"),
        loading_screen_text: s.opt_string("loading_screen_text"),
        loading_screen_title: s.opt_string("loading_screen_title"),
        loading_screen_subtitle: s.opt_string("loading_screen_subtitle"),
        game_data_set: s.opt_i32("game_data_set")?,
        prologue_screen_path: s.opt_string("prologue_screen_path"),
        prologue_screen_text: s.opt_string("prologue_screen_text"),
        prologue_screen_title: s.opt_string("prologue_screen_title"),
        prologue_screen_subtitle: s.opt_string("prologue_screen_subtitle"),
        fog,
        global_weather: match s.get("global_weather") {
            Some(text) => Some(parse_fourcc(text)?),
            None => None,
        },
        sound_environment: s.opt_string("sound_environment"),
        light_environment: match s.get("light_environment") {
            Some(_) => Some(parse_byte(s.require("light_environment")?)?),
            None => None,
        },
        water_tint,
        script_language: s.opt_i32("script_language")?,
        graphics_modes: s.opt_i32("graphics_modes")?,
        game_data_version: s.opt_i32("game_data_version")?,
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
        diagnostics: war3_core::diag::Diagnostics::new(),
    })
}

fn read_fog(section: &Section) -> Result<Fog> {
    let color = parse_bytes(section.require("color")?)?;
    let color: [u8; 4] = color
        .try_into()
        .map_err(|v: Vec<u8>| Error::msg(format!("fog color has {} values, not 4", v.len())))?;
    Ok(Fog {
        style: section.i32("style")?,
        start_height: section.f32("start_height")?,
        end_height: section.f32("end_height")?,
        density: section.f32("density")?,
        color,
    })
}

fn read_water_tint(section: &Section) -> Result<WaterTint> {
    Ok(WaterTint {
        r: parse_byte(section.require("r")?)?,
        g: parse_byte(section.require("g")?)?,
        b: parse_byte(section.require("b")?)?,
        a: parse_byte(section.require("a")?)?,
    })
}

fn read_player(section: &Section) -> Result<Player> {
    Ok(Player {
        slot: section.i32("slot")?,
        player_type: section.i32("player_type")?,
        race: section.i32("race")?,
        fixed_start_position: section.i32("fixed_start_position")?,
        name: section.require("name")?.to_string(),
        start_x: section.f32("start_x")?,
        start_y: section.f32("start_y")?,
        ally_low_priority: section.u32("ally_low_priority")?,
        ally_high_priority: section.u32("ally_high_priority")?,
        enemy_low_priority: section
            .get("enemy_low_priority")
            .map(|_| section.u32("enemy_low_priority"))
            .transpose()?,
        enemy_high_priority: section
            .get("enemy_high_priority")
            .map(|_| section.u32("enemy_high_priority"))
            .transpose()?,
    })
}

// ---------------------------------------------------------------------------
// Small conversions
// ---------------------------------------------------------------------------

fn numbers<T: std::fmt::Display>(values: &[T]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

fn byte_list(values: &[u8]) -> String {
    numbers(values)
}

/// A byte as text: the character when it is a letter, else hex.
///
/// Letters only, deliberately: a digit has to mean the decimal number, or `5`
/// could be either 5 or the character `'5'` and one of the two would be wrong.
fn byte_text(value: u8) -> String {
    if value.is_ascii_alphabetic() {
        (value as char).to_string()
    } else {
        format!("0x{value:02X}")
    }
}

fn parse_byte(text: &str) -> Result<u8> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x") {
        return u8::from_str_radix(hex, 16)
            .map_err(|_| Error::msg(format!("{text:?} is not a byte")));
    }
    if let Ok(value) = text.parse::<u8>() {
        return Ok(value);
    }
    let mut bytes = text.bytes();
    match (bytes.next(), bytes.next()) {
        (Some(b), None) if b.is_ascii_alphabetic() => Ok(b),
        _ => Err(Error::msg(format!(
            "{text:?} is not a decimal byte, a letter or a 0x byte"
        ))),
    }
}

/// A list of floats, each exact.
fn floats(values: &[f32]) -> String {
    values
        .iter()
        .map(|v| float_text(*v))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_floats(text: &str) -> Result<Vec<f32>> {
    text.split_whitespace().map(parse_float).collect()
}

fn optional_hex(section: &Section, key: &str) -> Result<Vec<u8>> {
    match section.get(key) {
        Some(_) => section.hex(key),
        None => Ok(Vec::new()),
    }
}

fn parse_numbers<T: std::str::FromStr>(text: &str) -> Result<Vec<T>> {
    text.split_whitespace()
        .map(|token| {
            token
                .parse::<T>()
                .map_err(|_| Error::msg(format!("{token:?} is not a number")))
        })
        .collect()
}

/// A fixed-size list, so a field that is an array in the model stays one.
fn parse_fixed<T: std::str::FromStr, const N: usize>(text: &str, what: &str) -> Result<[T; N]> {
    let values: Vec<T> = parse_numbers(text)?;
    values
        .try_into()
        .map_err(|v: Vec<T>| Error::msg(format!("{what} has {} values, not {N}", v.len())))
}

/// `0 1 2` as bytes, for the fog colour.
fn parse_bytes(text: &str) -> Result<Vec<u8>> {
    parse_numbers(text)
}

fn parse_i32(token: Option<&str>, what: &str) -> Result<i32> {
    let token = token.ok_or_else(|| Error::msg(format!("a {what} is missing")))?;
    token
        .parse()
        .map_err(|_| Error::msg(format!("{token:?} is not an integer ({what})")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use war3_core::FourCC;

    /// A version 25 model with one of everything the text form has to carry.
    fn a_model() -> MapInfo {
        MapInfo {
            format_version: 25,
            save_count: Some(71),
            editor_version: Some(6052),
            game_version: None,
            name: "TRIGSTR_010".into(),
            author: "someone".into(),
            description: "a map".into(),
            recommended_players: Some("1v1".into()),
            camera_bounds: [
                -2048.0, -2048.0, 2048.0, 2048.0, -2048.0, -2048.0, 2048.0, 2048.0,
            ],
            unplayable: Some([16, 20, 18, 18]),
            playable_width: 124,
            playable_height: 124,
            flags: MapFlags(0x0000_9C1E),
            tileset: Some(Tileset::from_byte(b'L')),
            loading_screen_number: Some(-1),
            loading_screen_path: Some(String::new()),
            loading_screen_text: Some("text".into()),
            loading_screen_title: Some("title".into()),
            loading_screen_subtitle: Some(String::new()),
            game_data_set: Some(-1),
            prologue_screen_path: Some(String::new()),
            prologue_screen_text: Some(String::new()),
            prologue_screen_title: Some(String::new()),
            prologue_screen_subtitle: Some(String::new()),
            fog: Some(Fog {
                style: 0,
                start_height: 3000.0,
                end_height: -1000.0,
                density: 0.5,
                color: [255, 255, 255, 255],
            }),
            global_weather: Some(FourCC::new(*b"LTlt")),
            sound_environment: Some(String::new()),
            light_environment: Some(b'L'),
            water_tint: Some(WaterTint {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            }),
            script_language: None,
            graphics_modes: None,
            game_data_version: None,
            legacy_pre_camera: Vec::new(),
            legacy_post_size: Vec::new(),
            legacy_post_data_version: Vec::new(),
            trailing: Vec::new(),
            trailing_sections: TrailingSections {
                players: true,
                forces: true,
                upgrades: true,
                tech: true,
                random_units: true,
                random_items: true,
            },
            players: vec![
                Player {
                    slot: 0,
                    player_type: 1,
                    race: 1,
                    fixed_start_position: 2,
                    name: "TRIGSTR_001".into(),
                    start_x: -6144.0,
                    start_y: -6144.0,
                    ally_low_priority: 0,
                    ally_high_priority: 0,
                    enemy_low_priority: None,
                    enemy_high_priority: None,
                },
                Player {
                    slot: 1,
                    player_type: 1,
                    race: 2,
                    fixed_start_position: 0,
                    name: String::new(),
                    start_x: 0.0,
                    start_y: 0.0,
                    ally_low_priority: 1,
                    ally_high_priority: 0,
                    enemy_low_priority: None,
                    enemy_high_priority: None,
                },
            ],
            forces: vec![Force {
                flags: 0,
                players: 0xFFFF,
                name: "force".into(),
            }],
            upgrades: vec![Upgrade {
                players: 1,
                id: FourCC::new(*b"Rhan"),
                level: 2,
                state: UpgradeState::Researched,
            }],
            tech: vec![FourCC::new(*b"A000"), FourCC::new([0x01, 0x02, 0x03, 0x04])],
            random_units: vec![RandomUnitTable {
                group: 0,
                name: "table".into(),
                column_types: vec![-1, 0],
                rows: vec![RandomUnitRow {
                    chance: 100,
                    ids: vec![FourCC::new(*b"hfoo")],
                }],
            }],
            random_items: vec![RandomItemTable {
                number: 3,
                name: "items".into(),
                sets: vec![
                    RandomItemSet {
                        items: vec![RandomItemEntry {
                            chance: 50,
                            id: FourCC::new(*b"ratf"),
                        }],
                    },
                    RandomItemSet { items: Vec::new() },
                ],
            }],
            diagnostics: war3_core::diag::Diagnostics::new(),
        }
    }

    #[test]
    fn the_document_round_trips_through_its_own_text() {
        let doc = to_document(&a_model());
        let rendered = doc.render();
        let parsed = Document::parse(&rendered).unwrap();
        assert_eq!(
            parsed, doc,
            "the text form has to parse back to the same document"
        );
        assert_eq!(
            from_document(&parsed).unwrap().to_bytes(),
            a_model().to_bytes()
        );
    }

    #[test]
    fn absent_and_empty_are_not_confused() {
        let mut info = a_model();
        info.loading_screen_title = None;
        info.loading_screen_text = Some(String::new());
        let doc = Document::parse(&to_document(&info).render()).unwrap();
        let back = from_document(&doc).unwrap();
        assert_eq!(back.loading_screen_title, None);
        assert_eq!(back.loading_screen_text, Some(String::new()));
    }

    #[test]
    fn a_section_with_a_zero_count_is_not_the_same_as_an_absent_one() {
        let mut info = a_model();
        info.players.clear();
        info.trailing_sections.players = true;
        let back = from_document(&Document::parse(&to_document(&info).render()).unwrap()).unwrap();
        assert!(
            back.trailing_sections.players,
            "an empty player list is still a list"
        );

        let mut info = a_model();
        info.trailing_sections.players = false;
        let back = from_document(&Document::parse(&to_document(&info).render()).unwrap()).unwrap();
        assert!(!back.trailing_sections.players);
    }

    #[test]
    fn undecoded_bytes_come_back_as_hex() {
        let mut info = a_model();
        info.legacy_pre_camera = vec![0xDE, 0xAD];
        info.legacy_post_data_version = vec![1, 2, 3, 4];
        info.trailing = vec![0xFF];
        let doc = Document::parse(&to_document(&info).render()).unwrap();
        let back = from_document(&doc).unwrap();
        assert_eq!(back.legacy_pre_camera, vec![0xDE, 0xAD]);
        assert_eq!(back.legacy_post_data_version, vec![1, 2, 3, 4]);
        assert_eq!(back.trailing, vec![0xFF]);
    }

    #[test]
    fn a_non_printable_id_keeps_its_bytes() {
        let id = FourCC::new([0x01, 0x02, 0x03, 0x04]);
        assert_eq!(fourcc_text(id), "0x01020304");
        assert_eq!(parse_fourcc("0x01020304").unwrap(), id);
        assert_eq!(parse_fourcc("A000").unwrap(), FourCC::new(*b"A000"));
        assert!(parse_fourcc("toolong").is_err());
    }

    #[test]
    fn the_codec_handles_only_what_it_has_a_text_form_for() {
        assert!(codec_for("war3map.w3i").is_some());
        assert!(codec_for("WAR3MAP.W3I").is_some());
        assert!(codec_for("war3map.w3e").is_none());
    }
}

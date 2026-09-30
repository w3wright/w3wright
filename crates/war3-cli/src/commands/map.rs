//! `war3 map ...` subcommands.

use std::process::ExitCode;

use war3_core::{Error, Result};
use war3_map::Map;

use crate::cli::required_arg;
use crate::outfmt;

const USAGE: &str = "\
USAGE:
  war3 map info <map> [--verbose]            map information
  war3 map list <map>                        files inside the map
  war3 map file <map> <member>               write one member file
  war3 map terrain <map> [--verbose]         terrain statistics
  war3 map archive <map>                     MPQ archive structure
";

/// Dispatches a `map` subcommand.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let Some(sub) = args.first().map(String::as_str) else {
        eprint!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    let rest = &args[1..];
    match sub {
        "info" => info(rest),
        "list" => list(rest),
        "file" => file(rest),
        "terrain" => terrain(rest),
        "archive" => archive(rest),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => {
            eprintln!("war3 map: unknown subcommand {other:?}");
            eprint!("{USAGE}");
            Ok(ExitCode::from(2))
        }
    }
}

fn verbose(args: &[String]) -> bool {
    args.iter().any(|a| a == "--verbose" || a == "-v")
}

/// `war3 map info`.
fn info(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map info <map>")?;
    let archive = war3_mpq::Archive::open(path)?;
    let map = Map::from_source(&archive)?;

    println!("map   {path}");
    println!("{}", "=".repeat(60));

    outfmt::section("general");
    outfmt::field("name", &map.metadata.name, 20);
    outfmt::field("author", &map.metadata.author, 20);
    if !map.metadata.description.is_empty() {
        outfmt::field("description", &map.metadata.description, 20);
    }
    outfmt::field("recommended", outfmt::opt(map.metadata.recommended_players.as_deref()), 20);

    outfmt::section("versions");
    outfmt::field(".w3i format version", map.metadata.format_version, 20);
    outfmt::field("editor version", outfmt::opt(map.metadata.editor_version), 20);
    outfmt::field("save count", outfmt::opt(map.metadata.save_count), 20);
    outfmt::field(
        "game version",
        map.metadata
            .game_version
            .map_or_else(|| "? (only from v27)".to_string(), outfmt::version),
        20,
    );
    outfmt::field("game data set", outfmt::opt(map.metadata.game_data_set), 20);

    outfmt::section("size");
    outfmt::field(
        "playable area",
        format!("{} x {} tiles", map.metadata.playable_width, map.metadata.playable_height),
        20,
    );
    if let Some([a, b, c, d]) = map.metadata.unplayable {
        outfmt::field("unplayable edges", format!("A={a} B={b} C={c} D={d}"), 20);
    }
    outfmt::field(
        "total size",
        format!("{} x {} tiles", map.metadata.width(), map.metadata.height()),
        20,
    );
    if let Some(t) = &map.terrain {
        outfmt::field("terrain tile points", format!("{} x {}", t.width, t.height), 20);
    }

    outfmt::section("terrain");
    outfmt::field(
        "tileset",
        map.metadata
            .tileset
            .map_or_else(|| "?".to_string(), |t| format!("{} ({})", t, t.to_byte() as char)),
        20,
    );
    outfmt::field(
        "custom tileset",
        outfmt::yesno(map.metadata.flags.has(war3_map::MapFlags::CUSTOM_TILESET)),
        20,
    );
    outfmt::field("melee map", outfmt::yesno(map.metadata.flags.is_melee()), 20);
    outfmt::field(
        "terrain fog",
        outfmt::yesno(map.metadata.flags.has(war3_map::MapFlags::TERRAIN_FOG)),
        20,
    );
    outfmt::field("raw flags", format!("0x{:08X}", map.metadata.flags.0), 20);

    outfmt::section("players");
    if map.metadata.players.is_empty() {
        println!("{}(none)", outfmt::INDENT);
    }
    for p in &map.metadata.players {
        let kind = match p.player_type {
            1 => "user",
            2 => "computer",
            3 => "neutral",
            4 => "reserved",
            _ => "unknown",
        };
        let race = match p.race {
            0 => "random",
            1 => "human",
            2 => "orc",
            3 => "undead",
            4 => "night elf",
            _ => "unknown",
        };
        println!(
            "{}{:>2}  {:<9} {:<10} {:<20} ({:.0}, {:.0}){}",
            outfmt::INDENT,
            p.slot,
            kind,
            race,
            p.name,
            p.start_x,
            p.start_y,
            if p.fixed_start_position { "  [fixed start]" } else { "" }
        );
    }

    if !map.metadata.forces.is_empty() {
        outfmt::section("forces");
        for f in &map.metadata.forces {
            let members: Vec<String> = (0..32)
                .filter(|i| f.players & (1 << i) != 0)
                .map(|i| i.to_string())
                .collect();
            println!(
                "{}{:<20} flags=0x{:04X}  members=[{}]",
                outfmt::INDENT,
                f.name,
                f.flags,
                members.join(", ")
            );
        }
    }

    if !map.metadata.upgrades.is_empty() {
        outfmt::section("custom upgrades");
        for u in &map.metadata.upgrades {
            println!("{}{}  level {}  state {:?}", outfmt::INDENT, u.id, u.level, u.state);
        }
    }

    if !map.metadata.tech.is_empty() {
        outfmt::section("custom tech");
        let ids: Vec<String> = map.metadata.tech.iter().map(|t| t.to_string()).collect();
        println!("{}{}", outfmt::INDENT, ids.join(", "));
    }

    outfmt::section("strings and imports");
    outfmt::field("war3map.wts", format!("{} entries", map.strings.len()), 20);
    outfmt::field("war3map.imp", format!("{} entries", map.imports.len()), 20);

    outfmt::section("members");
    outfmt::field("count", map.file_count(), 20);

    let mut all = map.diagnostics.clone();
    all.extend(archive.diagnostics().items().iter().cloned());
    let has_problems = outfmt::diagnostics(&all, verbose(args));

    Ok(if has_problems { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

/// `war3 map list`.
///
/// The listing comes from the enumeration ladder, so it prints two counts: how
/// many members exist and how many names were worked out. Those differing is
/// normal, since the format does not store names.
fn list(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map list <map>")?;
    let archive = war3_mpq::Archive::open(path)?;
    let info = archive.info();

    println!("map      {path}");
    println!(
        "archive  MPQ v{}  header at {}  sector {} B  block table {} entries",
        info.header.format_version,
        info.header.file_offset,
        info.header.sector_size(),
        info.header.block_table_size
    );
    println!(
        "members  {} block entries in use, {} names enumerated{}",
        archive.used_block_count(),
        archive.file_count(),
        if archive.used_block_count() != archive.file_count() {
            "  <- members without names (the format does not store them)"
        } else {
            ""
        }
    );
    println!();

    let mut names = archive.file_names();
    names.sort_unstable();
    for name in names {
        let block = archive
            .block_index_for_name(name)
            .and_then(|i| archive.block_entry(i as usize).copied());
        let flags = match block {
            Some(b) => format!(
                "{}  stored {}  {}",
                b.flags,
                outfmt::bytes(b.compressed_size as usize),
                b.compression_name()
            ),
            None => "?".to_string(),
        };
        // Reading each member yields its decompressed size. A failure is printed
        // with its reason rather than skipped.
        match archive.read_file(name) {
            Ok(bytes) => println!(
                "{}{:<32} {:>10}  {}",
                outfmt::INDENT,
                name,
                outfmt::bytes(bytes.len()),
                flags
            ),
            Err(e) => println!("{}{:<32} {:>10}  ({e})", outfmt::INDENT, name, "?"),
        }
    }

    let has_problems = outfmt::diagnostics(archive.diagnostics(), verbose(args));
    Ok(if has_problems { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

/// `war3 map file`: writes one member to standard output.
fn file(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map file <map> <member>")?;
    let name = required_arg(args, 1, "a member name", "war3 map file <map> <member>")?;

    let archive = war3_mpq::Archive::open(path)?;
    let bytes = archive
        .read_file(name)
        .map_err(|e| Error::msg(format!("could not read {name:?}: {e}")))?;

    use std::io::Write;
    std::io::stdout().write_all(&bytes)?;
    Ok(ExitCode::SUCCESS)
}

/// `war3 map terrain`.
fn terrain(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map terrain <map>")?;
    let archive = war3_mpq::Archive::open(path)?;
    let bytes = archive
        .read_file("war3map.w3e")
        .map_err(|e| Error::msg(format!("no readable war3map.w3e in this map: {e}")))?;
    let terrain = war3_terrain::Terrain::parse(&bytes)?;

    println!("terrain  {path}");
    println!("{}", "=".repeat(60));

    outfmt::section("layout");
    outfmt::field("on-disk version", terrain.version, 22);
    outfmt::field(
        "record size",
        format!("{} bytes per tile point", terrain.version.record_size()),
        22,
    );
    outfmt::field(
        "ground texture bits",
        format!("{} bits", terrain.version.texture_bits()),
        22,
    );
    outfmt::field("addressable textures", terrain.version.max_textures(), 22);
    outfmt::field("tile points", format!("{} x {}", terrain.width, terrain.height), 22);
    outfmt::field(
        "playable tiles",
        format!("{} x {}", terrain.tile_width(), terrain.tile_height()),
        22,
    );
    outfmt::field("tile point count", terrain.tiles.len(), 22);
    outfmt::field("tileset", terrain.tileset, 22);
    outfmt::field("custom tileset", outfmt::yesno(terrain.uses_custom_tileset), 22);
    outfmt::field(
        "centre offset",
        format!("({:.1}, {:.1})", terrain.center_offset_x, terrain.center_offset_y),
        22,
    );
    let (ox, oy) = terrain.world_origin();
    outfmt::field("south-west corner", format!("({ox:.1}, {oy:.1})"), 22);

    outfmt::section("textures");
    outfmt::field("ground textures", terrain.ground_textures.len(), 22);
    if !terrain.ground_textures.is_empty() {
        let ids: Vec<String> = terrain
            .ground_textures
            .iter()
            .take(terrain.version.max_textures() as usize)
            .map(|t| t.to_string())
            .collect();
        println!("{}addressable: {}", outfmt::INDENT, ids.join(" "));
    }
    outfmt::field("cliff textures", terrain.cliff_textures.len(), 22);
    let used = terrain.used_ground_textures();
    outfmt::field(
        "referenced",
        format!(
            "{} of them [{}]",
            used.len(),
            used.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
        ),
        22,
    );

    outfmt::section("heights");
    let (min, max) = terrain.world_height_range();
    outfmt::field("world range", format!("{min:.1} .. {max:.1} world units"), 22);
    outfmt::field("in tiles", format!("{:.2} .. {:.2}", min / 128.0, max / 128.0), 22);

    outfmt::section("layer height histogram");
    let hist = terrain.layer_histogram();
    for (level, count) in hist.iter().enumerate() {
        if *count > 0 {
            let bar = "#".repeat((*count * 40 / terrain.tiles.len().max(1)).max(1));
            println!("{}layer {:>2}  {:>8}  {}", outfmt::INDENT, level, count, bar);
        }
    }

    outfmt::section("flags");
    let counts = terrain.flag_counts();
    let total = terrain.tiles.len().max(1);
    let pct = |n: usize| format!("{n} ({:.1}%)", n as f64 * 100.0 / total as f64);
    outfmt::field("ramp", pct(counts.ramp), 22);
    outfmt::field("blight", pct(counts.blight), 22);
    outfmt::field("water", pct(counts.water), 22);
    outfmt::field("boundary (camera)", pct(counts.boundary), 22);
    outfmt::field("map edge (0x4000)", pct(counts.boundary_1), 22);

    let has_problems = outfmt::diagnostics(&terrain.diagnostics, verbose(args));
    Ok(if has_problems { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

/// `war3 map archive`: prints the container structure without parsing the map.
fn archive(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map archive <map>")?;
    let archive = war3_mpq::Archive::open(path)?;
    let info = archive.info();
    let h = &info.header;

    println!("archive  {path}");

    outfmt::section("MPQ header");
    outfmt::field("header offset", format!("{} (0x{:X})", h.file_offset, h.file_offset), 22);
    outfmt::field("header size", h.header_size, 22);
    outfmt::field(
        "archive size",
        format!("{} ({})", outfmt::bytes(h.archive_size as usize), h.archive_size),
        22,
    );
    outfmt::field("format version", h.format_version, 22);
    outfmt::field("sector size shift", h.sector_size_shift, 22);
    outfmt::field("sector size", format!("{} bytes", h.sector_size()), 22);
    outfmt::field("hash table offset", format!("0x{:X}", h.hash_table_pos), 22);
    outfmt::field("block table offset", format!("0x{:X}", h.block_table_pos), 22);
    outfmt::field("hash table size", h.hash_table_size, 22);
    outfmt::field("block table size", h.block_table_size, 22);

    outfmt::section("block table");
    println!(
        "{}{:>4}  {:>10}  {:>10}  {:>10}  {:>7}  {:<36}  flags",
        outfmt::INDENT, "#", "offset", "stored", "unpacked", "method", "method name"
    );
    for i in 0..h.block_table_size as usize {
        if let Some(b) = archive.block_entry(i) {
            if b.uncompressed_size == 0 {
                continue;
            }
            println!(
                "{}{:>4}  0x{:08X}  {:>10}  {:>10}    0x{:03X}  {:<36}  {}",
                outfmt::INDENT,
                i,
                b.file_pos,
                b.compressed_size,
                b.uncompressed_size,
                b.compression_method_bits(),
                b.compression_name(),
                b.flags
            );
        }
    }

    outfmt::section("summary");
    outfmt::field("block entries in use", archive.used_block_count(), 22);
    outfmt::field("names enumerated", archive.file_count(), 22);

    let has_problems = outfmt::diagnostics(archive.diagnostics(), verbose(args));
    Ok(if has_problems { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

//! `war3 map ...` subcommands.

use std::process::ExitCode;

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};
use war3_core::{Error, Result};
use war3_map::{Map, MapSource};

use crate::cli::required_arg;
use crate::outfmt;

const USAGE: &str = "\
USAGE:
  war3 map info <map> [--verbose]            map information
  war3 map list <map>                        files inside the map
  war3 map file <map> <member>               write one member file
  war3 map terrain <map> [--verbose]         terrain statistics
  war3 map doodads <map>                     placed doodads
  war3 map units <map>                       placed units and items
  war3 map objects <map> [--game-dir <dir>]  object data, named via the metadata
                                             when a game directory is given
  war3 map archive <map>                     MPQ archive structure
  war3 map rebuild <in> <out> [options]      rewrite the archive with our writer
                                             --stored       re-encode members we can decode
                                             --drop-unnamed drop members that have no name
                                             --no-verify    skip reading the result back
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
        "doodads" => doodads(rest),
        "units" => units(rest),
        "objects" => objects(rest),
        "archive" => archive(rest),
        "rebuild" => rebuild(rest),
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
    let archive = war3_archive::Archive::open(path)?;
    let map = Map::from_source(&archive)?;

    println!("map   {path}");
    println!("{}", "=".repeat(60));

    outfmt::section("general");
    outfmt::field("name", &map.metadata.name, 20);
    outfmt::field("author", &map.metadata.author, 20);
    if !map.metadata.description.is_empty() {
        outfmt::field("description", &map.metadata.description, 20);
    }
    outfmt::field(
        "recommended",
        outfmt::opt(map.metadata.recommended_players.as_deref()),
        20,
    );

    outfmt::section("versions");
    outfmt::field(".w3i format version", map.metadata.format_version, 20);
    outfmt::field(
        "editor version",
        outfmt::opt(map.metadata.editor_version),
        20,
    );
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
        format!(
            "{} x {} tiles",
            map.metadata.playable_width, map.metadata.playable_height
        ),
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
        outfmt::field(
            "terrain tile points",
            format!("{} x {}", t.width, t.height),
            20,
        );
    }

    outfmt::section("placement");
    // "the map has no such member" and "the member would not parse" are
    // different facts, and the second one has a diagnostic explaining it.
    let describe = |member: &str, count: Option<usize>| match count {
        Some(n) => n.to_string(),
        None => {
            let failed = map
                .diagnostics
                .items()
                .iter()
                .any(|d| d.message.contains(member));
            if failed {
                "unreadable, see diagnostics".to_string()
            } else {
                "none".to_string()
            }
        }
    };
    outfmt::field("doodads", describe("war3map.doo", map.doodad_count()), 20);
    outfmt::field(
        "units and items",
        describe("war3mapUnits.doo", map.unit_count()),
        20,
    );

    outfmt::section("terrain");
    outfmt::field(
        "tileset",
        map.metadata.tileset.map_or_else(
            || "?".to_string(),
            |t| format!("{} ({})", t, t.to_byte() as char),
        ),
        20,
    );
    outfmt::field(
        "custom tileset",
        outfmt::yesno(map.metadata.flags.has(war3_map::MapFlags::CUSTOM_TILESET)),
        20,
    );
    outfmt::field(
        "melee map",
        outfmt::yesno(map.metadata.flags.is_melee()),
        20,
    );
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
            if p.fixed_start_position {
                "  [fixed start]"
            } else {
                ""
            }
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
            println!(
                "{}{}  level {}  state {:?}",
                outfmt::INDENT,
                u.id,
                u.level,
                u.state
            );
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

    Ok(if has_problems {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `war3 map list`.
///
/// The listing comes from the enumeration ladder, so it prints two counts: how
/// many members exist and how many names were worked out. Those differing is
/// normal, since the format does not store names.
fn list(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map list <map>")?;
    let archive = war3_archive::Archive::open(path)?;
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
    Ok(if has_problems {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `war3 map file`: writes one member to standard output.
fn file(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map file <map> <member>")?;
    let name = required_arg(args, 1, "a member name", "war3 map file <map> <member>")?;

    let archive = war3_archive::Archive::open(path)?;
    let bytes = archive
        .read_file(name)
        .map_err(|e| Error::msg(format!("could not read {name:?}: {e}")))?;

    use std::io::Write;
    std::io::stdout().write_all(&bytes)?;
    Ok(ExitCode::SUCCESS)
}

/// What a member should be compared by, when checking a rebuild.
///
/// Decoded content when this workspace can decode it; otherwise the stored
/// bytes, because a member that cannot be decoded has nothing else to compare —
/// and it is exactly the member that must survive untouched.
fn comparable(archive: &war3_archive::Archive, name: &str) -> std::result::Result<Vec<u8>, String> {
    match archive.read_file(name) {
        Ok(data) => Ok(data),
        Err(_) => archive
            .raw_member(name)
            .map(|m| m.block)
            .map_err(|e| e.to_string()),
    }
}

/// `war3 map rebuild`: rewrites an archive with this workspace's writer.
///
/// This is the container half of the build direction, and the check the
/// prototype is judged by: a map that survives read → write → read unchanged.
/// Nothing here understands any `war3map.*` format; it moves blocks.
fn rebuild(args: &[String]) -> Result<ExitCode> {
    const USAGE: &str = "war3 map rebuild <in> <out> [--stored] [--drop-unnamed] [--no-verify]";
    let input = required_arg(args, 0, "an input path", USAGE)?;
    let output = required_arg(args, 1, "an output path", USAGE)?;
    let reencode = args.iter().any(|a| a == "--stored");
    let drop_unnamed = args.iter().any(|a| a == "--drop-unnamed");
    let verify = !args.iter().any(|a| a == "--no-verify");

    let source = war3_archive::Archive::open(input)?;
    let mut diagnostics = Diagnostics::new();

    // A member is identified by the hash of its name and by nothing else, so a
    // block whose name never surfaced cannot be written back. Refusing is the
    // only honest option: the alternative is an archive that quietly lost part
    // of the map.
    let named = source.file_count();
    let used = source.used_block_count();
    if named < used {
        let lost = used - named;
        if !drop_unnamed {
            return Err(Error::msg(format!(
                "{used} blocks are in use but only {named} names could be recovered, so rebuilding \
                 would drop {lost} member(s); pass --drop-unnamed to accept that"
            )));
        }
        diagnostics.push(Diagnostic::warn(
            DiagnosticCode::MpqNoListfile,
            format!("{lost} member(s) have no recoverable name and were dropped"),
        ));
    }

    let mut builder = war3_archive::ArchiveBuilder::with_prefix(source.prefix().to_vec())?;
    let mut names = source.file_names();
    names.sort_unstable();
    let count = names.len();

    let (mut verbatim, mut reencoded) = (0usize, 0usize);
    for name in &names {
        let raw = source.raw_member(name)?;
        if !reencode {
            builder.add_raw(raw)?;
            verbatim += 1;
            continue;
        }
        match source.read_file(name) {
            Ok(data) => {
                builder.add_stored(*name, data);
                reencoded += 1;
            }
            Err(e) => {
                // This workspace cannot decode it, so it goes back byte for byte
                // rather than being dropped or guessed at.
                diagnostics.push(Diagnostic::warn(
                    DiagnosticCode::MpqUnsupportedCompression,
                    format!("{name}: kept verbatim, {e}"),
                ));
                builder.add_raw(raw)?;
                verbatim += 1;
            }
        }
    }

    builder.write(output)?;
    let written = std::fs::metadata(output)
        .map(|m| m.len() as usize)
        .unwrap_or(0);

    println!("rebuild  {input} -> {output}");
    println!("{}", "=".repeat(60));
    outfmt::field("members", builder.member_count(), 22);
    outfmt::field(
        if reencode {
            "verbatim"
        } else {
            "copied verbatim"
        },
        verbatim,
        22,
    );
    if reencode {
        outfmt::field("re-encoded", reencoded, 22);
    }
    outfmt::field("prefix", format!("{} bytes", source.prefix().len()), 22);
    outfmt::field("output size", outfmt::bytes(written), 22);

    let mut mismatched = Vec::new();
    if verify {
        outfmt::section("verify");
        let rebuilt = war3_archive::Archive::open(output)?;
        // The prefix is what the game reads the map's name and flags out of, so
        // a rebuild has to reproduce it exactly.
        if rebuilt.prefix() != source.prefix() {
            mismatched.push(format!(
                "prefix: {} bytes vs {} bytes",
                source.prefix().len(),
                rebuilt.prefix().len()
            ));
        }
        for name in &names {
            let before = comparable(&source, name);
            let after = comparable(&rebuilt, name);
            match (before, after) {
                (Ok(a), Ok(b)) if a == b => {}
                (Ok(a), Ok(b)) => mismatched.push(format!(
                    "{name}: {a_len} vs {b_len} bytes differ",
                    a_len = a.len(),
                    b_len = b.len()
                )),
                (a, b) => mismatched.push(format!("{name}: {a:?} vs {b:?}")),
            }
        }
        if mismatched.is_empty() {
            outfmt::field("identical", format!("{count} of {count} members"), 22);
        } else {
            outfmt::field(
                "identical",
                format!("{} of {count}", count - mismatched.len()),
                22,
            );
            for line in &mismatched {
                println!("{}MISMATCH {line}", outfmt::INDENT);
            }
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::MpqHashCollisionSkipped,
                format!("{} member(s) did not survive the rebuild", mismatched.len()),
            ));
        }
    }

    let has_problems = outfmt::diagnostics(&diagnostics, verbose(args));
    Ok(if has_problems || !mismatched.is_empty() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `war3 map terrain`.
fn terrain(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map terrain <map>")?;
    let archive = war3_archive::Archive::open(path)?;
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
    outfmt::field(
        "tile points",
        format!("{} x {}", terrain.width, terrain.height),
        22,
    );
    outfmt::field(
        "playable tiles",
        format!("{} x {}", terrain.tile_width(), terrain.tile_height()),
        22,
    );
    outfmt::field("tile point count", terrain.tiles.len(), 22);
    outfmt::field("tileset", terrain.tileset, 22);
    outfmt::field(
        "custom tileset",
        outfmt::yesno(terrain.uses_custom_tileset),
        22,
    );
    outfmt::field(
        "centre offset",
        format!(
            "({:.1}, {:.1})",
            terrain.center_offset_x, terrain.center_offset_y
        ),
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
            used.iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        22,
    );

    outfmt::section("heights");
    let (min, max) = terrain.world_height_range();
    outfmt::field(
        "world range",
        format!("{min:.1} .. {max:.1} world units"),
        22,
    );
    outfmt::field(
        "in tiles",
        format!("{:.2} .. {:.2}", min / 128.0, max / 128.0),
        22,
    );

    outfmt::section("layer height histogram");
    let hist = terrain.layer_histogram();
    for (level, count) in hist.iter().enumerate() {
        if *count > 0 {
            let bar = "#".repeat((*count * 40 / terrain.tiles.len().max(1)).max(1));
            println!(
                "{}layer {:>2}  {:>8}  {}",
                outfmt::INDENT,
                level,
                count,
                bar
            );
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
    Ok(if has_problems {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `war3 map doodads`: the doodads placed on the map.
fn doodads(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map doodads <map>")?;
    let map = Map::from_source(&war3_archive::Archive::open(path)?)?;
    let Some(file) = &map.doodads else {
        // Say why rather than printing an empty list.
        let reason = map
            .diagnostics
            .items()
            .iter()
            .find(|d| d.message.contains("war3map.doo"))
            .map_or_else(
                || "the map has no war3map.doo".to_string(),
                |d| d.message.clone(),
            );
        return Err(Error::msg(format!("doodads unavailable: {reason}")));
    };

    println!("doodads  {path}");
    println!("{}", "=".repeat(60));
    outfmt::field(
        "file version",
        format!("v{}/sub{}", file.version, file.subversion),
        20,
    );
    outfmt::field("records", file.doodads.len(), 20);
    let mut kinds = std::collections::BTreeMap::new();
    for d in &file.doodads {
        *kinds.entry(d.kind.to_string()).or_insert(0usize) += 1;
    }
    outfmt::field("distinct types", kinds.len(), 20);
    outfmt::field("special doodads", file.special.len(), 20);

    outfmt::section("most common types");
    let mut ranked: Vec<_> = kinds.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (kind, count) in ranked.iter().take(12) {
        outfmt::field(kind, count, 20);
    }

    outfmt::section("first records");
    println!(
        "{}{:<6} {:>4}  {:<24} {:<20} {:>4} {:>10}",
        outfmt::INDENT,
        "type",
        "var",
        "position",
        "flags",
        "life",
        "editor"
    );
    for d in file.doodads.iter().take(20) {
        println!(
            "{}{:<6} {:>4}  ({:>8.1},{:>8.1},{:>7.1})  {:<20} {:>3}% {:>10}",
            outfmt::INDENT,
            d.kind,
            d.variation,
            d.position.x,
            d.position.y,
            d.position.z,
            war3_map::DoodadFile::flags_name(d.flags),
            d.life,
            d.editor_id
        );
    }
    if file.doodads.len() > 20 {
        println!("{}... {} more", outfmt::INDENT, file.doodads.len() - 20);
    }

    let has_problems = outfmt::diagnostics(&map.diagnostics, verbose(args));
    Ok(if has_problems {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `war3 map units`: the units and items placed on the map.
fn units(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map units <map>")?;
    let map = Map::from_source(&war3_archive::Archive::open(path)?)?;
    let Some(file) = &map.units else {
        let reason = map
            .diagnostics
            .items()
            .iter()
            .find(|d| d.message.contains("war3mapUnits.doo"))
            .map_or_else(
                || "the map has no war3mapUnits.doo".to_string(),
                |d| d.message.clone(),
            );
        return Err(Error::msg(format!("units unavailable: {reason}")));
    };

    println!("units  {path}");
    println!("{}", "=".repeat(60));
    outfmt::field(
        "file version",
        format!("v{}/sub{}", file.version, file.subversion),
        20,
    );
    outfmt::field("records", file.units.len(), 20);
    // Version 8 always carries the item-table field, so counting its presence
    // would claim every unit has item data. Only a real table index or an
    // inventory counts.
    let placed_items = file
        .units
        .iter()
        .filter(|u| u.item_table.is_some_and(|t| t >= 0) || !u.inventory.is_empty())
        .count();
    let levelled = file.units.iter().filter(|u| u.hero_level > 1).count();
    outfmt::field("levelled units", levelled, 20);
    outfmt::field("with item data", placed_items, 20);

    let mut kinds = std::collections::BTreeMap::new();
    for u in &file.units {
        *kinds.entry(u.kind.to_string()).or_insert(0usize) += 1;
    }
    outfmt::section("most common types");
    let mut ranked: Vec<_> = kinds.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (kind, count) in ranked.iter().take(12) {
        outfmt::field(kind, count, 20);
    }

    outfmt::section("by player");
    let mut players = std::collections::BTreeMap::new();
    for u in &file.units {
        *players.entry(u.player).or_insert(0usize) += 1;
    }
    for (player, count) in &players {
        outfmt::field(&format!("player {player}"), count, 20);
    }

    outfmt::section("first records");
    println!(
        "{}{:<6} {:>7}  {:<24} {:>6} {:>6} {:>6}",
        outfmt::INDENT,
        "type",
        "player",
        "position",
        "hp",
        "mana",
        "level"
    );
    for u in file.units.iter().take(20) {
        println!(
            "{}{:<6} {:>7}  ({:>8.1},{:>8.1},{:>7.1})  {:>6} {:>6} {:>6}",
            outfmt::INDENT,
            u.kind,
            u.player,
            u.position.x,
            u.position.y,
            u.position.z,
            if u.hit_points < 0 {
                "def".to_string()
            } else {
                u.hit_points.to_string()
            },
            if u.mana < 0 {
                "def".to_string()
            } else {
                u.mana.to_string()
            },
            u.hero_level
        );
    }
    if file.units.len() > 20 {
        println!("{}... {} more", outfmt::INDENT, file.units.len() - 20);
    }

    let has_problems = outfmt::diagnostics(&map.diagnostics, verbose(args));
    Ok(if has_problems {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `war3 map objects`: the map's object data, one file per category.
///
/// Field ids are meaningless without Blizzard's metadata tables, which live in
/// the game's archives rather than the map, so `--game-dir` is what turns
/// `uhpm` into a readable name. Without it the ids and values are still shown:
/// a user with no installation must not be left with nothing.
fn objects(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(
        args,
        0,
        "a map path",
        "war3 map objects <map> [--game-dir <dir>]",
    )?;
    let game_dir = args
        .iter()
        .position(|a| a == "--game-dir")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let archive = war3_archive::Archive::open(path)?;
    let metadata = game_dir.as_deref().map(|dir| {
        let mpq = crate::assets::MpqAssetSource::open(dir);
        let loose = war3_core::FileAssetSource::new(dir);
        let layered = crate::assets::LayeredSource::new(&mpq, &loose);
        war3_meta::MetaTableSet::load_from_assets(&layered)
    });
    // String-valued fields whose metadata marks them `stringExt` hold
    // `TRIGSTR_nnn` references, which only the map's own string table can
    // resolve. A map whose `.wts` cannot be read (it may be imploded) simply
    // leaves them as references.
    let strings = archive
        .get("war3map.wts")
        .map(|bytes| war3_map::StringTable::parse(&bytes));

    println!("objects  {path}");
    println!("{}", "=".repeat(60));
    if metadata.is_none() {
        println!(
            "{}no --game-dir given, so field ids are shown unresolved",
            outfmt::INDENT
        );
    }

    let mut diagnostics = Diagnostics::new();
    let mut parsed_any = false;
    for kind in war3_object::ObjectKind::ALL {
        let Some(bytes) = archive.get(kind.map_file()) else {
            continue;
        };
        parsed_any = true;
        let file = match war3_object::ObjectFile::parse(kind, &bytes) {
            Ok(file) => file,
            Err(e) => {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::ObjectUnknownField,
                    format!("{} failed to parse: {e}", kind.map_file()),
                ));
                continue;
            }
        };
        diagnostics.merge(&file.diagnostics);

        let table = &file.table;
        outfmt::section(&format!(
            "{} ({})",
            kind.map_file(),
            format!("{kind:?}").to_lowercase()
        ));
        outfmt::field("version", file.version, 20);
        outfmt::field(
            "objects",
            format!(
                "{} original, {} custom",
                table.original.len(),
                table.custom.len()
            ),
            20,
        );

        if let Some(set) = &metadata {
            table.diagnose_against(
                |id| {
                    let Some(meta_table) = set.get(kind) else {
                        // No metadata for this category, so no judgement to make:
                        // reporting every field as unknown would be a lie.
                        return Some(None);
                    };
                    // The value's stored type is cross-checked once the
                    // vocabulary-to-binary type mapping is settled; for now only
                    // "is this field in the table at all" is answered.
                    if meta_table.get(id).is_some() {
                        Some(None)
                    } else {
                        None
                    }
                },
                &mut diagnostics,
            );
        }

        for object in table.custom.iter().take(6) {
            println!(
                "{}{} <- {}{}",
                outfmt::INDENT,
                object.id,
                object.base_id,
                if object.is_hero(kind) { "  (hero)" } else { "" }
            );
            print_modifications(
                object,
                kind,
                metadata.as_ref(),
                strings.as_ref(),
                &mut diagnostics,
            );
        }
        for object in table.original.iter().take(3) {
            println!("{}{} (modified original)", outfmt::INDENT, object.id);
            print_modifications(
                object,
                kind,
                metadata.as_ref(),
                strings.as_ref(),
                &mut diagnostics,
            );
        }
    }

    if !parsed_any {
        println!("{}no object files in this map", outfmt::INDENT);
    }

    let has_problems = outfmt::diagnostics(&diagnostics, verbose(args));
    Ok(if has_problems {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Prints one object's modifications, naming fields when metadata is available.
fn print_modifications(
    object: &war3_object::Object,
    kind: war3_object::ObjectKind,
    metadata: Option<&war3_meta::MetaTableSet>,
    strings: Option<&war3_map::StringTable>,
    diagnostics: &mut Diagnostics,
) {
    for m in object.modifications.iter().take(12) {
        let meta = metadata
            .and_then(|set| set.get(kind))
            .and_then(|table| table.get(m.field));
        let level = m.level.map(|l| l.level).unwrap_or(0);
        let name = match meta {
            // A levelled field's readable name carries its level, e.g. `Ilif`
            // at level 2 is `Ilif2`.
            Some(meta) if level > 0 => meta.level_field_name(level),
            Some(meta) => meta.field.clone(),
            None => m.field.to_string(),
        };
        // A string field the metadata marks `stringExt` may hold a `TRIGSTR_nnn`
        // reference; the map's own table is what turns it into text.
        let value = match (&m.value, meta) {
            (war3_object::FieldValue::String(text), Some(meta)) if meta.string_ext => {
                strings.map_or_else(|| text.clone(), |table| table.resolve(text, diagnostics))
            }
            _ => m.value.to_string(),
        };
        println!(
            "{}{:<28} = {}{}",
            outfmt::INDENT.repeat(2),
            name,
            value,
            if m.level.is_some() {
                format!("   (level {level})")
            } else {
                String::new()
            }
        );
    }
    if object.modifications.len() > 12 {
        println!(
            "{}... {} more fields",
            outfmt::INDENT.repeat(2),
            object.modifications.len() - 12
        );
    }
}

/// `war3 map archive`: prints the container structure without parsing the map.
fn archive(args: &[String]) -> Result<ExitCode> {
    let path = required_arg(args, 0, "a map path", "war3 map archive <map>")?;
    let archive = war3_archive::Archive::open(path)?;
    let info = archive.info();
    let h = &info.header;

    println!("archive  {path}");

    outfmt::section("MPQ header");
    outfmt::field(
        "header offset",
        format!("{} (0x{:X})", h.file_offset, h.file_offset),
        22,
    );
    outfmt::field("header size", h.header_size, 22);
    outfmt::field(
        "archive size",
        format!(
            "{} ({})",
            outfmt::bytes(h.archive_size as usize),
            h.archive_size
        ),
        22,
    );
    outfmt::field("format version", h.format_version, 22);
    outfmt::field("sector size shift", h.sector_size_shift, 22);
    outfmt::field("sector size", format!("{} bytes", h.sector_size()), 22);
    outfmt::field("hash table offset", format!("0x{:X}", h.hash_table_pos), 22);
    outfmt::field(
        "block table offset",
        format!("0x{:X}", h.block_table_pos),
        22,
    );
    outfmt::field("hash table size", h.hash_table_size, 22);
    outfmt::field("block table size", h.block_table_size, 22);

    outfmt::section("block table");
    println!(
        "{}{:>4}  {:>10}  {:>10}  {:>10}  {:>7}  {:<36}  flags",
        outfmt::INDENT,
        "#",
        "offset",
        "stored",
        "unpacked",
        "method",
        "method name"
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
    Ok(if has_problems {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

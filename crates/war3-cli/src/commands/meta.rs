//! `war3 meta ...` subcommands.
//!
//! Object metadata, object **names** and trigger definitions come from three different places, and
//! all are needed by later phases:
//!
//! | Data | Source | Shipped with the repository |
//! | --- | --- | --- |
//! | mechanical field metadata | the game's `*MetaData.slk` | no, read at runtime |
//! | object display names | `Units\*Strings.txt`, or a `.slk` `Name` column plus `UI\WorldEditStrings.txt` | no, read at runtime |
//! | display text for those fields | `UI\WorldEditStrings.txt` | no, read at runtime |
//! | trigger definitions | `UI\TriggerData.txt` | no, read at runtime |
//!
//! Hence a command that answers "can my installation be found?" before a `.wtg`
//! parse is attempted and fails for an unrelated-looking reason, and one that answers "what is this
//! object called" so the resolution can be checked against a real installation without a GUI.

use std::path::Path;
use std::process::ExitCode;

use war3_core::{AssetSource, Error, Result};
use war3_game::{GameAssets, Resolver};
use war3_meta::{MetaTableSet, ObjectKind, TriggerData};

use crate::cli::required_arg;
use crate::outfmt;

const USAGE: &str = "\
USAGE:
  war3 meta check <game-dir> [--verbose]
      Check whether object metadata, names and trigger definitions can be found.
      <game-dir> should point at the directory holding war3.mpq and War3x.mpq,
      or at an already unpacked tree.
  war3 meta name <game-dir> <id> [<id>...]
      Print what each object id is called, and where the name came from.
      Ids may be prefixed to choose the kind, e.g. 'unit:hfoo' or 'ability:AHhb';
      without a prefix the kind is guessed from the id's shape.
";

/// Dispatches a `meta` subcommand.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let Some(sub) = args.first().map(String::as_str) else {
        eprint!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    match sub {
        "check" => check(&args[1..]),
        "name" => name(&args[1..]),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => {
            eprintln!("war3 meta: unknown subcommand {other:?}");
            eprint!("{USAGE}");
            Ok(ExitCode::from(2))
        }
    }
}

/// Which kind an id belongs to, when the caller did not say.
///
/// # Why guessing is acceptable here and not in a parser
///
/// This decides what to *label* an id on a command line, and the worst case is that the wrong
/// table is consulted and the id comes back unchanged. A file format never does this: `war3-object`
/// knows a member's kind from which file it came from, which is a fact, and this is a convenience
/// for a person typing one id to see what happens.
///
/// The rule used is the one the game's own object ids follow: units and destructables and doodads
/// start with an uppercase letter, abilities with an uppercase one too, and the first character's
/// case distinguishes a hero from a unit only within the unit table. That is not enough to be
/// certain, so the caller can always write `unit:hfoo` and be sure.
fn guess_kind(id: &str) -> ObjectKind {
    // Items are the one kind whose ids are four lower-case characters in this corpus; abilities
    // share the unit convention, so units are the better guess for an unknown id.
    if id.len() == 4 && id.chars().all(|c| c.is_ascii_lowercase()) {
        ObjectKind::Item
    } else {
        ObjectKind::Unit
    }
}

/// Splits `kind:id`, or guesses.
fn parse_query(query: &str) -> (ObjectKind, &str) {
    let Some((prefix, rest)) = query.split_once(':') else {
        return (guess_kind(query), query);
    };
    let kind = ObjectKind::ALL
        .into_iter()
        .find(|k| format!("{k}").eq_ignore_ascii_case(prefix))
        .unwrap_or_else(|| guess_kind(rest));
    (kind, rest)
}

/// `war3 meta name`
fn name(args: &[String]) -> Result<ExitCode> {
    let root = required_arg(
        args,
        0,
        "a game directory",
        "war3 meta name <game-dir> <id>...",
    )?;
    let queries: Vec<&String> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    if queries.is_empty() {
        return Err(Error::msg(
            "war3 meta name needs at least one object id, e.g. 'unit:hfoo'",
        ));
    }
    let path = Path::new(root);
    if !path.is_dir() {
        return Err(Error::msg(format!("{root} is not a directory")));
    }

    let assets = GameAssets::open(path);
    let resolver = Resolver::load(&assets);

    println!("game directory  {}", path.display());
    println!(
        "name tables     {} ids, {} WESTRING keys",
        resolver.stats().total,
        resolver.string_count()
    );
    println!("{}", "=".repeat(60));
    for query in queries {
        let (kind, id) = parse_query(query);
        let got = resolver.resolve(kind, id);
        println!(
            "{:<12} {:<6} -> {:<28} [{}]",
            format!("{kind}"),
            id,
            got.name,
            got.source.label()
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn check(args: &[String]) -> Result<ExitCode> {
    let root = required_arg(args, 0, "a game directory", "war3 meta check <game-dir>")?;
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");

    let path = Path::new(root);
    if !path.is_dir() {
        return Err(Error::msg(format!("{root} is not a directory")));
    }

    // One value holding the whole installation, rather than three whose ordering is the caller's
    // problem. The archives still win over the loose tree; see `war3_game::mpq`.
    let assets = GameAssets::open(path);
    let source = assets.loose();
    println!("game directory  {}", path.display());
    println!(
        "loose files     {} indexed (most game metadata is packed inside MPQ archives)",
        source.file_count()
    );

    // Scanning the directory tree alone is not enough: the metadata, names and trigger
    // definitions normally live inside war3.mpq and War3x.mpq. This chain also exercises the MPQ
    // reader against the game's own archives.
    let mpq = assets.archives();
    println!("MPQ archives    {} opened", mpq.archive_count());
    for (p, names, blocks) in mpq.name_counts() {
        println!(
            "{}  {:<18} {names} names enumerated, {blocks} block entries in use",
            outfmt::INDENT,
            p.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        );
    }
    for (p, why) in mpq.failures() {
        println!(
            "{}{} could not be opened: {why}",
            outfmt::INDENT,
            p.display()
        );
    }
    println!("{}", "=".repeat(60));

    // Archives over loose files, defined once in `war3-game` so this cannot be the place the order
    // is got wrong.
    let layered = assets.layered();

    // ---- mechanical metadata ----
    outfmt::section("object field metadata (mechanical layer, shippable)");
    let meta = MetaTableSet::load_from_assets(&layered);
    for kind in ObjectKind::ALL {
        let rel = format!("{}\\{}", kind.metadata_dir(), kind.metadata_slk());
        match meta.get(kind) {
            Some(table) if !table.is_empty() => {
                println!(
                    "{}{:<14} {:>6} fields   {}",
                    outfmt::INDENT,
                    kind,
                    table.len(),
                    rel
                );
            }
            _ => {
                println!(
                    "{}{:<14} {:>6}          {}  <- missing",
                    outfmt::INDENT,
                    kind,
                    "-",
                    rel
                );
            }
        }
    }

    // The `type` column holds a word, not a code; this is what turns one into
    // the other. Printing the count makes a stale `UnitEditorData.txt` — the
    // one in `war3.mpq` has 12 sections against the patch's 36 — visible.
    println!(
        "{}{:<14} {:>6} words  {} from UnitEditorData.txt sections",
        outfmt::INDENT,
        "type vocabulary",
        meta.types.len(),
        meta.types.from_sections()
    );

    // ---- object names ----
    //
    // The section that makes an ID recognisable. ⚠️ A count of zero for a kind is the useful
    // failure: it is what a wrong path or a missing localisation pack looks like, and nothing else
    // in the program would report it — every object of that kind simply shows its ID.
    outfmt::section("object names (read at runtime, not shipped)");
    let resolver = Resolver::load(&assets);
    for (kind, count) in resolver.stats().per_kind {
        let source_file = match kind {
            ObjectKind::Doodad => "Doodads\\Doodads.slk".to_string(),
            ObjectKind::Destructable => "Units\\DestructableData.slk".to_string(),
            _ => format!("{}\\*Strings.txt", kind.metadata_dir()),
        };
        println!(
            "{}{:<14} {:>6} names  {source_file}",
            outfmt::INDENT,
            kind,
            count
        );
    }
    println!(
        "{}{:<14} {:>6} keys   UI\\WorldEditStrings.txt",
        outfmt::INDENT,
        "WESTRING",
        resolver.string_count()
    );

    // ---- expressive layer ----
    outfmt::section("display text (read at runtime, not shipped)");
    for (label, rel) in [
        ("world editor strings", "UI\\WorldEditStrings.txt"),
        ("unit editor data", "UI\\UnitEditorData.txt"),
        ("trigger definitions", "UI\\TriggerData.txt"),
        ("trigger strings", "UI\\TriggerStrings.txt"),
    ] {
        match layered.get_text(rel) {
            Some(text) => println!(
                "{}{:<22} {:>10}  {}",
                outfmt::INDENT,
                label,
                outfmt::bytes(text.len()),
                rel
            ),
            None => println!(
                "{}{:<22} {:>10}  {}  <- missing",
                outfmt::INDENT,
                label,
                "-",
                rel
            ),
        }
    }

    // ---- trigger definitions ----
    outfmt::section("trigger definitions");
    let triggers = TriggerData::load_from_assets(&layered);
    if triggers.is_usable() {
        println!("{}{}", outfmt::INDENT, triggers);
        println!(
            "{}{} types, {} preset values",
            outfmt::INDENT,
            triggers.types.len(),
            triggers.params.len()
        );
        println!(
            "{}note: the parameter count of a call is not stored in the .wtg file, so it has to be \
             recomputed from these definitions, and parameters of type nothing are skipped",
            outfmt::INDENT
        );
    } else {
        println!("{}{}", outfmt::INDENT, triggers);
        println!(
            "{}without TriggerData.txt, .wtg and .wct cannot be parsed at all",
            outfmt::INDENT
        );
    }

    // ---- diagnostics ----
    let mut all = meta.diagnostics.clone();
    all.merge(&triggers.diagnostics);
    let has_problems = outfmt::diagnostics(&all, verbose);

    outfmt::section("summary");
    println!(
        "{}object field metadata: {}",
        outfmt::INDENT,
        if meta.is_empty() {
            "unavailable (display names degrade to raw identifiers)"
        } else {
            "available"
        }
    );
    println!(
        "{}trigger definitions:  {}",
        outfmt::INDENT,
        if triggers.is_usable() {
            "available"
        } else {
            "unavailable (.wtg and .wct cannot be parsed)"
        }
    );

    Ok(if has_problems {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

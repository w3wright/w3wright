//! `war3 meta ...` subcommands.
//!
//! Object metadata and trigger definitions come from two different places, and
//! both are needed by later phases:
//!
//! | Data | Source | Shipped with the repository |
//! | --- | --- | --- |
//! | mechanical field metadata | the game's `*MetaData.slk` | yes, prebuilt |
//! | display text for those fields | `UI\WorldEditStrings.txt` | no, read at runtime |
//! | trigger definitions | `UI\TriggerData.txt` | no, read at runtime |
//!
//! Hence a command that answers "can my installation be found?" before a `.wtg`
//! parse is attempted and fails for an unrelated-looking reason.

use std::path::Path;
use std::process::ExitCode;

use war3_core::{AssetSource, Error, FileAssetSource, Result};
use war3_meta::{MetaTableSet, ObjectKind, TriggerData};

use crate::assets::MpqAssetSource;
use crate::cli::required_arg;
use crate::outfmt;

/// Two asset chains, archive first and loose files second.
///
/// A dedicated type so that the ordering is defined in exactly one place;
/// reversing it would not fail, it would quietly read a different copy.
#[derive(Debug)]
struct LayeredSource<'a> {
    first: &'a MpqAssetSource,
    second: &'a FileAssetSource,
}

impl<'a> LayeredSource<'a> {
    const fn new(first: &'a MpqAssetSource, second: &'a FileAssetSource) -> Self {
        Self { first, second }
    }
}

impl AssetSource for LayeredSource<'_> {
    fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.first.get(path).or_else(|| self.second.get(path))
    }
}

const USAGE: &str = "\
USAGE:
  war3 meta check <game-dir> [--verbose]
      Check whether object metadata and trigger definitions can be found.
      <game-dir> should point at the directory holding war3.mpq and War3x.mpq,
      or at an already unpacked tree.
";

/// Dispatches a `meta` subcommand.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let Some(sub) = args.first().map(String::as_str) else {
        eprint!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    match sub {
        "check" => check(&args[1..]),
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

fn check(args: &[String]) -> Result<ExitCode> {
    let root = required_arg(args, 0, "a game directory", "war3 meta check <game-dir>")?;
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");

    let path = Path::new(root);
    if !path.is_dir() {
        return Err(Error::msg(format!("{root} is not a directory")));
    }

    let source = FileAssetSource::new(path);
    println!("game directory  {}", source.root().display());
    println!(
        "loose files     {} indexed (most game metadata is packed inside MPQ archives)",
        source.file_count()
    );

    // Scanning the directory tree alone is not enough: the metadata and trigger
    // definitions normally live inside war3.mpq and War3x.mpq. This chain also
    // exercises the MPQ reader against the game's own archives.
    let mpq = MpqAssetSource::open(path);
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

    // Archive first, loose files second.
    let layered = LayeredSource::new(&mpq, &source);

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

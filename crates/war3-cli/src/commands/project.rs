//! `war3 build`: a map from a source project.
//!
//! The format-level counterpart of `war3 map extract`. It is deliberately not
//! `war3 map build`: the input is a project directory, not a map, and Phase 2
//! puts the build orchestration (validate → compile script → pack) in front of
//! this one day.

use std::path::Path;
use std::process::ExitCode;

use war3_core::Result;

use crate::cli::required_arg;
use crate::outfmt;

const USAGE: &str = "\
USAGE:
  war3 build <project-dir> [--out <file>] [--verbose]
                                  build an archive from a source project
                                  --out  write here instead of [build] output
";

const VALIDATE_USAGE: &str = "\
USAGE:
  war3 validate <project-dir>     check a source project without building it
";

/// Dispatches `war3 validate`.
///
/// Read-only. It reports **every** problem it finds instead of stopping at the
/// first, because a project a human edited usually has more than one. Exit code 2
/// means "problems found" — the same code a refusal uses, so a script can tell a
/// validation failure from a crash.
pub fn validate(args: &[String]) -> Result<ExitCode> {
    let Some(first) = args.first().map(String::as_str) else {
        eprint!("{VALIDATE_USAGE}");
        return Ok(ExitCode::from(2));
    };
    if first == "-h" || first == "--help" || first == "help" {
        print!("{VALIDATE_USAGE}");
        return Ok(ExitCode::SUCCESS);
    }

    let dir = required_arg(
        args,
        0,
        "a project directory",
        "war3 validate <project-dir>",
    )?;
    let report = war3_project::validate(Path::new(dir))?;

    println!("validate  {dir}");
    println!("{}", "=".repeat(60));
    outfmt::field("members as text", report.text, 22);
    outfmt::field("members as content", report.binary, 22);
    outfmt::field("members as stored block", report.raw, 22);
    for warning in &report.warnings {
        println!("  [warn] {warning}");
    }
    if report.is_ok() {
        println!("  no problems found");
        return Ok(ExitCode::SUCCESS);
    }
    println!("  {} problem(s):", report.errors.len());
    for error in &report.errors {
        println!("    {error}");
    }
    Ok(ExitCode::from(2))
}

/// Dispatches `war3 build`.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let Some(first) = args.first().map(String::as_str) else {
        eprint!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    if first == "-h" || first == "--help" || first == "help" {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }

    let dir = required_arg(args, 0, "a project directory", "war3 build <project-dir>")?;
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|index| args.get(index + 1))
        .map(Path::new);

    let report = war3_project::build(Path::new(dir), out)?;

    println!("build  {dir} -> {}", report.output.display());
    println!("{}", "=".repeat(60));
    outfmt::field("members", report.members, 22);
    outfmt::field("written as text", report.text, 22);
    outfmt::field("written as content", report.binary, 22);
    outfmt::field("written as stored block", report.raw, 22);
    outfmt::field(
        "output size",
        outfmt::bytes(report.output_bytes as usize),
        22,
    );

    outfmt::section("verify");
    if report.mismatched.is_empty() {
        outfmt::field(
            "identical",
            format!("{} of {} members", report.members, report.members),
            22,
        );
    } else {
        outfmt::field(
            "identical",
            format!(
                "{} of {}",
                report.members - report.mismatched.len(),
                report.members
            ),
            22,
        );
        for line in &report.mismatched {
            println!("{}MISMATCH {line}", outfmt::INDENT);
        }
    }

    let has_problems = outfmt::diagnostics(&report.diagnostics, verbose(args));
    Ok(if has_problems || !report.mismatched.is_empty() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn verbose(args: &[String]) -> bool {
    args.iter().any(|a| a == "--verbose" || a == "-v")
}

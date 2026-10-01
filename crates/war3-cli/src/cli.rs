//! Argument parsing and command dispatch.
//!
//! No argument-parsing dependency on purpose: the surface is small, and the
//! errors need to name the exact usage that was expected.

use std::process::ExitCode;

use war3_core::Result;

use crate::commands;

/// The command help text.
pub const HELP: &str = "\
war3 - Warcraft III map toolchain

USAGE:
  war3 <command> [arguments...]

COMMANDS:
  map info <map>             print map information in readable form
  map list <map>             list the files inside a map
  map file <map> <member>    write one member file to standard output
  map terrain <map>          print terrain statistics
  map doodads <map>          print the placed doodads
  map units <map>            print the placed units and items
  map objects <map>          print the map's object data
  map archive <map>          print MPQ archive structure
  map rebuild <in> <out>     rewrite an archive with this workspace's writer
  meta check <game-dir>      check whether metadata and trigger definitions are findable
  help                       show this help

EXIT CODES:
  0  success
  1  success, but diagnostics were reported
  2  usage error, or the input could not be read
";

/// Parses and runs a command.
pub fn run(args: &[String]) -> Result<ExitCode> {
    let Some(command) = args.first().map(String::as_str) else {
        print!("{HELP}");
        return Ok(ExitCode::SUCCESS);
    };

    match command {
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        "map" => commands::map::run(&args[1..]),
        "meta" => commands::meta::run(&args[1..]),
        other => {
            eprintln!("war3: unknown command {other:?}");
            eprintln!();
            eprint!("{HELP}");
            Ok(ExitCode::from(2))
        }
    }
}

/// Returns a positional argument, or an error naming the expected usage.
pub fn required_arg<'a>(
    args: &'a [String],
    index: usize,
    what: &str,
    usage: &str,
) -> Result<&'a str> {
    match args.get(index) {
        Some(v) => Ok(v.as_str()),
        None => Err(war3_core::Error::msg(format!(
            "missing {what}; usage: {usage}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_args_prints_help_and_succeeds() {
        let code = run(&[]).unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn help_flag_succeeds() {
        assert_eq!(run(&["--help".into()]).unwrap(), ExitCode::SUCCESS);
        assert_eq!(run(&["-h".into()]).unwrap(), ExitCode::SUCCESS);
    }

    #[test]
    fn unknown_command_exits_two() {
        let code = run(&["nope".into()]).unwrap();
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::from(2)));
    }

    #[test]
    fn map_without_subcommand_exits_two() {
        let code = run(&["map".into()]).unwrap();
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::from(2)));
    }

    #[test]
    fn required_arg_reports_the_usage_line() {
        let args: Vec<String> = vec![];
        let err = required_arg(&args, 0, "a map path", "war3 map info <map>").unwrap_err();
        assert!(err.to_string().contains("war3 map info <map>"), "{err}");
    }

    #[test]
    fn help_mentions_every_documented_command() {
        for cmd in [
            "map info",
            "map list",
            "map file",
            "map terrain",
            "map doodads",
            "map units",
            "map objects",
            "map archive",
            "map rebuild",
            "meta check",
        ] {
            assert!(HELP.contains(cmd), "help is missing {cmd}");
        }
    }
}

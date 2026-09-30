//! `war3`, the w3wright command-line tool.

#![forbid(unsafe_code)]

mod assets;
mod cli;
mod commands;
// Not named `fmt`, which would collide with `std::fmt` at every `use` site.
mod outfmt;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::run(&args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("war3: {err}");
            ExitCode::from(2)
        }
    }
}

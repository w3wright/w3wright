//! Checks that `war3map.doo` survives a parse → serialise round trip.
//!
//! ```text
//! cargo run --example doodads_roundtrip -p war3-map -- "D:\Warcraft3\Maps"
//! ```
//!
//! Real maps are not in the repository, so this is how the writer is checked
//! against the corpus rather than against fixtures alone. A difference is not
//! automatically a bug: it is a file this build does not reproduce exactly, which
//! is what `war3 map extract` uses to decide whether a member may be textified.

use std::path::{Path, PathBuf};

use war3_archive::Archive;
use war3_map::{DoodadFile, UnitFile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let roots: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if roots.is_empty() {
        eprintln!("usage: doodads_roundtrip <map-or-directory>...");
        std::process::exit(2);
    }
    let mut maps = Vec::new();
    for root in &roots {
        collect(root, &mut maps);
    }
    maps.sort();

    let (mut checked, mut identical, mut doodads) = (0usize, 0usize, 0usize);
    let (mut units_checked, mut units_identical, mut units_total) = (0usize, 0usize, 0usize);
    let mut problems = Vec::new();
    for map in &maps {
        let Ok(archive) = Archive::open(map) else {
            continue;
        };
        let names = archive.file_names();

        // war3mapUnits.doo: the other placement file, same question. Checked first
        // because the doodad lookup below skips a map that has no war3map.doo.
        if let Some(found) = names
            .iter()
            .find(|n| n.eq_ignore_ascii_case("war3mapUnits.doo"))
        {
            let member = (*found).to_string();
            match archive.read_file(&member) {
                Ok(bytes) => match UnitFile::parse(&bytes) {
                    Ok(file) => {
                        units_checked += 1;
                        units_total += file.units.len();
                        if file.to_bytes() == bytes {
                            units_identical += 1;
                        } else {
                            let at = file.to_bytes().iter().zip(&bytes).position(|(a, b)| a != b);
                            problems.push(format!(
                                "{}: {member} {} bytes, first difference at {at:?}",
                                file_name(map),
                                bytes.len()
                            ));
                        }
                    }
                    Err(e) => {
                        problems.push(format!("{}: {member} unparsable: {e}", file_name(map)));
                    }
                },
                Err(e) => problems.push(format!("{}: {member} unreadable: {e}", file_name(map))),
            }
        }
        let Some(found) = names.iter().find(|n| n.eq_ignore_ascii_case("war3map.doo")) else {
            continue;
        };
        let member = (*found).to_string();
        let Ok(bytes) = archive.read_file(&member) else {
            problems.push(format!("{}: {member} could not be read", file_name(map)));
            continue;
        };
        checked += 1;
        match DoodadFile::parse(&bytes) {
            Ok(file) => {
                doodads += file.doodads.len();
                let back = file.to_bytes();
                if back == bytes {
                    identical += 1;
                } else {
                    let at = back.iter().zip(&bytes).position(|(a, b)| a != b);
                    problems.push(format!(
                        "{}: {} bytes in, {} out, first difference at {at:?}",
                        file_name(map),
                        bytes.len(),
                        back.len()
                    ));
                }
            }
            Err(e) => problems.push(format!("{}: unparsable: {e}", file_name(map))),
        }
    }

    println!("maps walked    : {}", maps.len());
    println!("war3map.doo    : {checked} found, {identical} byte-identical");
    println!("doodads total  : {doodads}");
    println!(
        "units file     : {units_checked} found, {units_identical} byte-identical, {units_total} units"
    );
    println!("problems       : {}", problems.len());
    for line in &problems {
        println!("  {line}");
    }
    if !problems.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                collect(&entry.path(), out);
            }
        }
        return;
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if matches!(extension.as_deref(), Some("w3x" | "w3m")) {
        out.push(path.to_path_buf());
    }
}

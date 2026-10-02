//! Checks that object data survives a parse → serialise round trip.
//!
//! ```text
//! cargo run --example objects_roundtrip -p war3-object -- "D:\Warcraft3\Maps"
//! ```
//!
//! # Why an example rather than a test
//!
//! Real maps are not in the repository, and object data is exactly the part a
//! hand-made fixture cannot vouch for: it is where the field ids, the value tags
//! and the levelled values of a dozen map editors meet. This walks a directory of
//! maps and reports, per member, whether the bytes that went in are the bytes that
//! came out.
//!
//! A difference is not automatically a bug in the serialiser: it is a file this
//! build does not reproduce exactly, which is what `war3 map extract` uses to
//! decide whether a member may be textified at all.

use std::path::{Path, PathBuf};

use war3_archive::Archive;
use war3_object::{ObjectFile, ObjectKind};

/// The member name of each kind, as it appears inside the archive.
const MEMBERS: &[(&str, ObjectKind)] = &[
    ("war3map.w3u", ObjectKind::Unit),
    ("war3map.w3t", ObjectKind::Item),
    ("war3map.w3a", ObjectKind::Ability),
    ("war3map.w3b", ObjectKind::Destructable),
    ("war3map.w3d", ObjectKind::Doodad),
    ("war3map.w3h", ObjectKind::Buff),
    ("war3map.w3q", ObjectKind::Upgrade),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let roots: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if roots.is_empty() {
        eprintln!("usage: objects_roundtrip <map-or-directory>...");
        std::process::exit(2);
    }
    let mut maps = Vec::new();
    for root in &roots {
        collect(root, &mut maps);
    }
    maps.sort();

    let (mut checked, mut identical, mut unreadable) = (0usize, 0usize, 0usize);
    let mut differed = Vec::new();
    let mut saw_kind = [0usize; 7];

    for map in &maps {
        let Ok(archive) = Archive::open(map) else {
            continue;
        };
        for (member, kind) in MEMBERS {
            // Member names inside an archive are upper case; match case-insensitively.
            let names = archive.file_names();
            let Some(found) = names.iter().find(|n| n.eq_ignore_ascii_case(member)) else {
                continue;
            };
            let name = (*found).to_string();
            let Ok(bytes) = archive.read_file(&name) else {
                unreadable += 1;
                differed.push(format!("{}: {name} could not be read", file_name(map)));
                continue;
            };
            checked += 1;
            saw_kind[MEMBERS.iter().position(|(_, k)| k == kind).unwrap_or(0)] += 1;
            match ObjectFile::parse(*kind, &bytes) {
                Ok(file) => {
                    let back = file.to_bytes();
                    if back == bytes {
                        identical += 1;
                    } else {
                        let at = back.iter().zip(&bytes).position(|(a, b)| a != b);
                        differed.push(format!(
                            "{}: {name} {} bytes in, {} out, first difference at {at:?}",
                            file_name(map),
                            bytes.len(),
                            back.len()
                        ));
                        if let Some(at) = at {
                            differed.push(format!("      want {}", window(&bytes, at)));
                            differed.push(format!("      got  {}", window(&back, at)));
                        }
                    }
                }
                Err(e) => {
                    unreadable += 1;
                    differed.push(format!("{}: {name} unparsable: {e}", file_name(map)));
                }
            }
        }
    }

    println!("maps walked        : {}", maps.len());
    println!("members checked    : {checked}");
    println!("byte-identical     : {identical}");
    println!("differed/unreadable: {}", differed.len());
    for (i, (member, _)) in MEMBERS.iter().enumerate() {
        if saw_kind[i] > 0 {
            println!("  {member}: {}", saw_kind[i]);
        }
    }
    println!("read failures      : {unreadable}");
    for line in &differed {
        println!("DIFF  {line}");
    }
    Ok(())
}

fn window(bytes: &[u8], at: usize) -> String {
    let start = at.saturating_sub(12);
    let end = (at + 12).min(bytes.len());
    bytes[start..end]
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if start + i == at {
                format!("[{b:02X}]")
            } else {
                format!("{b:02X}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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

//! Checks that `war3map.w3i` survives a parse → serialise round trip.
//!
//! ```text
//! cargo run --example w3i_roundtrip -p war3-map -- "D:\Warcraft3\Maps"
//! ```
//!
//! # Why an example rather than a test
//!
//! Real maps are not in the repository — Blizzard's and other projects' maps
//! carry their own licences (see `examples/lost-temple/README.md`) — so the
//! serialiser cannot be checked against the corpus from `cargo test`. This walks
//! a directory instead and reports, per map, whether the bytes that went in are
//! the bytes that came out.
//!
//! A difference is not necessarily a bug in the serialiser: it is a file whose
//! content this build does not reproduce exactly, which is what
//! `war3 map extract` uses to decide whether a member may be textified at all.

use std::path::{Path, PathBuf};

use war3_archive::Archive;
use war3_map::w3i::MapInfo;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let roots: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if roots.is_empty() {
        eprintln!("usage: w3i_roundtrip <map-or-directory>...");
        std::process::exit(2);
    }

    let mut maps = Vec::new();
    for root in &roots {
        collect(root, &mut maps);
    }
    maps.sort();

    let (mut identical, mut skipped, mut unreadable) = (0usize, 0usize, 0usize);
    let mut differed = Vec::new();
    let mut missing = Vec::new();

    for map in &maps {
        let bytes = match Archive::open(map).and_then(|archive| archive.read_file("war3map.w3i")) {
            Ok(bytes) => bytes,
            Err(e) => {
                unreadable += 1;
                missing.push(format!("{}: {e}", map.display()));
                continue;
            }
        };
        let info = match MapInfo::parse(&bytes) {
            Ok(info) => info,
            Err(e) => {
                skipped += 1;
                missing.push(format!("{}: {e}", map.display()));
                continue;
            }
        };
        let back = info.to_bytes();
        if back == bytes {
            identical += 1;
        } else {
            let at = back.iter().zip(&bytes).position(|(a, b)| a != b);
            differed.push(format!(
                "{}: {} bytes in, {} out, first difference at {:?}, version {}, sections {:?}",
                file_name(map),
                bytes.len(),
                back.len(),
                at,
                info.format_version,
                info.trailing_sections
            ));
            if let Some(at) = at {
                differed.push(format!("      want {}", window(&bytes, at)));
                differed.push(format!("      got  {}", window(&back, at)));
            }
        }
    }

    println!("maps walked     : {}", maps.len());
    println!("byte-identical  : {identical}");
    println!("differed        : {}", differed.len());
    println!("w3i unparsable  : {skipped}");
    println!("no readable w3i : {unreadable}");
    for line in &differed {
        println!("DIFF  {line}");
    }
    for line in &missing {
        println!("NONE  {line}");
    }
    Ok(())
}

/// Sixteen bytes either side of `at`, as hex, for eyeballing a difference. The
/// differing byte is bracketed.
fn window(bytes: &[u8], at: usize) -> String {
    let start = at.saturating_sub(16);
    let end = (at + 16).min(bytes.len());
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

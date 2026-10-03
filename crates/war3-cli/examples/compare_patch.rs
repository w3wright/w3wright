//! Narrows down an in-place patch the game sometimes refuses.
//!
//! The observed behaviour is **intermittent**: the same patched file loaded once and was reported
//! `war3map.w3u is corrupt` two other times. Intermittent rules out "the bytes are wrong" as a
//! complete explanation, so this prints the things that are not intermittently checked by a reader:
//!
//! 1. every differing **region** between the original and patched member, not just the first byte;
//! 2. the patched archive's **block table entry** for `war3map.w3u` — a size or offset that the
//!    archive-level reader tolerates and the game does not would look exactly like this;
//! 3. what a **rebuild** of the same change produces, for comparison: if rebuild works and patch does
//!    not, the fault is in `ArchivePatcher`, not in the object bytes.
//!
//! Usage: `cargo run -p war3-cli --example compare_patch -- <original.w3x> <patched.w3x>`
fn main() {
    let mut args = std::env::args().skip(1);
    let original_path = args.next().expect("usage: <original.w3x> <patched.w3x>");
    let patched_path = args.next().expect("usage: <original.w3x> <patched.w3x>");

    let original = war3_archive::Archive::open(&original_path).expect("original opens");
    let patched = war3_archive::Archive::open(&patched_path).expect("patched opens");

    let a = original.read_file("war3map.w3u").expect("original w3u");
    let b = patched.read_file("war3map.w3u").expect("patched w3u");

    println!("=== 1. every differing region in war3map.w3u ===");
    println!("original {} bytes, patched {} bytes", a.len(), b.len());
    for (start, len) in differing_regions(&a, &b) {
        println!("--- region at {start}, {len} bytes ---");
        println!("  original: {:02x?}", &a[start..(start + len).min(a.len())]);
        println!("  patched : {:02x?}", &b[start..(start + len).min(b.len())]);
        // As text, because a name is what changed.
        println!(
            "  original text: {:?}",
            String::from_utf8_lossy(&a[start..(start + len).min(a.len())])
        );
        println!(
            "  patched  text: {:?}",
            String::from_utf8_lossy(&b[start..(start + len).min(b.len())])
        );
    }

    println!();
    println!("=== 2. the block table entry for war3map.w3u ===");
    for (label, archive) in [("original", &original), ("patched", &patched)] {
        match archive
            .block_index_for_name("war3map.w3u")
            .and_then(|i| archive.block_entry(i as usize))
        {
            Some(entry) => println!("{label:>9}: {entry:?}"),
            None => println!("{label:>9}: no block entry"),
        }
    }

    println!();
    println!("=== 3. archive-level numbers ===");
    for (label, path) in [("original", &original_path), ("patched", &patched_path)] {
        let bytes = std::fs::read(path).expect("reads");
        let archive = war3_archive::Archive::open(path).expect("opens");
        println!(
            "{label:>9}: {} bytes, {} members, {} diagnostics",
            bytes.len(),
            archive.file_names().len(),
            archive.diagnostics().items().len()
        );
    }

    // Every member must still read, and the ones the game needs most are the ones to name.
    println!();
    println!("=== 4. members that fail to read in the patched archive ===");
    let mut bad = Vec::new();
    for name in patched.file_names() {
        if patched.read_file(name).is_err() {
            bad.push(name);
        }
    }
    println!("{bad:?}");

    // A rebuild of the same content, through the *other* writer. If the game accepts this and rejects
    // the patched one, the writer is the difference.
    println!();
    println!("=== 5. what a rebuild of the patched archive looks like ===");
    match war3_archive::RebuildPreview::of_file(&patched_path) {
        Ok(preview) => println!("{preview}"),
        Err(e) => println!("a rebuild reports: {e}"),
    }
}

/// The `(start, len)` of every run where `a` and `b` differ, coalescing nearby runs.
///
/// Coalesced because a changed string produces a run of differing bytes, and a byte-by-byte listing of
/// a 105 KB file would be unreadable.
fn differing_regions(a: &[u8], b: &[u8]) -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    for i in 0..a.len().max(b.len()) {
        let differs = a.get(i) != b.get(i);
        match (&mut current, differs) {
            (Some((_, len)), true) => *len += 1,
            (Some((start, len)), false) => {
                regions.push((*start, *len));
                current = None;
            }
            (None, true) => current = Some((i, 1)),
            (None, false) => {}
        }
    }
    if let Some(region) = current {
        regions.push(region);
    }
    regions
}

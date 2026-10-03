//! Diagnoses an in-place patch: why does the game reject the patched `war3map.w3u`?
//!
//! Three questions, in increasing scope. The first is the one that decides everything:
//!
//! 1. **Does `ObjectFile::to_bytes()` reproduce the input byte for byte?** If not, the serialiser is
//!    wrong and no patching strategy can work — the game reads a different file than we wrote.
//! 2. If it is exact, **does a one-field change survive**? Print the two files' lengths and the first
//!    difference, so the shape of the difference is visible.
//! 3. **Is the patched member readable from the patched archive?** The game's message is about the
//!    member, but the archive could be the thing that broke.
//!
//! Usage: `cargo run -p war3-cli --example diagnose_patch -- <original.w3x> <patched.w3x>`

fn main() {
    let mut args = std::env::args().skip(1);
    let original_path = args.next().expect("usage: <original.w3x> <patched.w3x>");
    let patched_path = args.next().expect("usage: <original.w3x> <patched.w3x>");

    // ---- 1. re-serialise the original and compare ----
    let original_archive = war3_archive::Archive::open(&original_path).expect("original opens");
    let original_w3u = original_archive
        .read_file("war3map.w3u")
        .expect("original has war3map.w3u");

    let parsed = war3_object::ObjectFile::parse(war3_object::ObjectKind::Unit, &original_w3u)
        .expect("original parses");
    let reserialised = parsed.to_bytes();

    println!("1. re-serialise the original:");
    println!("   original      {} bytes", original_w3u.len());
    println!("   re-serialised {} bytes", reserialised.len());
    match first_difference(&original_w3u, &reserialised) {
        None if original_w3u.len() == reserialised.len() => {
            println!("   IDENTICAL — the serialiser is exact, so the fault is elsewhere");
        }
        None => println!("   ⚠️ same prefix but different lengths — the tail is missing"),
        Some(at) => {
            println!("   ⚠️ FIRST DIFFERENCE AT BYTE {at}");
            let from = at.saturating_sub(8);
            println!(
                "   original      {:02x?}",
                &original_w3u[from..(at + 16).min(original_w3u.len())]
            );
            println!(
                "   re-serialised {:02x?}",
                &reserialised[from..(at + 16).min(reserialised.len())]
            );
        }
    }

    // ---- 2. the patched member, as it sits in the patched archive ----
    let patched_archive = war3_archive::Archive::open(&patched_path).expect("patched opens");
    let patched_w3u = patched_archive
        .read_file("war3map.w3u")
        .expect("patched has war3map.w3u");
    println!();
    println!("2. the patched member:");
    println!("   patched w3u   {} bytes", patched_w3u.len());
    match first_difference(&original_w3u, &patched_w3u) {
        Some(at) => println!("   differs from the original at byte {at}"),
        None => println!("   ⚠️ IDENTICAL to the original — the patch did not land"),
    }
    // Where does the name live, and how long is it? A length prefix that disagrees with the string is
    // the classic cause of "file data is corrupt".
    for (label, bytes) in [("original", &original_w3u), ("patched", &patched_w3u)] {
        match find(bytes, b"TRIGSTR_117").or_else(|| find(bytes, b"W3WRIGHT PATCH TEST")) {
            Some(at) => {
                let len_at = at.saturating_sub(4);
                let declared = i32::from_le_bytes([
                    bytes[len_at],
                    bytes[len_at + 1],
                    bytes[len_at + 2],
                    bytes[len_at + 3],
                ]);
                println!("   {label}: name text at {at}, declared length just before = {declared}");
            }
            None => println!("   {label}: neither string found"),
        }
    }

    // ---- 3. archive-level sanity ----
    println!();
    println!("3. the patched archive:");
    println!(
        "   members: original {} / patched {}",
        original_archive.file_names().len(),
        patched_archive.file_names().len()
    );
    println!(
        "   patched diagnostics: {}",
        patched_archive.diagnostics().items().len()
    );
    let raw = std::fs::read(&patched_path).expect("patched reads");
    println!("   file size: {} bytes", raw.len());
    println!(
        "   original size: {} bytes",
        std::fs::metadata(&original_path)
            .map(|m| m.len())
            .unwrap_or(0)
    );

    // Does every member still read? A member the game needs that we broke would look like this too.
    let mut bad = Vec::new();
    for name in patched_archive.file_names() {
        if patched_archive.read_file(name).is_err() {
            bad.push(name);
        }
    }
    println!("   members that no longer read: {bad:?}");
}

/// The first index where two slices differ, or `None` when one is a prefix of the other.
fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

/// The first index of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

//! Real-archive acceptance for the in-place patcher (ADR-0028).
//!
//! Proves the property the editor's safety rests on: after editing one member,
//! **every other byte of the user's map is the original byte**.
//!
//! Run:
//! ```text
//! cargo run --release --example patch_w3i -p war3-project -- <map> [out.w3x]
//! ```
//!
//! The edit itself goes through the same text round trip the source project uses
//! (`to_document` → change a field → `from_document`), so this also checks that the
//! textification path is lossless on a real map — which is the assumption the whole
//! `[text]` member design rests on.

use war3_archive::ArchivePatcher;
use war3_project::w3i_text;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let map = args
        .next()
        .ok_or("usage: patch_w3i <map> [out.w3x]")?;
    let out = args.next().unwrap_or_else(|| {
        let mut p = std::path::PathBuf::from(&map);
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        p.set_file_name(format!("{stem}-patched.w3x"));
        p.to_string_lossy().to_string()
    });

    let original = std::fs::read(&map)?;
    let archive = war3_archive::Archive::open(&map)?;
    let w3i = archive.read_file("war3map.w3i")?;
    println!("map      {map}");
    println!("original {} bytes", original.len());

    // ---- the edit, through the text form the source project uses ----
    let info = war3_map::MapInfo::parse(&w3i)?;
    let before_name = info.name.clone();
    let mut doc = w3i_text::to_document(&info);
    if std::env::var_os("W3_DUMP_DOC").is_some() {
        eprintln!("{}", doc.render());
    }
    // `set` lives on the section, because a key only means something inside one.
    doc.sections
        .iter_mut()
        .find(|s| s.name == "info")
        .ok_or("the rendered document has no [info] section")?
        .set("name", "W3wright patched name");
    let doc_name = doc
        .sections
        .iter()
        .find(|s| s.name == "info")
        .and_then(|s| s.get("name"))
        .unwrap_or("<missing>")
        .to_string();
    eprintln!("document [info].name is now {doc_name:?}");
    let edited = w3i_text::from_document(&doc)?.to_bytes();
    println!(
        "w3i      {} -> {} bytes, name {before_name:?} -> {:?}",
        w3i.len(),
        edited.len(),
        war3_map::MapInfo::parse(&edited)?.name
    );

    // The round trip must not change anything else about the file's meaning.
    let reparsed = war3_map::MapInfo::parse(&edited)?;
    assert_eq!(reparsed.author, info.author, "author must survive the edit");
    assert_eq!(
        reparsed.description, info.description,
        "description must survive the edit"
    );
    assert_eq!(
        reparsed.playable_width, info.playable_width,
        "geometry must survive the edit"
    );

    // ---- the patch ----
    let mut patcher = ArchivePatcher::new(original.clone())?;
    patcher.replace("war3map.w3i", edited.clone())?;
    let patched = patcher.to_bytes();
    std::fs::write(&out, &patched)?;

    // ---- the acceptance assertions ----
    println!("patched  {} bytes (+{})", patched.len(), patched.len() - original.len());

    // Everything up to the block table is untouched: prefix, header, all the member
    // data, and the hash table. This is the strong claim a rebuild cannot make.
    let block_table_at = archive.header().block_table_offset() as usize;
    if patched[..block_table_at] != original[..block_table_at] {
        let at = patched[..block_table_at]
            .iter()
            .zip(original[..block_table_at].iter())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        return Err(format!(
            "FAIL: something before the block table changed, first at offset {at}"
        )
        .into());
    }
    println!("✓ prefix, header, member data and hash table are byte identical");
    // ---- the block table's *plaintext* must be preserved for every other member ----
    //
    // ⚠️ The ciphertext of the whole table necessarily changes: MPQ table encryption
    // chains each word to the previous one, so editing entry 0 re-encrypts entries
    // 1..n differently even though their plaintext is identical. Comparing ciphertext
    // would therefore look alarming and mean nothing. Comparing the decoded entries is
    // the check that matters, and it is the one that says the neighbours are intact.
    let patched_index = archive.block_index_for_name("war3map.w3i")
        .ok_or("war3map.w3i has no block")? as usize;
    let before = war3_archive::Archive::from_bytes(original.clone())?;
    let after = war3_archive::Archive::from_bytes(patched.clone())?;
    let mut compared = 0usize;
    for index in 0..before.header().block_table_size as usize {
        let (Some(a), Some(b)) = (before.block_entry(index), after.block_entry(index)) else {
            return Err(format!("FAIL: block entry {index} disappeared").into());
        };
        if index == patched_index {
            // The patched member: its position and sizes must differ, by design.
            if a.file_pos == b.file_pos {
                return Err("FAIL: the patched entry still points at the old block".into());
            }
            continue;
        }
        if a != b {
            return Err(format!(
                "FAIL: block entry {index} changed: {a:?} vs {b:?}"
            )
            .into());
        }
        compared += 1;
    }
    println!("✓ {compared} other block table entries decode identically (patched index {patched_index})");


    // The appended bytes are exactly the edited member.
    // ⚠️ Not "everything after the block table": a real map can carry its own bytes
    // past the tables (this one has 260 of them). The appended block begins exactly
    // where the original file ended, which is the only correct boundary here.
    if patched[original.len()..] != edited[..] {
        return Err(format!(
            "FAIL: the bytes appended after the original end are not the edited member \
             (expected {} bytes, found {})",
            edited.len(),
            patched.len() - original.len()
        )
        .into());
    }
    println!("✓ the only added bytes are the edited member");

    // Every untouched member reads back identically, compared through the reader
    // rather than by offset arithmetic.
    let reopened = war3_archive::Archive::open(&out)?;
    let mut checked = 0usize;
    for name in reopened.file_names() {
        if name.eq_ignore_ascii_case("war3map.w3i") {
            continue;
        }
        let old = archive.read_file(name);
        let new = reopened.read_file(name);
        match (old, new) {
            (Ok(a), Ok(b)) if a == b => checked += 1,
            (Err(a), Err(b)) if a.to_string() == b.to_string() => checked += 1,
            (a, b) => {
                return Err(format!(
                    "FAIL: member {name} changed: {a:?} vs {b:?}"
                )
                .into())
            }
        }
    }
    println!("✓ {checked} untouched members read back identically");

    // And the edit is visible through the map model.
    let map_after = war3_map::Map::from_source(&reopened)?;
    println!("map name now {:?}", map_after.metadata.name);
    if map_after.metadata.name != "W3wright patched name" {
        return Err("FAIL: the patched name is not visible through the map model".into());
    }
    println!("✓ the edit is visible through the map model");

    println!("\nwrote {out}");
    Ok(())
}

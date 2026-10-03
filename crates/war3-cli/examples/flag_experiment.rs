//! Produces in-place patches that differ **only** in the block-table flags, so the game can decide
//! which one it accepts.
//!
//! # Why this exists
//!
//! `ArchivePatcher::replace` writes a replacement block as `FLAG_EXISTS | FLAG_SINGLE_UNIT`
//! (`0x81000000`) with `compressed_size == uncompressed_size` — plain and unsectored — and clears the
//! original block's compression bit. Our own reader accepts that, and **the game reported
//! `war3map.w3u` as corrupt**.
//!
//! Reading it back ourselves proves nothing: `Archive::read_file` takes the `SINGLE_UNIT` branch and,
//! for an uncompressed block, truncates to the uncompressed size — it never checks whether the flags
//! describe a layout a *different* reader would accept. That is the gap this experiment closes.
//!
//! The member's **bytes are identical in every variant**; only the three block-table words change. So
//! if one variant loads in the game and another does not, the flags are the answer and nothing else
//! can be.
//!
//! Usage: `cargo run -p war3-cli --example flag_experiment -- <in.w3x> <out-dir>`

/// The combinations to try.
///
/// The original block is `0x80000200` — `EXISTS | COMPRESSED`, a **compressed, unsectored** block (no
/// `SINGLE_UNIT`, no `MULTI_BLOCK`). ⚠️ `SIZE_IN_BLOCK` (`0x100`) is set there, and the first
/// replacement **dropped it**: `replace` writes only `EXISTS | SINGLE_UNIT`. The standard says a
/// single-unit block carries `SIZE_IN_BLOCK`, which is why it is the first thing to put back.
const COMBINATIONS: &[(&str, u32)] = &[
    ("a-exists-single", 0x8000_0000 | 0x0100_0000),
    (
        "b-exists-single-sizeinblock",
        0x8000_0000 | 0x0100_0000 | 0x0000_0100,
    ),
    ("c-exists-sizeinblock", 0x8000_0000 | 0x0000_0100),
    ("d-exists-only", 0x8000_0000),
    (
        "e-exists-single-multiblock",
        0x8000_0000 | 0x0100_0000 | 0x0400_0000,
    ),
];

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("usage: <in.w3x> <out-dir>");
    let out_dir = args.next().expect("usage: <in.w3x> <out-dir>");
    std::fs::create_dir_all(&out_dir).expect("the output directory is creatable");

    let archive = war3_archive::Archive::open(&input).expect("the map opens");
    let original = archive.read_file("war3map.w3u").expect("war3map.w3u");

    let mut file =
        war3_object::ObjectFile::parse(war3_object::ObjectKind::Unit, &original).expect("parses");
    let mut changed = 0;
    for object in &mut file.table.custom {
        if object.id.to_string().eq_ignore_ascii_case("hC06") {
            for modification in &mut object.modifications {
                if modification.field.to_string().eq_ignore_ascii_case("unam") {
                    modification.value =
                        war3_object::FieldValue::String("W3WRIGHT PATCH TEST".into());
                    changed += 1;
                }
            }
        }
    }
    assert_eq!(changed, 1, "expected to change exactly one field");
    let patched = file.to_bytes();

    let buffer = std::fs::read(&input).expect("the map reads");
    let block_index = archive
        .block_index_for_name("war3map.w3u")
        .expect("war3map.w3u is in the block table");
    let entry = *archive
        .block_entry(block_index as usize)
        .expect("the block entry exists");
    println!("member   {} -> {} bytes", original.len(), patched.len());
    println!("original {entry:?}");

    for (label, flags) in COMBINATIONS {
        let mut bytes = buffer.clone();
        let new_at = bytes.len() as u32;
        bytes.extend_from_slice(&patched);
        rewrite_block_entry(
            &mut bytes,
            &archive,
            block_index,
            new_at,
            patched.len() as u32,
            *flags,
        );

        let path = std::path::Path::new(&out_dir).join(format!("{label}.w3x"));
        std::fs::write(&path, &bytes).expect("the variant writes");

        // Our reader must accept every variant, or the experiment would be measuring two things.
        let ours = war3_archive::Archive::open(&path).and_then(|a| a.read_file("war3map.w3u"));
        println!(
            "  {label:30} flags=0x{flags:08x} our-reader={:?}",
            ours.map(|d| d.len()).map_err(|e| e.to_string())
        );
    }

    println!();
    println!("Test each file in {out_dir} in the game, in order a..e.");
    println!("The first that loads says which flags the replacement block needs.");
}

/// Rewrites one block-table entry's position, sizes and flags, decrypting and re-encrypting the
/// **whole** table.
///
/// ⚠️ The whole table, not the one entry: MPQ table encryption is a stream, so rewriting 16 bytes in
/// place corrupts every entry after it. `ArchivePatcher` documents the same trap with a real failure
/// behind it (a member left pointing at offset 133,171,052); this repeats the whole-table step because
/// an experiment that corrupted the table would be measuring the wrong thing.
fn rewrite_block_entry(
    bytes: &mut [u8],
    archive: &war3_archive::Archive,
    block_index: u32,
    new_at: u32,
    size: u32,
    flags: u32,
) {
    let table = war3_archive::crypt_table();
    let key = war3_archive::hash_string(
        &table,
        war3_archive::HashType::FileKey,
        war3_archive::BLOCK_TABLE_KEY_NAME,
    );
    let header = archive.header();
    let block_pos = header.block_table_offset() as usize;
    let table_len = header.block_table_size as usize * 16;
    let table_bytes = &mut bytes[block_pos..block_pos + table_len];
    let mut words = war3_archive::bytes_to_u32_le(table_bytes);
    war3_archive::decrypt(&table, &mut words, key);

    let at = block_index as usize * 4;
    words[at] = new_at - header.file_offset as u32;
    words[at + 1] = size;
    words[at + 2] = size;
    words[at + 3] = flags;

    war3_archive::encrypt(&table, &mut words, key);
    for (i, word) in words.iter().enumerate() {
        table_bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
}

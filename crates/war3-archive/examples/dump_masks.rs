//! Print each member's compression mask byte, straight from the sector data.
//!
//! `war3 map list` reports a compression *name*, which is derived from the block
//! flags. When a member is unsupported it is worth seeing the mask itself rather
//! than a name: the mask is what the decoder dispatches on, and the name is a
//! summary of it that can hide which bits are set.
//!
//! ⚠️ **Known limitation: encrypted members come out as noise.** This reads
//! `Archive::raw_member`, which is the *stored* form — it is not decrypted, and
//! neither is the sector table. So for a member with the encrypted flag, the
//! "first sector at" offsets below are ciphertext read as little-endian numbers and
//! the mask is a ciphertext byte. Seen on `侏罗纪公园1.5.w3x`, where `(ATTRIBUTES)`
//! reported a sector offset of 2578083829.
//!
//! Decrypting would mean duplicating the archive's key derivation here, which is
//! exactly the kind of second implementation of format logic the design set rules
//! out. If this tool is needed for encrypted members, the right change is to expose
//! the already-decrypted mask from the archive — not to reimplement crypto in an
//! example.
//!
//! Run: `cargo run --release --example dump_masks -p war3-archive -- <map>`

use war3_archive::Archive;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let map = std::env::args()
        .nth(1)
        .ok_or("usage: dump_masks <map>")?;
    let archive = Archive::open(&map)?;

    let mut names = archive.file_names();
    names.sort_unstable();

    for name in names {
        let Ok(raw) = archive.raw_member(name) else {
            println!("{name}: no block");
            continue;
        };
        let block = &raw.block;
        let single = raw.flags.0 & 0x0100_0000 != 0;
        let masked = raw.flags.0 & 0x0000_0200 != 0;

        // The first byte of the member's data is the mask, but only when it is
        // compressed and stored per sector: a single-unit member has no sector
        // table, and an uncompressed one has no mask at all.
        let (mask, note) = if !masked {
            (0u8, "uncompressed".to_string())
        } else if single {
            (block.first().copied().unwrap_or(0), "single unit".to_string())
        } else {
            let start = u32::from_le_bytes([block[0], block[1], block[2], block[3]]) as usize;
            (
                block.get(start).copied().unwrap_or(0),
                format!("sector table, first sector at {start}"),
            )
        };

        let flags = format!(
            "0x{:08X}{}",
            raw.flags.0,
            if raw.flags.0 & 0x0001_0000 != 0 {
                " enc"
            } else {
                ""
            }
        );
        println!("{name:<28} mask=0x{mask:02X} flags={flags}  {note}");
    }
    Ok(())
}

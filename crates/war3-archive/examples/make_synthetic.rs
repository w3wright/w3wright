//! Writes a **structurally valid** MPQ archive, used to test the reader.
//!
//! The point of this tool is to separate "the reader is wrong" from "this
//! particular file is odd". The reader's path has several linked stages — header
//! discovery, table decryption, hash lookup, name enumeration, sector
//! decryption, decompression — and a real map that fails to open gives no clue
//! which stage is at fault. An archive with a known-correct answer does.
//!
//! The generator writes:
//!
//! - hash and block tables encrypted with the `HASH` and `BLK#` keys;
//! - three members covering the multi-block, single-block and stored layouts;
//! - sectors split at 4096 bytes with the per-sector compression flag;
//! - an `HM3W` prefix so the MPQ header sits at offset 512;
//! - **no `(listfile)`**, forcing the reader to use its known-names fallback.
//!
//! Because the generator shares the crypto primitives with the reader, it cannot
//! catch a mistake in the crypto itself. What it guards is the assembly:
//! offset systems, sector lengths, the enumeration ladder, branch selection.
//!
//! ```text
//! cargo run --example make_synthetic -p war3-archive
//! ```

use war3_archive::{crypt_table, hash_string, HashType};

/// Encrypts 32-bit words; the exact inverse of the reader's `decrypt`.
fn encrypt(table: &[u32; 0x500], data: &mut [u32], mut key: u32) {
    let mut seed: u32 = 0xEEEE_EEEE;
    for value in data.iter_mut() {
        seed = seed.wrapping_add(table[0x400 + (key & 0xFF) as usize]);
        let plain = *value;
        *value = plain ^ key.wrapping_add(seed);
        key = ((!key << 0x15).wrapping_add(3)) ^ plain.wrapping_add(seed).wrapping_add(seed << 5);
        seed = plain
            .wrapping_add(seed)
            .wrapping_add(seed << 5)
            .wrapping_add(3);
    }
}

fn from_words(words: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 4);
    for w in words {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

/// Sector size, matching the shift used by real archives.
const SECTOR_SIZE: usize = 4096;
/// Shift stored in the header. `512 << 3 == 4096`.
const SECTOR_SHIFT: u16 = 3;
/// Absolute offset of the MPQ header; the preceding bytes are the `HM3W` prefix.
const HEADER_POS: u32 = 0x200;
/// Where member data starts.
const DATA_START: u32 = 0x400;

struct Member {
    name: &'static str,
    /// On-disk block, including the per-sector flag bytes.
    block: Vec<u8>,
    uncompressed_size: u32,
    flags: u32,
    file_pos: u32,
}

/// How a member is stored on disk.
#[derive(Clone, Copy, PartialEq)]
enum Storage {
    /// Not compressed.
    Raw,
    /// A single zlib stream with no sector table.
    Single,
    /// A sector offset table plus independently compressed sectors.
    Multi,
}

fn main() {
    let table = crypt_table();
    let out_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "synthetic.w3x".to_string());

    let compressible: Vec<u8> = (0..8000u32).map(|i| (i % 7) as u8 + b'A').collect();
    let w3i = b"\x19\x00\x00\x00stub map info payload".to_vec();
    let script = b"function main takes nothing returns nothing\nendfunction\n".to_vec();

    let sources: [(&'static str, Vec<u8>, Storage); 3] = [
        ("war3map.w3i", w3i, Storage::Single),
        ("war3map.w3e", compressible, Storage::Multi),
        ("war3map.j", script, Storage::Raw),
    ];

    let mut cursor = DATA_START;
    let mut built: Vec<Member> = Vec::new();

    for (name, payload, storage) in sources {
        let uncompressed_size = payload.len() as u32;
        let block: Vec<u8>;
        let flags: u32;

        match storage {
            Storage::Multi => {
                // Layout: sector offset table (`sector_count + 1` entries) then
                // the sectors. The final entry is the end of the data; without it
                // the last sector's length cannot be determined.
                let sector_count = payload.len().div_ceil(SECTOR_SIZE).max(1);
                let mut sectors: Vec<Vec<u8>> = Vec::with_capacity(sector_count);
                for chunk in payload.chunks(SECTOR_SIZE) {
                    let mut s = vec![0xFF]; // this sector is compressed
                    s.extend_from_slice(&zlib_store(chunk));
                    sectors.push(s);
                }
                let mut b = vec![0u8; (sector_count + 1) * 4];
                let mut offset = b.len();
                for (i, sector) in sectors.iter().enumerate() {
                    b[i * 4..i * 4 + 4].copy_from_slice(&(offset as u32).to_le_bytes());
                    offset += sector.len();
                }
                b[sector_count * 4..sector_count * 4 + 4]
                    .copy_from_slice(&(offset as u32).to_le_bytes());
                for sector in &sectors {
                    b.extend_from_slice(sector);
                }
                block = b;
                flags = 0x0000_0200 | 0x0400_0000; // COMPRESSED | MULTI_BLOCK
            }
            Storage::Single => {
                // A whole zlib stream: no sector table, no flag bytes.
                block = zlib_store(&payload);
                flags = 0x0000_0200 | 0x0100_0000; // COMPRESSED | SINGLE_UNIT
            }
            Storage::Raw => {
                block = payload.clone();
                flags = 0x0000_0000;
            }
        }

        built.push(Member {
            name,
            block,
            uncompressed_size,
            flags,
            file_pos: cursor,
        });
        cursor += built.last().unwrap().block.len() as u32;
    }

    // ---- tables ----
    const HASH_SIZE: u32 = 64;
    let block_count = built.len() as u32;
    let mut hash_words = vec![0xFFFF_FFFFu32; HASH_SIZE as usize * 4];
    let mut block_words = vec![0u32; block_count as usize * 4];

    for (index, m) in built.iter().enumerate() {
        let upper = m.name.to_uppercase();
        let mut slot = hash_string(&table, HashType::TableOffset, &upper) & (HASH_SIZE - 1);
        let want = hash_string(&table, HashType::NameA, &upper);
        loop {
            if hash_words[slot as usize * 4 + 3] == 0xFFFF_FFFF {
                hash_words[slot as usize * 4] = want;
                hash_words[slot as usize * 4 + 1] = hash_string(&table, HashType::NameB, &upper);
                hash_words[slot as usize * 4 + 2] = 0; // locale | platform
                hash_words[slot as usize * 4 + 3] = index as u32;
                break;
            }
            slot = (slot + 1) & (HASH_SIZE - 1);
        }
        let base = index * 4;
        block_words[base] = m.file_pos;
        block_words[base + 1] = m.block.len() as u32;
        block_words[base + 2] = m.uncompressed_size;
        block_words[base + 3] = m.flags;
    }

    // Self-check before encryption: counting non-empty slots in the *ciphertext*
    // is meaningless, since no slot is `0xFFFFFFFF` after encryption. Getting
    // this order wrong once produced an empty table, and the symptom was "the
    // reader finds no names" — which looks like a reader bug.
    let filled = hash_words
        .chunks_exact(4)
        .filter(|e| e[3] != 0xFFFF_FFFF)
        .count();
    assert_eq!(
        filled,
        built.len(),
        "plaintext hash table should hold exactly {} entries, holds {filled}",
        built.len()
    );

    encrypt(&table, &mut hash_words, u32::from_le_bytes(*b"HASH"));
    encrypt(&table, &mut block_words, u32::from_le_bytes(*b"BLK#"));

    // ---- layout ----
    let data_end = built
        .iter()
        .map(|m| m.file_pos + m.block.len() as u32)
        .max()
        .unwrap_or(DATA_START);
    let hash_pos = data_end;
    let block_pos = hash_pos + HASH_SIZE * 16;
    let archive_end = block_pos + block_count * 16;

    let mut out = vec![0u8; archive_end as usize];

    // The HM3W prefix, which is why the header is not at offset 0.
    out[0..4].copy_from_slice(b"HM3W");
    let map_name = b"Synthetic Test Map";
    out[8..8 + map_name.len()].copy_from_slice(map_name);

    // Header.
    let h = HEADER_POS as usize;
    out[h..h + 4].copy_from_slice(b"MPQ\x1a");
    out[h + 4..h + 8].copy_from_slice(&32u32.to_le_bytes()); // header size
    out[h + 8..h + 12].copy_from_slice(&(archive_end - HEADER_POS).to_le_bytes());
    out[h + 12..h + 14].copy_from_slice(&0u16.to_le_bytes()); // format version
    out[h + 14..h + 16].copy_from_slice(&SECTOR_SHIFT.to_le_bytes());
    // Table positions are relative to the header.
    out[h + 16..h + 20].copy_from_slice(&(hash_pos - HEADER_POS).to_le_bytes());
    out[h + 20..h + 24].copy_from_slice(&(block_pos - HEADER_POS).to_le_bytes());
    out[h + 24..h + 28].copy_from_slice(&HASH_SIZE.to_le_bytes());
    out[h + 28..h + 32].copy_from_slice(&block_count.to_le_bytes());

    for m in &built {
        let at = m.file_pos as usize;
        out[at..at + m.block.len()].copy_from_slice(&m.block);
    }
    let hw = from_words(&hash_words);
    out[hash_pos as usize..hash_pos as usize + hw.len()].copy_from_slice(&hw);
    let bw = from_words(&block_words);
    out[block_pos as usize..block_pos as usize + bw.len()].copy_from_slice(&bw);

    std::fs::write(&out_path, &out).expect("failed to write output");

    println!("wrote {out_path} ({} bytes)", out.len());
    println!(
        "  header at {HEADER_POS}, sector {SECTOR_SIZE}, hash {HASH_SIZE}, blocks {block_count}"
    );
    println!("  hash table at 0x{hash_pos:X}, block table at 0x{block_pos:X}");
    for m in &built {
        println!(
            "  {:<14} pos=0x{:06X} on-disk {:6} uncompressed {:6} flags=0x{:08X}",
            m.name,
            m.file_pos,
            m.block.len(),
            m.uncompressed_size,
            m.flags
        );
    }
    println!();
    println!("note: no (listfile) is written, so the reader must use its known-names fallback");
}

/// Builds a zlib stream whose deflate payload is stored blocks.
///
/// This avoids implementing a compressor. The reader's three deflate paths are
/// covered by its own unit tests; all this needs to do is produce a *valid*
/// stream.
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // CMF 0x78 is deflate with a 32K window; pick a FLG that makes the 16-bit
    // big-endian header divisible by 31.
    let mut flg = 0u8;
    while (0x7800u16 | u16::from(flg)) % 31 != 0 {
        flg += 1;
    }
    out.push(0x78);
    out.push(flg);

    // Only the last stored block sets BFINAL.
    let chunks: Vec<&[u8]> = data.chunks(65535).collect();
    if chunks.is_empty() {
        out.push(0x01); // empty input: one BFINAL stored block of length 0
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    } else {
        for (i, chunk) in chunks.iter().enumerate() {
            let is_last = i + 1 == chunks.len();
            out.push(u8::from(is_last));
            let len = chunk.len() as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(chunk);
        }
    }

    // Adler-32
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

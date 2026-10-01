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
//! - hash and block tables encrypted with the keys the format prescribes, the
//!   `HashString` of `(hash table)` and `(block table)`;
//! - four members covering the single-unit, multi-block, stored-single-unit and
//!   stored-multi-block layouts;
//! - sectors that are **genuinely** deflate-compressed — with the `0x02`
//!   compression-mask byte and a stored size below the data they hold — plus
//!   sectors stored raw, which is the other branch the reader has to choose
//!   between;
//! - an `HM3W` prefix so the MPQ header sits at offset 512, which is what makes
//!   the archive-relative member offsets matter;
//! - **no `(listfile)`**, forcing the reader to use its known-names fallback.
//!
//! # What this cannot catch
//!
//! The generator encrypts the tables with the same cipher the reader decrypts
//! with, and the two are an exact inverse pair. A mistake *shared* by both
//! therefore round-trips here perfectly and never shows up. That is not
//! hypothetical: this workspace once read every real Blizzard archive as noise
//! while this generator's own output read back byte for byte. The guards
//! against that are the pinned ciphertext in `war3_archive::crypto`'s tests and
//! the real files described in `examples/lost-temple/README.md` — not this
//! program. What this program guards is the assembly: offset bases, sector
//! lengths, the compression branch, the enumeration ladder.
//!
//! ```text
//! cargo run --example make_synthetic -p war3-archive
//! ```

use std::collections::HashMap;

use war3_archive::{
    crypt_table, encrypt, hash_string, HashType, BLOCK_TABLE_KEY_NAME, HASH_TABLE_KEY_NAME,
};

/// Absolute offset of the MPQ header; the preceding bytes are the `HM3W` prefix.
const HEADER_POS: u32 = 0x200;
/// Size of the header written here.
const HEADER_SIZE: u32 = 32;
/// Where member data starts, as an absolute file offset.
const DATA_START: u32 = HEADER_POS + HEADER_SIZE;
/// Sector size, matching the shift real archives use.
const SECTOR_SIZE: usize = 4096;
/// Shift stored in the header. `512 << 3 == 4096`.
const SECTOR_SHIFT: u16 = 3;
/// Hash table size: a power of two, and larger than the number of members.
const HASH_SIZE: u32 = 64;
/// `MPQ_FILE_EXISTS`: the block holds a file rather than free space.
const FLAG_EXISTS: u32 = 0x8000_0000;
/// `MPQ_FILE_COMPRESSED`.
const FLAG_COMPRESSED: u32 = 0x0000_0200;
/// `MPQ_FILE_SINGLE_UNIT`.
const FLAG_SINGLE_UNIT: u32 = 0x0100_0000;
/// `MPQ_FILE_MULTI_BLOCK`.
const FLAG_MULTI_BLOCK: u32 = 0x0400_0000;

/// How a member is stored on disk.
#[derive(Clone, Copy, PartialEq)]
enum Storage {
    /// One deflate stream, no sector offset table.
    SingleDeflate,
    /// A sector offset table plus independently deflated sectors.
    MultiDeflate,
    /// Stored as is, in one block.
    Stored,
    /// Stored as is, split across sectors, with no sector offset table.
    StoredMulti,
}

struct Member {
    name: &'static str,
    /// On-disk block, including the per-sector mask bytes.
    block: Vec<u8>,
    uncompressed_size: u32,
    flags: u32,
    /// Offset of the block, relative to the archive header.
    file_pos: u32,
}

fn main() {
    let table = crypt_table();
    let out_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "synthetic.w3x".to_string());

    // A plausible `.w3i`: its version word, then filler that deflates well.
    let w3i = {
        let mut v = Vec::new();
        v.extend_from_slice(&25u32.to_le_bytes());
        while v.len() < 1200 {
            v.extend_from_slice(b"war3wright synthetic map info payload; ");
        }
        v
    };
    // Repetitive with period 7, so deflate finds long matches and the sectors
    // end up far below `SECTOR_SIZE`.
    let terrain: Vec<u8> = (0..8800u32).map(|i| (i % 7) as u8 + b'A').collect();
    let script = b"function main takes nothing returns nothing\nendfunction\n".to_vec();
    // Arbitrary bytes: this member is stored, so it must not need to compress.
    let stored_multi: Vec<u8> = (0..9000u32).map(|i| (i % 251) as u8).collect();

    let sources: [(&'static str, Vec<u8>, Storage); 4] = [
        ("war3map.w3i", w3i, Storage::SingleDeflate),
        ("war3map.w3e", terrain, Storage::MultiDeflate),
        ("war3map.j", script, Storage::Stored),
        ("war3map.wts", stored_multi, Storage::StoredMulti),
    ];

    let mut cursor = DATA_START;
    let mut built: Vec<Member> = Vec::new();

    for (name, payload, storage) in sources {
        let uncompressed_size = payload.len() as u32;
        let (block, flags) = match storage {
            Storage::SingleDeflate => {
                let block = sector(0x02, deflate_fixed(&payload), &payload);
                (block, FLAG_EXISTS | FLAG_COMPRESSED | FLAG_SINGLE_UNIT)
            }
            Storage::MultiDeflate => {
                // Layout: sector offset table (`sector_count + 1` entries) then
                // the sectors. The final entry is the end of the data; without
                // it the last sector's length cannot be determined.
                let chunks: Vec<&[u8]> = payload.chunks(SECTOR_SIZE).collect();
                let sectors: Vec<Vec<u8>> = chunks
                    .iter()
                    .map(|chunk| sector(0x02, deflate_fixed(chunk), chunk))
                    .collect();
                let mut block = vec![0u8; (sectors.len() + 1) * 4];
                let mut offset = block.len();
                for (i, sec) in sectors.iter().enumerate() {
                    block[i * 4..i * 4 + 4].copy_from_slice(&(offset as u32).to_le_bytes());
                    offset += sec.len();
                }
                block[sectors.len() * 4..sectors.len() * 4 + 4]
                    .copy_from_slice(&(offset as u32).to_le_bytes());
                for sec in &sectors {
                    block.extend_from_slice(sec);
                }
                (block, FLAG_EXISTS | FLAG_COMPRESSED | FLAG_MULTI_BLOCK)
            }
            Storage::Stored => (payload.clone(), FLAG_EXISTS | FLAG_SINGLE_UNIT),
            Storage::StoredMulti => (payload.clone(), FLAG_EXISTS),
        };

        // A compressed member's sectors only exercise the compressed branch if
        // they really are smaller than the data they hold; otherwise the reader
        // correctly reads them as stored and this fixture would be claiming
        // coverage it does not have.
        if storage == Storage::SingleDeflate {
            assert!(
                block.len() < payload.len(),
                "{name}: the deflate stream is not smaller than its payload"
            );
        }

        built.push(Member {
            name,
            block,
            uncompressed_size,
            flags,
            // Block offsets are relative to the archive header, not the file.
            file_pos: cursor - HEADER_POS,
        });
        cursor += built.last().unwrap().block.len() as u32;
    }

    // ---- tables ----
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

    encrypt(
        &table,
        &mut hash_words,
        hash_string(&table, HashType::FileKey, HASH_TABLE_KEY_NAME),
    );
    encrypt(
        &table,
        &mut block_words,
        hash_string(&table, HashType::FileKey, BLOCK_TABLE_KEY_NAME),
    );

    // ---- layout ----
    let data_end = built
        .iter()
        .map(|m| m.file_pos + HEADER_POS + m.block.len() as u32)
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
    out[h + 4..h + 8].copy_from_slice(&HEADER_SIZE.to_le_bytes());
    out[h + 8..h + 12].copy_from_slice(&(archive_end - HEADER_POS).to_le_bytes());
    out[h + 12..h + 14].copy_from_slice(&0u16.to_le_bytes()); // format version
    out[h + 14..h + 16].copy_from_slice(&SECTOR_SHIFT.to_le_bytes());
    // Table positions are relative to the header.
    out[h + 16..h + 20].copy_from_slice(&(hash_pos - HEADER_POS).to_le_bytes());
    out[h + 20..h + 24].copy_from_slice(&(block_pos - HEADER_POS).to_le_bytes());
    out[h + 24..h + 28].copy_from_slice(&HASH_SIZE.to_le_bytes());
    out[h + 28..h + 32].copy_from_slice(&block_count.to_le_bytes());

    for m in &built {
        let at = (m.file_pos + HEADER_POS) as usize;
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
            "  {:<14} pos=+0x{:06X} on-disk {:6} uncompressed {:6} flags=0x{:08X}",
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

/// Builds one on-disk sector: the compression-mask byte, then the payload.
///
/// The mask byte counts towards the sector's stored size, which is what the
/// reader compares against the length the sector must hold.
fn sector(mask: u8, compressed: Vec<u8>, payload: &[u8]) -> Vec<u8> {
    assert!(
        compressed.len() + 1 < payload.len(),
        "a compressed sector must be smaller than its payload, or the format \
         requires it to be stored raw"
    );
    let mut out = Vec::with_capacity(compressed.len() + 1);
    out.push(mask);
    out.extend_from_slice(&compressed);
    out
}

fn from_words(words: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 4);
    for w in words {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// A minimal DEFLATE encoder
// ---------------------------------------------------------------------------
//
// The fixture needs sectors that are genuinely smaller than the data they hold.
// Decompression is what the reader is being tested on, so the generator cannot
// simply store deflate's uncompressed-block form, which is always a few bytes
// *larger* than its input. Hence a real encoder — small, but real: fixed
// Huffman codes and greedy matching, no dynamic codes and no lazy matching.
//
// It is deliberately not in the library: nothing in `war3-archive` needs to
// write DEFLATE, and a second compressor is a second thing to keep correct. The
// reader's own inflate is what validates this code, and the reader's inflate is
// in turn validated against real Blizzard archives.

/// Longest match DEFLATE can encode.
const MAX_MATCH: usize = 258;
/// Largest representable distance.
const MAX_DISTANCE: usize = 32768;

/// Base length of the 29 length codes, symbol 257 upwards.
const LENGTH_BASE: [usize; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// Extra bits carried by each length code.
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Base distance of the 30 distance codes.
const DIST_BASE: [usize; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
/// Extra bits carried by each distance code.
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Collects bits in the order DEFLATE wants them.
///
/// Bits are appended least significant first *within* an element, while Huffman
/// codes are appended most significant bit first. Getting this backwards is the
/// classic DEFLATE mistake, so the two are separate methods.
struct BitWriter {
    out: Vec<u8>,
    /// How many bits of the final byte are in use.
    used: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            used: 0,
        }
    }

    /// Appends `count` bits of `value`, least significant bit first.
    fn write_bits(&mut self, value: u32, count: u32) {
        for i in 0..count {
            if self.used == 0 {
                self.out.push(0);
            }
            if (value >> i) & 1 != 0 {
                let last = self.out.len() - 1;
                self.out[last] |= 1 << self.used;
            }
            self.used = (self.used + 1) & 7;
        }
    }

    /// Appends a Huffman code, most significant bit first.
    fn write_code(&mut self, code: u32, width: u32) {
        for i in (0..width).rev() {
            self.write_bits((code >> i) & 1, 1);
        }
    }

    /// The finished stream, with the final partial byte zero-padded.
    fn finish(self) -> Vec<u8> {
        self.out
    }
}

/// The fixed-Huffman code for a literal, length or end-of-block symbol.
fn fixed_code(symbol: usize) -> (u32, u32) {
    match symbol {
        0..=143 => (0x30 + symbol as u32, 8),
        144..=255 => (0x190 + (symbol - 144) as u32, 9),
        256..=279 => ((symbol - 256) as u32, 7),
        _ => (0xC0 + (symbol - 280) as u32, 8),
    }
}

/// The three bytes at `pos`, as one key for the match finder.
fn key3(data: &[u8], pos: usize) -> u32 {
    (u32::from(data[pos]) << 16) | (u32::from(data[pos + 1]) << 8) | u32::from(data[pos + 2])
}

fn write_symbol(bits: &mut BitWriter, symbol: usize) {
    let (code, width) = fixed_code(symbol);
    bits.write_code(code, width);
}

fn write_match(bits: &mut BitWriter, length: usize, distance: usize) {
    let mut len_code = LENGTH_BASE.len() - 1;
    while LENGTH_BASE[len_code] > length {
        len_code -= 1;
    }
    write_symbol(bits, 257 + len_code);
    if LENGTH_EXTRA[len_code] > 0 {
        bits.write_bits(
            (length - LENGTH_BASE[len_code]) as u32,
            LENGTH_EXTRA[len_code],
        );
    }

    let mut dist_code = DIST_BASE.len() - 1;
    while DIST_BASE[dist_code] > distance {
        dist_code -= 1;
    }
    bits.write_code(dist_code as u32, 5);
    if DIST_EXTRA[dist_code] > 0 {
        bits.write_bits(
            (distance - DIST_BASE[dist_code]) as u32,
            DIST_EXTRA[dist_code],
        );
    }
}

/// Compresses as a single fixed-Huffman block with greedy matching.
fn deflate_fixed(data: &[u8]) -> Vec<u8> {
    let mut bits = BitWriter::new();
    bits.write_bits(1, 1); // BFINAL: this is the last block
    bits.write_bits(1, 2); // BTYPE: 01, fixed Huffman codes

    // The most recent position each three-byte prefix was seen at. One
    // candidate is enough for the repetitive fixture payloads; it only costs
    // ratio, never correctness.
    let mut heads: HashMap<u32, usize> = HashMap::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let mut best: Option<(usize, usize)> = None;
        if pos + 3 <= data.len() {
            if let Some(&prev) = heads.get(&key3(data, pos)) {
                let distance = pos - prev;
                if distance <= MAX_DISTANCE {
                    let limit = (data.len() - pos).min(MAX_MATCH);
                    let mut length = 3;
                    // A match may overlap the position being encoded: that is
                    // how DEFLATE expresses runs, and the decoder copies byte
                    // by byte.
                    while length < limit && data[prev + length] == data[pos + length] {
                        length += 1;
                    }
                    best = Some((length, distance));
                }
            }
        }

        match best {
            Some((length, distance)) => {
                write_match(&mut bits, length, distance);
                let last = (pos + length).min(data.len().saturating_sub(2));
                for p in pos..last {
                    heads.insert(key3(data, p), p);
                }
                pos += length;
            }
            None => {
                write_symbol(&mut bits, usize::from(data[pos]));
                if pos + 3 <= data.len() {
                    heads.insert(key3(data, pos), pos);
                }
                pos += 1;
            }
        }
    }
    write_symbol(&mut bits, 256); // end of block
    zlib_wrap(&bits.finish(), data)
}

/// Wraps a DEFLATE stream in the zlib header and trailer the format expects.
fn zlib_wrap(deflate: &[u8], original: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(deflate.len() + 6);
    // CMF 0x78 is deflate with a 32K window; pick a FLG that makes the 16-bit
    // big-endian header divisible by 31.
    let mut flg = 0u8;
    while (0x7800u16 | u16::from(flg)) % 31 != 0 {
        flg += 1;
    }
    out.push(0x78);
    out.push(flg);
    out.extend_from_slice(deflate);

    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in original {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

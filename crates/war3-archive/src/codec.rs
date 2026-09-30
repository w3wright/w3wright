//! Sector decompression.
//!
//! zlib is the compression method found in practice, so it is implemented here
//! from RFC 1951 rather than pulled in as a dependency. That keeps the crate
//! free of C code, which matters because the core has to compile to WASM and
//! should build with a plain Rust toolchain.
//!
//! Other methods (bzip2, PKWare implode, LZMA) return an explicit error rather
//! than being skipped silently.

use std::fmt;

/// Why decompression failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// The data ended early.
    Eof,
    /// The zlib header is not valid.
    BadZlibHeader([u8; 2]),
    /// A deflate block declared the reserved type 3.
    BadBlockType(u8),
    /// The zlib compression method is not 8 (deflate).
    BadCompressionMethod(u8),
    /// A Huffman code-length table is not valid.
    BadCodeLengths,
    /// The stream contained an invalid Huffman code.
    BadCode,
    /// A back-reference pointed outside the output produced so far.
    BadDistance(usize),
    /// A stored block's length and its complement disagree.
    StoredBlockLengthMismatch {
        /// Declared length.
        len: u16,
        /// Declared complement.
        nlen: u16,
    },
    /// The decompressed length does not match what the container promised.
    SizeMismatch {
        /// Length the container declared.
        expected: usize,
        /// Length actually produced.
        got: usize,
    },
    /// The compression method is recognised but not implemented.
    Unsupported(u32),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof => f.write_str("compressed data ended early"),
            Self::BadZlibHeader(h) => write!(f, "invalid zlib header: {h:02X?}"),
            Self::BadBlockType(b) => write!(f, "invalid deflate block type {b}"),
            Self::BadCompressionMethod(m) => write!(f, "invalid zlib compression method {m}"),
            Self::BadCodeLengths => f.write_str("invalid deflate code-length table"),
            Self::BadCode => f.write_str("invalid deflate code in stream"),
            Self::BadDistance(d) => write!(f, "deflate distance {d} exceeds output so far"),
            Self::StoredBlockLengthMismatch { len, nlen } => {
                write!(f, "stored block length {len:#06X} and complement {nlen:#06X} disagree")
            }
            Self::SizeMismatch { expected, got } => {
                write!(f, "decompressed length mismatch: expected {expected}, got {got}")
            }
            Self::Unsupported(m) => write!(f, "unsupported compression mask {m:#010X}"),
        }
    }
}

impl std::error::Error for CodecError {}

// ---------------------------------------------------------------------------
// Compression masks
// ---------------------------------------------------------------------------

/// No compression.
pub const COMPRESSION_NONE: u32 = 0x0000_0000;
/// zlib. The common case for map members.
pub const COMPRESSION_ZLIB: u32 = 0x0000_0002;
/// bzip2.
pub const COMPRESSION_BZIP2: u32 = 0x0000_0010;
/// PKWare implode.
pub const COMPRESSION_PKWARE: u32 = 0x0000_0100;
/// LZMA.
///
/// Its value is `0x12`, the combination of zlib and bzip2, rather than a bit of
/// its own. Compression masks are priority lists, so reusing a combination is
/// how the format grew; inventing a distinct bit here would be wrong.
pub const COMPRESSION_LZMA: u32 = 0x0000_0012;

/// Bits that name a compression *algorithm*.
///
/// # Two different bits
///
/// | Bit | Meaning |
/// | --- | --- |
/// | `0x0000_0200` (`MPQ_FILE_COMPRESSED`) | "this member is compressed"; carries no algorithm |
/// | low byte (`0x02`, `0x10`, `0x100`) | which algorithm, as a priority list |
///
/// In real archives a zlib member has flags such as `0x04000200` — the
/// compressed bit is set and the low byte is *zero*. The algorithm therefore
/// cannot be read off the flags; it is found by trying. This constant is
/// consequently only useful for display.
pub const COMPRESSION_METHOD_MASK: u32 =
    COMPRESSION_ZLIB | COMPRESSION_BZIP2 | COMPRESSION_PKWARE | COMPRESSION_LZMA;

/// `MPQ_FILE_COMPRESSED`: "this member is compressed".
pub const COMPRESSION_FLAG_COMPRESSED: u32 = 0x0000_0200;

/// Whether a mask means "stored uncompressed".
///
/// The test is whether `MPQ_FILE_COMPRESSED` is clear, *not* whether the low
/// byte is zero: a real zlib member has flags `0x04000200`, whose low byte is
/// zero.
#[must_use]
pub const fn is_uncompressed_mask(mask: u32) -> bool {
    mask & COMPRESSION_FLAG_COMPRESSED == 0
}

/// Decompresses according to a member's flags.
///
/// - `MPQ_FILE_COMPRESSED` clear: the data is stored, returned as is.
/// - set: decompressed as zlib.
/// - anything else: [`CodecError::Unsupported`].
///
/// `expected` is the length the container declared. It is checked, because an
/// unchecked mismatch produces data of the wrong length that only fails much
/// later.
pub fn decompress(data: &[u8], mask: u32, expected: usize) -> Result<Vec<u8>, CodecError> {
    if is_uncompressed_mask(mask) {
        return Ok(data.to_vec());
    }

    // If the low byte names algorithms and zlib is not among them, say so
    // instead of handing, say, a bzip2 stream to the zlib decoder.
    let declared = mask & COMPRESSION_METHOD_MASK & 0xFF;
    if declared != 0 && declared & COMPRESSION_ZLIB == 0 {
        return Err(CodecError::Unsupported(mask));
    }

    let out = zlib_decompress(data, expected)?;
    if out.len() != expected {
        return Err(CodecError::SizeMismatch { expected, got: out.len() });
    }
    Ok(out)
}

/// Decompresses a zlib stream (RFC 1950 wrapper around RFC 1951 data).
///
/// `expected` is used for pre-allocation only.
pub fn zlib_decompress(data: &[u8], expected: usize) -> Result<Vec<u8>, CodecError> {
    if data.len() < 2 {
        return Err(CodecError::Eof);
    }
    let cmf = data[0];
    let flg = data[1];
    let cm = cmf & 0x0F;
    if cm != 8 {
        return Err(CodecError::BadCompressionMethod(cm));
    }
    // CMF and FLG together form a 16-bit big-endian value divisible by 31.
    if (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0 {
        return Err(CodecError::BadZlibHeader([cmf, flg]));
    }
    // A preset dictionary never occurs in these archives; seeing the flag means
    // the read position is wrong.
    if flg & 0x20 != 0 {
        return Err(CodecError::BadZlibHeader([cmf, flg]));
    }
    inflate(&data[2..], expected)
}

// ---------------------------------------------------------------------------
// DEFLATE (RFC 1951)
// ---------------------------------------------------------------------------

/// LSB-first bit reader.
///
/// Deflate reads bits from the least significant bit upwards, while Huffman
/// codes themselves are read most significant bit first. Getting this backwards
/// is the classic deflate mistake.
struct BitReader<'a> {
    data: &'a [u8],
    /// Absolute bit position of the next bit to read.
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read_bit(&mut self) -> Result<u32, CodecError> {
        let byte_idx = self.bit_pos >> 3;
        if byte_idx >= self.data.len() {
            return Err(CodecError::Eof);
        }
        let bit = (self.data[byte_idx] >> (self.bit_pos & 7)) & 1;
        self.bit_pos += 1;
        Ok(u32::from(bit))
    }

    /// Reads `count` bits (at most 16), least significant first.
    fn read_bits(&mut self, count: u32) -> Result<u32, CodecError> {
        let mut value = 0u32;
        for i in 0..count {
            value |= self.read_bit()? << i;
        }
        Ok(value)
    }

    /// Discards bits up to the next byte boundary.
    fn align_to_byte(&mut self) {
        self.bit_pos = (self.bit_pos + 7) & !7;
    }

    /// Reads whole bytes; the reader must already be aligned.
    fn read_aligned_bytes(&mut self, count: usize) -> Result<&'a [u8], CodecError> {
        debug_assert_eq!(self.bit_pos & 7, 0);
        let start = self.bit_pos >> 3;
        let end = start.checked_add(count).ok_or(CodecError::Eof)?;
        if end > self.data.len() {
            return Err(CodecError::Eof);
        }
        self.bit_pos = end << 3;
        Ok(&self.data[start..end])
    }
}

/// A canonical Huffman decoding table.
///
/// Stores, per code length, how many codes have that length and where their
/// symbols start. Decoding accumulates one bit at a time until the code falls
/// inside a length's range.
#[derive(Debug, Clone)]
struct Huffman {
    /// `counts[len]` is the number of symbols with code length `len` (`len` from 1).
    counts: [u16; 16],
    /// Symbols ordered by code length.
    symbols: Vec<u16>,
}

impl Huffman {
    /// Builds a table from per-symbol code lengths, validating the Kraft inequality.
    fn from_code_lengths(lengths: &[u8]) -> Result<Self, CodecError> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            if len > 15 {
                return Err(CodecError::BadCodeLengths);
            }
            counts[len as usize] += 1;
        }
        // A length of 0 means the symbol never appears.
        counts[0] = 0;

        // The shifted-out bits must be filled exactly (an all-empty table is fine).
        let mut left = 1i32;
        for (_, &count) in counts.iter().enumerate().take(16).skip(1) {
            left <<= 1;
            left -= i32::from(count);
            if left < 0 {
                return Err(CodecError::BadCodeLengths);
            }
        }

        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + counts[len];
        }

        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &len) in lengths.iter().enumerate() {
            if len != 0 {
                symbols[offsets[len as usize] as usize] = symbol as u16;
                offsets[len as usize] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    /// Decodes one symbol.
    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, CodecError> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= reader.read_bit()? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                let pos = (index + (code - first)) as usize;
                return self.symbols.get(pos).copied().ok_or(CodecError::BadCode);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(CodecError::BadCode)
    }
}

/// The fixed literal/length table from RFC 1951.
fn fixed_literal_table() -> Huffman {
    let mut lengths = [0u8; 288];
    for (i, slot) in lengths.iter_mut().enumerate() {
        *slot = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Huffman::from_code_lengths(&lengths).expect("fixed literal table is always valid")
}

/// The fixed distance table from RFC 1951.
fn fixed_distance_table() -> Huffman {
    let lengths = [5u8; 32];
    Huffman::from_code_lengths(&lengths).expect("fixed distance table is always valid")
}

/// Base lengths for length codes 257..=285.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// Extra bits for each length code.
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Base distances for distance codes 0..=29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
/// Extra bits for each distance code.
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// The order in which code lengths are stored, which is not the natural order.
const CODE_LENGTH_ORDER: [usize; 19] =
    [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Decompresses raw deflate data.
///
/// `expected` is used for pre-allocation only.
pub fn inflate(data: &[u8], expected: usize) -> Result<Vec<u8>, CodecError> {
    let mut reader = BitReader::new(data);
    let mut out: Vec<u8> = Vec::with_capacity(expected);

    loop {
        let is_final = reader.read_bit()? == 1;
        let block_type = reader.read_bits(2)?;
        match block_type {
            0 => inflate_stored_block(&mut reader, &mut out)?,
            1 => {
                let lit = fixed_literal_table();
                let dist = fixed_distance_table();
                inflate_huffman_block(&mut reader, &mut out, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = read_dynamic_tables(&mut reader)?;
                inflate_huffman_block(&mut reader, &mut out, &lit, &dist)?;
            }
            other => return Err(CodecError::BadBlockType(other as u8)),
        }
        if is_final {
            break;
        }
    }
    Ok(out)
}

fn inflate_stored_block(reader: &mut BitReader<'_>, out: &mut Vec<u8>) -> Result<(), CodecError> {
    reader.align_to_byte();
    let header = reader.read_aligned_bytes(4)?;
    let len = u16::from_le_bytes([header[0], header[1]]);
    let nlen = u16::from_le_bytes([header[2], header[3]]);
    if len != !nlen {
        return Err(CodecError::StoredBlockLengthMismatch { len, nlen });
    }
    let payload = reader.read_aligned_bytes(usize::from(len))?;
    out.extend_from_slice(payload);
    Ok(())
}

fn read_dynamic_tables(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), CodecError> {
    let hlit = reader.read_bits(5)? as usize + 257;
    let hdist = reader.read_bits(5)? as usize + 1;
    let hclen = reader.read_bits(4)? as usize + 4;

    let mut code_length_lengths = [0u8; 19];
    for &slot in CODE_LENGTH_ORDER.iter().take(hclen) {
        code_length_lengths[slot] = reader.read_bits(3)? as u8;
    }
    let code_length_table = Huffman::from_code_lengths(&code_length_lengths)?;

    // Expand the code-length sequence, which uses three run-length codes.
    let total = hlit + hdist;
    let mut lengths = Vec::with_capacity(total);
    while lengths.len() < total {
        let symbol = code_length_table.decode(reader)?;
        match symbol {
            0..=15 => lengths.push(symbol as u8),
            16 => {
                let prev = *lengths.last().ok_or(CodecError::BadCodeLengths)?;
                let repeat = 3 + reader.read_bits(2)? as usize;
                for _ in 0..repeat {
                    lengths.push(prev);
                }
            }
            17 => {
                let repeat = 3 + reader.read_bits(3)? as usize;
                lengths.resize(lengths.len() + repeat, 0);
            }
            18 => {
                let repeat = 11 + reader.read_bits(7)? as usize;
                lengths.resize(lengths.len() + repeat, 0);
            }
            _ => return Err(CodecError::BadCodeLengths),
        }
    }
    if lengths.len() != total {
        return Err(CodecError::BadCodeLengths);
    }

    let lit = Huffman::from_code_lengths(&lengths[..hlit])?;
    let dist = Huffman::from_code_lengths(&lengths[hlit..])?;
    Ok((lit, dist))
}

fn inflate_huffman_block(
    reader: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    lit: &Huffman,
    dist: &Huffman,
) -> Result<(), CodecError> {
    loop {
        let symbol = lit.decode(reader)?;
        match symbol {
            0..=255 => out.push(symbol as u8),
            256 => return Ok(()),
            257..=285 => {
                let idx = usize::from(symbol) - 257;
                let length =
                    u32::from(LENGTH_BASE[idx]) + reader.read_bits(u32::from(LENGTH_EXTRA[idx]))?;
                let dist_symbol = dist.decode(reader)? as usize;
                if dist_symbol >= DIST_BASE.len() {
                    return Err(CodecError::BadCode);
                }
                let distance = u32::from(DIST_BASE[dist_symbol])
                    + reader.read_bits(u32::from(DIST_EXTRA[dist_symbol]))?;
                let distance = distance as usize;
                if distance == 0 || distance > out.len() {
                    return Err(CodecError::BadDistance(distance));
                }
                // Byte at a time, because distance may be smaller than length
                // and the copy then overlaps what it is producing (this is RLE).
                let start = out.len() - distance;
                for i in 0..length as usize {
                    let byte = out[start + i];
                    out.push(byte);
                }
            }
            _ => return Err(CodecError::BadCode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real vector: `python -c "import zlib;print(zlib.compress(b'hello world',6).hex())"`.
    ///
    /// Hand-written zlib streams are error-prone (header check bits plus a
    /// trailing Adler-32), so every vector here is produced by zlib itself.
    const ZLIB_HELLO_WORLD: &[u8] = &[
        0x78, 0x9C, 0xCB, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0x28, 0xCF, 0x2F, 0xCA, 0x49, 0x01, 0x00,
        0x1A, 0x0B, 0x04, 0x5D,
    ];

    #[test]
    fn zlib_round_trip_of_short_string() {
        let out = zlib_decompress(ZLIB_HELLO_WORLD, 11).unwrap();
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn stored_block_is_handled() {
        // Hand-built BFINAL=1, BTYPE=00 block.
        let payload = b"abc";
        let mut data = vec![0x01]; // bit0 = BFINAL, bits1-2 = BTYPE
        data.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        data.extend_from_slice(&(!(payload.len() as u16)).to_le_bytes());
        data.extend_from_slice(payload);
        let out = inflate(&data, payload.len()).unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn stored_block_length_mismatch_is_rejected() {
        let mut data = vec![0x01];
        data.extend_from_slice(&3u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes()); // should be 0xFFFC
        data.extend_from_slice(b"abc");
        assert!(matches!(
            inflate(&data, 3),
            Err(CodecError::StoredBlockLengthMismatch { .. })
        ));
    }

    #[test]
    fn zlib_rejects_bad_compression_method() {
        // CM = 7 rather than 8.
        assert!(matches!(
            zlib_decompress(&[0x77, 0x9C, 0, 0, 0, 0], 0),
            Err(CodecError::BadCompressionMethod(7))
        ));
    }

    #[test]
    fn zlib_rejects_header_failing_check_bits() {
        assert!(matches!(
            zlib_decompress(&[0x78, 0x00, 0, 0, 0, 0], 0),
            Err(CodecError::BadZlibHeader(_))
        ));
    }

    #[test]
    fn uncompressed_mask_passes_through() {
        let out = decompress(b"raw bytes", COMPRESSION_NONE, 9).unwrap();
        assert_eq!(out, b"raw bytes");
    }

    #[test]
    fn real_world_masks_are_classified_correctly() {
        // Values taken from real archives. `0x200` is the compressed flag and
        // says nothing about the algorithm; the low byte of a zlib member is
        // often zero.
        assert!(is_uncompressed_mask(0x0000_0000), "no flags");
        assert!(is_uncompressed_mask(0x0000_0008), "only unrelated low bits");
        assert!(!is_uncompressed_mask(0x0400_0200), "multi-block + COMPRESSED, as seen for zlib");
        assert!(!is_uncompressed_mask(0x0100_0200), "single-unit + COMPRESSED, as seen for zlib");
        assert!(!is_uncompressed_mask(0x0000_0202), "low byte also names zlib");
    }

    #[test]
    fn compressed_flag_low_byte_is_zero_yet_data_is_still_zlib() {
        // Regression guard: treating "low byte has no algorithm bits" as "stored"
        // made every `0x04000200` sector come back raw.
        let out = decompress(ZLIB_HELLO_WORLD, 0x0400_0200, 11).unwrap();
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn explicitly_non_zlib_low_byte_is_reported_as_unsupported() {
        assert_eq!(
            decompress(b"x", 0x0000_0210, 1),
            Err(CodecError::Unsupported(0x210))
        );
        assert_eq!(
            decompress(b"x", 0x0000_0218, 1),
            Err(CodecError::Unsupported(0x218))
        );
    }

    #[test]
    fn size_mismatch_is_reported_not_silently_accepted() {
        assert!(matches!(
            decompress(ZLIB_HELLO_WORLD, 0x0000_0202, 999),
            Err(CodecError::SizeMismatch { expected: 999, .. })
        ));
    }

    #[test]
    fn overlapping_copy_behaves_like_rle() {
        // `python -c "import zlib;print(zlib.compress(b'a'*10,6).hex())"`.
        // Distance 1 with a length above 1 is RLE, so the copy must be
        // byte-by-byte rather than a slice copy.
        let compressed: &[u8] = &[
            0x78, 0x9C, 0x4B, 0x4C, 0x84, 0x01, 0x00, 0x14, 0xE1, 0x03, 0xCB,
        ];
        let out = zlib_decompress(compressed, 10).unwrap();
        assert_eq!(out, vec![b'a'; 10]);
    }

    #[test]
    fn fixed_huffman_block_is_decoded() {
        // `python -c "import zlib;print(zlib.compress(b'abcdef',6).hex())"`.
        let compressed: &[u8] = &[
            0x78, 0x9C, 0x4B, 0x4C, 0x4A, 0x4E, 0x49, 0x4D, 0x03, 0x00, 0x08, 0x1E, 0x02, 0x56,
        ];
        assert_eq!(zlib_decompress(compressed, 6).unwrap(), b"abcdef");
    }

    #[test]
    fn bad_block_type_is_rejected() {
        assert!(matches!(inflate(&[0x07], 0), Err(CodecError::BadBlockType(3))));
    }

    #[test]
    fn truncated_input_is_eof_not_panic() {
        assert_eq!(inflate(&[], 0), Err(CodecError::Eof));
        assert_eq!(zlib_decompress(&[0x78], 0), Err(CodecError::Eof));
    }
}

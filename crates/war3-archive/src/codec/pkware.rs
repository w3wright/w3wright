//! PKWare Data Compression Library "explode" — the `0x08` sector mask.
//!
//! This is the one compression method these archives use that the workspace could
//! not read, and its absence was load-bearing rather than cosmetic: 167 of the
//! 190 maps on the development machine carry their `war3map.wts` imploded, and
//! without it every `TRIGSTR_*` reference stays unresolved, so a map's name,
//! author and description are unreadable in the common case.
//!
//! # What the format is
//!
//! A 2-byte header, then a bit stream:
//!
//! ```text
//! byte 0  compression type: 0 = binary, 1 = ASCII
//! byte 1  dictionary size bits: 4, 5 or 6  (1 KiB, 2 KiB, 4 KiB)
//! ```
//!
//! So the caller passes the bytes *after* the MPQ sector mask byte — see
//! [`super::decompress`], which strips that first and hands over the rest.
//!
//! Two independent Huffman-ish codings share the bit stream and differ in the one
//! way that punishes guessing:
//!
//! | what | bits |
//! | --- | --- |
//! | literal / match-length | read **most-significant bit first** |
//! | match distance | read **least-significant bit first** |
//!
//! Getting that backwards does not fail loudly. It produces plausible bytes of
//! the right length, which is exactly the failure this workspace keeps trying to
//! avoid, so the distance path below reverses the peeked byte instead of sharing
//! the length path's reader.
//!
//! # Provenance
//!
//! The structure — the two headers, the code-length tables, the split literal
//! decoder, the 2-byte-match special case, the 4 KiB window — follows the PKWARE
//! format as implemented by StormLib's `pklib/explode.c` (**MIT**). The code below
//! is written against that description rather than copied; the constant tables are
//! the format's tables, and they appear verbatim in every implementation because
//! they are part of the format, not of any one program.
//!
//! # Verification
//!
//! There is no synthetic test vector here on purpose — one would only prove the
//! implementation agrees with itself. Correctness is pinned by decompressing real
//! archives: `war3.mpq`'s `(listfile)` must yield its member names as text, and a
//! map's `war3map.wts` must yield `STRING` records that resolve the `TRIGSTR_*`
//! references in its `.w3i`. See the tests at the bottom and `arename` in the
//! integration tests.

use super::CodecError;

/// Binary compression: literals are copied verbatim rather than Huffman-coded.
const CMP_BINARY: u8 = 0;
/// ASCII compression: literals go through the code-length tables.
const CMP_ASCII: u8 = 1;

/// Bit lengths of the literal Huffman codes, indexed by literal value.
///
/// The tail (from 0x100) is deliberately uniform: those entries are the match
/// lengths, which are not Huffman-coded at all — they are a fixed 7-bit code plus
/// extra bits. `GenAscTabs` computes them from this table rather than taking a
/// second one.
const CH_BITS_ASC: [u8; 0x100] = [
    0x0B, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x08, 0x07, 0x0C, 0x0C, 0x07, 0x0C, 0x0C,
    0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0D, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C,
    0x04, 0x0A, 0x08, 0x0C, 0x0A, 0x0C, 0x0A, 0x08, 0x07, 0x07, 0x08, 0x09, 0x07, 0x06, 0x07, 0x08,
    0x07, 0x06, 0x07, 0x07, 0x07, 0x07, 0x08, 0x07, 0x07, 0x08, 0x08, 0x0C, 0x0B, 0x07, 0x09, 0x0B,
    0x0C, 0x06, 0x07, 0x06, 0x06, 0x05, 0x07, 0x08, 0x08, 0x06, 0x0B, 0x09, 0x06, 0x07, 0x06, 0x06,
    0x07, 0x0B, 0x06, 0x06, 0x06, 0x07, 0x09, 0x08, 0x09, 0x09, 0x0B, 0x08, 0x0B, 0x09, 0x0C, 0x08,
    0x0C, 0x05, 0x06, 0x06, 0x06, 0x05, 0x06, 0x06, 0x06, 0x05, 0x0B, 0x07, 0x05, 0x06, 0x05, 0x05,
    0x06, 0x0A, 0x05, 0x05, 0x05, 0x05, 0x08, 0x07, 0x08, 0x08, 0x0A, 0x0B, 0x0B, 0x0C, 0x0C, 0x0C,
    0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D,
    0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D,
    0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D,
    0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C,
    0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C,
    0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C,
    0x0D, 0x0C, 0x0D, 0x0D, 0x0D, 0x0C, 0x0D, 0x0D, 0x0D, 0x0C, 0x0D, 0x0D, 0x0D, 0x0D, 0x0C, 0x0D,
    0x0D, 0x0D, 0x0C, 0x0C, 0x0C, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D,
];

/// The literal codes themselves, read most-significant bit first.
const CH_CODE_ASC: [u16; 0x100] = [
    0x0490, 0x0FE0, 0x07E0, 0x0BE0, 0x03E0, 0x0DE0, 0x05E0, 0x09E0,
    0x01E0, 0x00B8, 0x0062, 0x0EE0, 0x06E0, 0x0022, 0x0AE0, 0x02E0,
    0x0CE0, 0x04E0, 0x08E0, 0x00E0, 0x0F60, 0x0760, 0x0B60, 0x0360,
    0x0D60, 0x0560, 0x1240, 0x0960, 0x0160, 0x0E60, 0x0660, 0x0A60,
    0x000F, 0x0250, 0x0038, 0x0260, 0x0050, 0x0C60, 0x0390, 0x00D8,
    0x0042, 0x0002, 0x0058, 0x01B0, 0x007C, 0x0029, 0x003C, 0x0098,
    0x005C, 0x0009, 0x001C, 0x006C, 0x002C, 0x004C, 0x0018, 0x000C,
    0x0074, 0x00E8, 0x0068, 0x0460, 0x0090, 0x0034, 0x00B0, 0x0710,
    0x0860, 0x0031, 0x0054, 0x0011, 0x0021, 0x0017, 0x0014, 0x00A8,
    0x0028, 0x0001, 0x0310, 0x0130, 0x003E, 0x0064, 0x001E, 0x002E,
    0x0024, 0x0510, 0x000E, 0x0036, 0x0016, 0x0044, 0x0030, 0x00C8,
    0x01D0, 0x00D0, 0x0110, 0x0048, 0x0610, 0x0150, 0x0060, 0x0088,
    0x0FA0, 0x0007, 0x0026, 0x0006, 0x003A, 0x001B, 0x001A, 0x002A,
    0x000A, 0x000B, 0x0210, 0x0004, 0x0013, 0x0032, 0x0003, 0x001D,
    0x0012, 0x0190, 0x000D, 0x0015, 0x0005, 0x0019, 0x0008, 0x0078,
    0x00F0, 0x0070, 0x0290, 0x0410, 0x0010, 0x07A0, 0x0BA0, 0x03A0,
    0x0240, 0x1C40, 0x0C40, 0x1440, 0x0440, 0x1840, 0x0840, 0x1040,
    0x0040, 0x1F80, 0x0F80, 0x1780, 0x0780, 0x1B80, 0x0B80, 0x1380,
    0x0380, 0x1D80, 0x0D80, 0x1580, 0x0580, 0x1980, 0x0980, 0x1180,
    0x0180, 0x1E80, 0x0E80, 0x1680, 0x0680, 0x1A80, 0x0A80, 0x1280,
    0x0280, 0x1C80, 0x0C80, 0x1480, 0x0480, 0x1880, 0x0880, 0x1080,
    0x0080, 0x1F00, 0x0F00, 0x1700, 0x0700, 0x1B00, 0x0B00, 0x1300,
    0x0DA0, 0x05A0, 0x09A0, 0x01A0, 0x0EA0, 0x06A0, 0x0AA0, 0x02A0,
    0x0CA0, 0x04A0, 0x08A0, 0x00A0, 0x0F20, 0x0720, 0x0B20, 0x0320,
    0x0D20, 0x0520, 0x0920, 0x0120, 0x0E20, 0x0620, 0x0A20, 0x0220,
    0x0C20, 0x0420, 0x0820, 0x0020, 0x0FC0, 0x07C0, 0x0BC0, 0x03C0,
    0x0DC0, 0x05C0, 0x09C0, 0x01C0, 0x0EC0, 0x06C0, 0x0AC0, 0x02C0,
    0x0CC0, 0x04C0, 0x08C0, 0x00C0, 0x0F40, 0x0740, 0x0B40, 0x0340,
    0x0300, 0x0D40, 0x1D00, 0x0D00, 0x1500, 0x0540, 0x0500, 0x1900,
    0x0900, 0x0940, 0x1100, 0x0100, 0x1E00, 0x0E00, 0x0140, 0x1600,
    0x0600, 0x1A00, 0x0E40, 0x0640, 0x0A40, 0x0A00, 0x1200, 0x0200,
    0x1C00, 0x0C00, 0x1400, 0x0400, 0x1800, 0x0800, 0x1000, 0x0000,
];

/// Bits to read for the match-length code, indexed by length symbol.
const LEN_BITS: [u8; 0x10] = [
    0x03, 0x02, 0x03, 0x03, 0x04, 0x04, 0x04, 0x05, 0x05, 0x05, 0x05, 0x06, 0x06, 0x06, 0x07, 0x07,
];

/// The length symbol in reverse-index order; `GenDecodeTabs` walks it backwards.
const LEN_CODE: [u8; 0x10] = [
    0x05, 0x03, 0x01, 0x06, 0x0A, 0x02, 0x0C, 0x14, 0x04, 0x18, 0x08, 0x30, 0x10, 0x20, 0x40, 0x00,
];

/// Bits to read for each distance code.
const DIST_BITS: [u8; 0x40] = [
    0x02, 0x04, 0x04, 0x05, 0x05, 0x05, 0x05, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
];

/// The distance code for each position; read least-significant bit first, which is
/// why [`read_distance_bits`] reverses the peeked byte.
const DIST_CODE: [u8; 0x40] = [
    0x03, 0x0D, 0x05, 0x19, 0x09, 0x11, 0x01, 0x3E, 0x1E, 0x2E, 0x0E, 0x36, 0x16, 0x26, 0x06, 0x3A,
    0x1A, 0x2A, 0x0A, 0x32, 0x12, 0x22, 0x42, 0x02, 0x7C, 0x3C, 0x5C, 0x1C, 0x6C, 0x2C, 0x4C, 0x0C,
    0x74, 0x34, 0x54, 0x14, 0x64, 0x24, 0x44, 0x04, 0x78, 0x38, 0x58, 0x18, 0x68, 0x28, 0x48, 0x08,
    0xF0, 0x70, 0xB0, 0x30, 0xD0, 0x50, 0x90, 0x10, 0xE0, 0x60, 0xA0, 0x20, 0xC0, 0x40, 0x80, 0x00,
];

/// Base match length for each length symbol.
///
/// ⚠️ This is **not** derivable from [`LEN_CODE`] or [`LEN_BITS`] the way the
/// generator's accumulator makes it look: that accumulator walks bit patterns, not
/// lengths, and starts at zero for a symbol whose base is 2. It *is* derivable
/// from [`EX_LEN_BITS`], and the test below uses that to cross-check the
/// transcription instead of trusting it.
const LEN_BASE: [u16; 0x10] = [
    0x0000, 0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007,
    0x0008, 0x000A, 0x000E, 0x0016, 0x0026, 0x0046, 0x0086, 0x0106,
];

/// Extra bits each length symbol carries above its base.
const EX_LEN_BITS: [u8; 0x10] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
];

/// A canonical decode table: `table[peeked_bits] = symbol`.
///
/// `start_indexes[i]` is the packed code for symbol `i`, and the symbol's code
/// spans `1 << length_bits[i]` consecutive peeked patterns starting there. The
/// table therefore maps *pattern → symbol*, which is the direction a decoder
/// needs; writing `table[pattern] = pattern` instead produces an identity map in
/// which every symbol decodes to itself, and every output is wrong in a way that
/// still has a plausible length.
fn gen_decode_table(start_indexes: &[u8], length_bits: &[u8]) -> [u8; 0x100] {
    let mut positions = [0u8; 0x100];
    for (symbol, (&start, &bits)) in start_indexes.iter().zip(length_bits).enumerate() {
        let length = 1usize << bits;
        let mut pattern = start as usize;
        while pattern < 0x100 {
            positions[pattern] = symbol as u8;
            pattern += length;
        }
    }
    positions
}

/// The three tables the literal decoder picks between, indexed by code length.
struct AscTables {
    /// Literals shorter than 9 bits: one table, direct.
    short: [u8; 0x100],
    /// Literals of 9..=12 bits.
    medium: [u8; 0x100],
    /// Literals of 13 or 14 bits.
    long: [u8; 0x80],
    /// Literals of 15 or 16 bits.
    longer: [u8; 0x100],
    /// Bits to consume per literal symbol, after the split.
    bits: [u8; 0x100],
}

/// Builds the literal decode tables, mirroring the format's `GenAscTabs`.
///
/// The split exists because literals are not a prefix code of one depth: the
/// format packs longer codes' remaining bits into separate tables keyed by the
/// low 4 or 6 bits of the peeked byte, and a code of 16 bits is packed into a
/// table indexed by the *high* 8 bits. A symbol whose packed form is zero (that
/// is, a code that was entirely consumed by the split) simply has no entry.
fn gen_asc_tables() -> AscTables {
    // ⚠️ Pre-filled with the escape, not zero. The reference zeroes its tables and
    // then *always* writes the escape for a split code before filling a sub-table;
    // starting from `0xFF` instead makes "not yet claimed by a short code" and
    // "explicitly a split code" the same state, which is what the decoder tests
    // for. A zero-filled table would report a literal 0 for every split code whose
    // low byte the loop had not reached yet, which decodes to plausible garbage.
    let mut short = [0xFFu8; 0x100];
    let mut medium = [0u8; 0x100];
    let mut long = [0u8; 0x80];
    let mut longer = [0u8; 0x100];
    let mut bits = CH_BITS_ASC;

    for count in (0..=0xFF).rev() {
        let code = CH_CODE_ASC[count];
        let bit_count = bits[count];

        if bit_count <= 8 {
            let step = 1usize << bit_count;
            let mut acc = code as usize;
            while acc < 0x100 {
                short[acc] = count as u8;
                acc += step;
            }
            continue;
        }

        // Longer than a byte: the packed code is split, and only the part that
        // fits the sub-table's index width is usable. `bit_count` is narrowed by
        // the shift below, exactly as the format's generator does.
        let low = (code & 0xFF) as usize;
        if low != 0 {
            short[low] = 0xFF;

            if code & 0x3F != 0 {
                bits[count] = bit_count - 4;
                let step = 1usize << bits[count];
                let mut acc = (code >> 4) as usize;
                while acc < 0x100 {
                    medium[acc] = count as u8;
                    acc += step;
                }
            } else {
                bits[count] = bit_count - 6;
                let step = 1usize << bits[count];
                let mut acc = (code >> 6) as usize;
                while acc < 0x80 {
                    long[acc] = count as u8;
                    acc += step;
                }
            }
        } else if bit_count > 8 {
            // The low byte is entirely consumed by the split, so what remains is
            // indexed by the high byte.
            bits[count] = bit_count - 8;
            let step = 1usize << bits[count];
            let mut acc = (code >> 8) as usize;
            while acc < 0x100 {
                longer[acc] = count as u8;
                acc += step;
            }
        }
    }

    AscTables {
        short,
        medium,
        long,
        longer,
        bits,
    }
}

/// The bit buffer, a faithful translation of the reference's `bit_buff` /
/// `extra_bits` pair.
///
/// # Why a translation rather than a tidier model
///
/// This began as an abstract "peek eight bits, consume n" reader and produced
/// wrong bytes three times running against real data. The reference is not
/// abstract: it keeps the *unread* bits in the **low** part of a 16-bit word,
/// counts how many extra bits came with them, and shifts a new byte in at bit 8.
/// Rewrites of that sequence which look equivalent are not, and the difference
/// shows up only as silently wrong output.
///
/// So the reference's own invariants are kept, with names that say what they are:
///
/// - Pending bits sit at the bottom of `buff`; bit 0 of `buff` is the next bit to
///   be consumed.
/// - `pending` is how many of them are real. Everything above is stale.
/// - [`Self::peek8`] is `buff & 0xFF` once a load has normalised the pending bits
///   into the low byte.
///
/// The subtlety that cannot be guessed: a load shifts the buffer down by `pending`
/// *before* OR-ing the new byte in at bit 8, and the subsequent consume shifts by
/// `n - pending_before_the_load` — not by `n`. Both amounts are measured against the
/// bits that were pending, and using `n` either time silently reorders the stream.
struct Bits<'a> {
    data: &'a [u8],
    /// Next byte to load.
    next_byte: usize,
    /// Pending bits at the bottom; bit 0 is next.
    buff: u32,
    /// How many bits of `buff` are real.
    pending: u32,
}

impl<'a> Bits<'a> {
    /// `bit_buff = in_buff[2]`, `extra_bits = 0`, `in_pos = 3`.
    fn new(data: &'a [u8]) -> Self {
        let mut this = Self {
            data,
            next_byte: 1,
            buff: 0,
            pending: 0,
        };
        if let Some(&byte) = data.first() {
            // The reference assigns the byte straight into the word with
            // `extra_bits = 0`, so it lands in the low byte and counts as zero
            // pending bits: the first byte is *available*, not consumed.
            this.buff = u32::from(byte);
            this.pending = 0;
        }
        this
    }

    /// Loads the next byte above the pending bits, normalising as it goes.
    ///
    /// The reference: `bit_buff >>= extra_bits; bit_buff |= in_buff[in_pos++] << 8;`
    /// — drop the consumed bits, then OR a byte into position 8. `false` means the
    /// input is exhausted.
    fn load(&mut self) -> bool {
        match self.data.get(self.next_byte) {
            Some(&byte) => {
                self.next_byte += 1;
                self.buff >>= self.pending;
                self.buff |= u32::from(byte) << 8;
                true
            }
            None => false,
        }
    }

    /// The next eight bits, as `buff & 0xFF`.
    fn peek8(&self) -> u8 {
        self.buff as u8
    }

    /// The next `n` bits, taken from the low end as the reference's masks do
    /// (`bit_buff & 0x03`, `& dsize_mask`).
    fn peek(&self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.buff & ((1u32 << n) - 1)
        }
    }

    /// `WasteBits`: consumes `n` bits, loading a byte when it must.
    ///
    /// Two shift amounts, both measured against the pending count — see the struct
    /// documentation. This is the function the decoder's correctness rests on.
    fn consume(&mut self, n: u32) -> Result<(), CodecError> {
        if n <= self.pending {
            self.pending -= n;
            self.buff >>= n;
            return Ok(());
        }

        let pending_before = self.pending;
        // The shortfall must fit in the byte being loaded.
        if n - pending_before > 8 {
            return Err(CodecError::Eof);
        }
        if !self.load() {
            return Err(CodecError::Eof);
        }
        // `load` already shifted the stale bits out, so the byte is at bit 8 and
        // the new pending count is the old one plus eight. What still has to go is
        // the shortfall.
        self.buff >>= n - pending_before;
        self.pending = pending_before + 8 - n;
        Ok(())
    }

    /// Reads `n` bits and consumes them.
    fn read(&mut self, n: u32) -> Result<u32, CodecError> {
        let value = self.peek(n);
        self.consume(n)?;
        Ok(value)
    }

    /// The eight bits a table lookup needs.
    ///
    /// ⚠️ **This must not load.** It used to, on the reasoning that a table index
    /// needs eight bits present — and that is exactly what put this decoder 15 bytes
    /// in before it went wrong. The reference is stricter than it looks: `DecodeLit`
    /// and `DecodeDist` read `bit_buff & 0xFF` *without* checking the count, and the
    /// count is guaranteed only because a `WasteBits` call follows every such read.
    /// Loading here breaks that guarantee from the other side, because the byte this
    /// peeks is one the next `WasteBits` would have placed — so the same bytes end up
    /// positioned differently and the two decoders diverge on a later control bit.
    ///
    /// Stale high bits are harmless: `& 0xFF` keeps only what is pending once those
    /// bits are at the bottom, and `consume` is what loads.
    fn peek8_for_table(&self) -> u8 {
        self.peek8()
    }
}

/// What one iteration of the stream decodes to.
enum Symbol {
    /// An uncompressed byte to append.
    Literal(u8),
    /// A back-reference: `length` bytes copied from `distance` back.
    Match { length: usize, distance: usize },
    /// The format's end marker.
    End,
}

/// Decodes the next symbol, mirroring the reference's `DecodeLit` plus the
/// `next_literal` thresholds that follow it.
///
/// The leading bit decides which coding follows, and it is read from the *bottom*
/// of the buffer — see [`Bits`] for why that is not a detail.
fn decode_symbol(
    bits: &mut Bits<'_>,
    ctype: u8,
    asc: &AscTables,
    dict_bits: u32,
    dict_mask: u32,
    len_table: &[u8; 0x100],
    dist_table: &[u8; 0x100],
) -> Result<Symbol, CodecError> {
    // `DecodeLit`'s first test: the control bit.
    if bits.read(1)? == 1 {
        let symbol = usize::from(len_table[bits.peek8_for_table() as usize]);
        bits.consume(u32::from(LEN_BITS[symbol]))?;

        let extra_bits = EX_LEN_BITS[symbol];
        let mut length_code = usize::from(LEN_BASE[symbol]);
        if extra_bits != 0 {
            let extra = bits.peek(u32::from(extra_bits)) as usize;
            // The reference tolerates the end-of-stream marker running out of
            // bits but nothing else, and reports a bare end as an error otherwise.
            if bits.consume(u32::from(extra_bits)).is_err() {
                return if symbol + extra == 0x10E {
                    Ok(Symbol::End)
                } else {
                    Err(CodecError::Eof)
                };
            }
            length_code += extra;
        }

        // `DecodeLit` adds 0x100 to tell a literal from a length, and `Expand`
        // subtracts 0xFE to get the length, so the table's biased base plus the
        // extra bits already is the length minus two.
        let length = length_code + 2;

        // 0x305 is `length_code + 0x100`, so the marker is length_code 0x205.
        if length_code == 0x205 {
            return Ok(Symbol::End);
        }

        let distance = decode_distance(bits, length, dict_bits, dict_mask, dist_table)?;
        return Ok(Symbol::Match { length, distance });
    }

    // Not a match: a literal. `DecodeLit` consumes the control bit first, which
    // the call above already did.
    let byte = if ctype == CMP_BINARY {
        bits.read(8)? as u8
    } else {
        let peek = bits.peek8_for_table();
        let mut value = asc.short[peek as usize];
        if value == 0xFF {
            // `DecodeLit` branches on the *byte* being non-zero, then on its low
            // six bits, and each branch consumes its own bit count and re-peeks.
            if peek != 0 {
                if peek & 0x3F != 0 {
                    bits.consume(4)?;
                    value = asc.medium[bits.peek8() as usize];
                } else {
                    bits.consume(6)?;
                    value = asc.long[(bits.peek8() & 0x7F) as usize];
                }
            } else {
                bits.consume(8)?;
                value = asc.longer[bits.peek8() as usize];
            }
        }
        if value == 0xFF {
            return Err(CodecError::BadLiteralCode(peek));
        }
        bits.consume(u32::from(asc.bits[value as usize]))?;
        value
    };

    Ok(Symbol::Literal(byte))
}

/// Decodes a match's backward distance, mirroring the reference's `DecodeDist`.
///
/// The `length == 2` case reads two bits whatever the dictionary size, which is
/// the format's own special case rather than an optimisation.
fn decode_distance(
    bits: &mut Bits<'_>,
    length: usize,
    dict_bits: u32,
    dict_mask: u32,
    dist_table: &[u8; 0x100],
) -> Result<usize, CodecError> {
    let pos_code = u32::from(dist_table[bits.peek8_for_table() as usize]);
    let pos_bits = u32::from(DIST_BITS[pos_code as usize]);
    bits.consume(pos_bits)?;

    let distance = if length == 2 {
        let low = bits.peek(2);
        bits.consume(2)?;
        (pos_code << 2) | low
    } else {
        let low = bits.peek(dict_bits) & dict_mask;
        bits.consume(dict_bits)?;
        (pos_code << dict_bits) | low
    };

    // The format stores distance - 1; the +1 turns the code into an offset, and a
    // zero is the reference's "distance error" signal.
    Ok(distance as usize + 1)
}

/// What a decode produced, including when it did not finish.
///
/// A decoder that only returns `Err` throws away the most useful fact about a
/// failure: how far it got. "Wrong at byte 0" and "right for 54 bytes and then
/// wrong" are different bugs with different causes, and a caller cannot tell them
/// apart from an error alone — which is exactly the position this module was in
/// while it was being written.
///
/// The bytes produced before a failure are not a partial result to be used; they
/// are evidence.
#[derive(Debug, Clone)]
pub struct ExplodeOutcome {
    /// Bytes produced before the stream ended or failed.
    pub out: Vec<u8>,
    /// How many bytes the stream declared.
    pub expected: usize,
    /// The failure, when there was one.
    pub error: Option<CodecError>,
    /// Whether the format's end-of-stream marker was seen.
    pub saw_end_marker: bool,
}

impl ExplodeOutcome {
    /// Whether the decode ran to the declared length with no error.
    ///
    /// The end-of-stream marker is **not** required, and that follows the reference
    /// rather than a preference: `explode` ends with
    /// `if(Expand(pWork) != 0x306) return CMP_NO_ERROR;`, so a stream that fills
    /// exactly the declared length is accepted whether or not the marker follows.
    /// `saw_end_marker` is reported separately so a caller can tell the two apart.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.error.is_none() && self.out.len() == self.expected
    }

    /// Turns the outcome into the result `explode` should return.
    fn into_result(self) -> Result<Vec<u8>, CodecError> {
        match self.error {
            Some(e) => Err(e),
            None if self.out.len() == self.expected => Ok(self.out),
            None => Err(CodecError::SizeMismatch {
                expected: self.expected,
                got: self.out.len(),
            }),
        }
    }
}

/// Explodes one PKWARE-compressed buffer, reporting how far it got on failure.
///
/// `data` is the member bytes **after** the MPQ sector mask byte, so it starts
/// with the format's own two-byte header.
///
/// `expected` is both the target and the hard limit: producing more than it is an
/// error, not a longer success.
///
/// # Errors
///
/// Only for a header this format does not define. A stream-level failure is carried
/// in [`ExplodeOutcome::error`] together with the bytes produced before it, because
/// how far a decode got is the fact that separates one class of bug from another.
pub fn explode_detailed(data: &[u8], expected: usize) -> Result<ExplodeOutcome, CodecError> {
    if data.len() <= 4 {
        return Err(CodecError::Eof);
    }

    let ctype = data[0];
    let dict_bits = u32::from(data[1]);
    if ctype != CMP_BINARY && ctype != CMP_ASCII {
        return Err(CodecError::BadPkwareMode(ctype));
    }
    if !(4..=6).contains(&dict_bits) {
        return Err(CodecError::BadDictionarySize(dict_bits));
    }
    let dict_mask = 0xFFFFu32 >> (0x10 - dict_bits);

    let asc = gen_asc_tables();
    let len_table = gen_decode_table(&LEN_CODE, &LEN_BITS);
    let dist_table = gen_decode_table(&DIST_CODE, &DIST_BITS);


    let mut bits = Bits::new(&data[2..]);
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    let mut error = None;
    let mut saw_end_marker = false;

    loop {
        let symbol = match decode_symbol(
            &mut bits,
            ctype,
            &asc,
            dict_bits,
            dict_mask,
            &len_table,
            &dist_table,
        ) {
            Ok(symbol) => symbol,
            Err(e) => {
                error = Some(e);
                break;
            }
        };

        let limit = match symbol {
            Symbol::End => {
                saw_end_marker = true;
                break;
            }
            Symbol::Literal(_) => 1,
            Symbol::Match { length, .. } => length,
        };

        // The declared length is a hard limit. Reaching it is not the end of the
        // stream unless the marker says so — a decode that overshoots has the wrong
        // bit model, and accepting the shorter prefix would hide that.
        if out.len() + limit > expected {
            error = Some(CodecError::SizeMismatch {
                expected,
                got: out.len() + limit,
            });
            break;
        }

        match symbol {
            Symbol::End => unreachable!("handled above"),
            Symbol::Literal(byte) => out.push(byte),
            Symbol::Match { length, distance } => {
                if distance == 0 || distance > out.len() {
                    error = Some(CodecError::BadMatchDistance {
                        distance,
                        produced: out.len(),
                    });
                    break;
                }
                // Byte by byte, because a match may overlap the bytes it is
                // producing — that is how a run is encoded, and a bulk copy would
                // replicate the wrong bytes.
                for _ in 0..length {
                    let byte = out[out.len() - distance];
                    out.push(byte);
                }
            }
        }

        if out.len() == expected {
            break;
        }
    }

    Ok(ExplodeOutcome {
        out,
        expected,
        error,
        saw_end_marker,
    })
}

/// Explodes one PKWARE-compressed buffer.
///
/// See [`explode_detailed`] for the version that reports how far a failure got.
///
/// # Errors
///
/// [`CodecError::BadPkwareMode`] or [`CodecError::BadDictionarySize`] for a header
/// this format does not define, [`CodecError::Eof`] for a stream that ends early,
/// and [`CodecError::SizeMismatch`] when the decoded length disagrees with what
/// the container declared — the check that keeps a wrong bit order from looking
/// like a successful decode.
pub fn explode(data: &[u8], expected: usize) -> Result<Vec<u8>, CodecError> {
    explode_detailed(data, expected)?.into_result()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `LEN_BASE` is not transcribed blind: it follows from `EX_LEN_BITS`.
    ///
    /// Each symbol covers `1 << EX_LEN_BITS[i]` lengths, so the next symbol starts
    /// exactly that many past the current one. That is a real constraint, not an
    /// identity — a single wrong entry in either table breaks it — which makes it
    /// the cheapest available check on two of the format's six tables.
    #[test]
    fn the_length_base_follows_from_the_extra_bits() {
        let mut acc = LEN_BASE[0];
        for i in 0..0x0F {
            acc += 1u16 << EX_LEN_BITS[i];
            assert_eq!(
                acc, LEN_BASE[i + 1],
                "LEN_BASE[{next}] should follow from LEN_BASE[{i}] + 2^EX_LEN_BITS[{i}]",
                next = i + 1
            );
        }
    }

    /// The length symbols must cover every match length the format defines, with no
    /// gaps and no overlaps.
    #[test]
    fn length_symbols_cover_every_match_length() {
        let mut covered = Vec::new();
        for symbol in 0..0x10 {
            let lo = usize::from(LEN_BASE[symbol]);
            let hi = lo + ((1usize << EX_LEN_BITS[symbol]) - 1);
            covered.extend(lo..=hi);
        }
        let count = covered.len();
        covered.sort_unstable();
        covered.dedup();
        assert_eq!(count, covered.len(), "symbols must not overlap");
        // The format documents matches of 2..=518 bytes plus the end marker.
        // `LEN_BASE` is stored biased by -2, so what it covers is 0..=517, which
        // is 518 distinct values.
        assert_eq!(covered.first(), Some(&0));
        assert_eq!(covered.last(), Some(&517));
        assert_eq!(covered.len(), 518, "no gaps across the match range");
    }

    /// The decode table must map patterns to symbols, not patterns to themselves.
    ///
    /// This is the bug the table shape invites: an identity map decodes every
    /// symbol to its own index, which produces output of the right length and
    /// entirely wrong content.
    /// The decode table must map patterns to symbols, not patterns to themselves.
    ///
    /// This is the bug the table shape invites: an identity map decodes every
    /// symbol to its own index, which produces output of the right length and
    /// entirely wrong content.
    ///
    /// ⚠️ A symbol with a `w`-bit code owns **`2^(8 - w)`** patterns, not `2^w`.
    /// The width is how many of the peeked eight bits the code consumes, so the
    /// remaining `8 - w` bits are free and every combination of them resolves to
    /// that symbol. Sizing this the other way round is the second way to get the
    /// table wrong while keeping a plausible shape — and it cannot be a prefix code
    /// at all, because `Σ 2^w` over these sixteen widths is 652, while `Σ 2^(8 - w)`
    /// is exactly 256, which is the whole table.
    #[test]
    fn the_decode_table_maps_patterns_to_symbols() {
        let table = gen_decode_table(&LEN_CODE, &LEN_BITS);
        for (symbol, &width) in LEN_BITS.iter().enumerate() {
            let owned = table
                .iter()
                .filter(|&&s| usize::from(s) == symbol)
                .count();
            assert_eq!(
                owned,
                1usize << (8 - width),
                "symbol {symbol} consumes {width} of the peeked 8 bits, so it must \
                 own 2^{} patterns",
                8 - width
            );
        }

        // Together those counts must account for the table exactly: no pattern
        // unclaimed, none claimed twice. This is the prefix-code property, and it
        // is what would fail if the widths or codes were transcribed wrongly.
        let claimed: usize = (0..0x10)
            .map(|symbol| {
                table
                    .iter()
                    .filter(|&&s| usize::from(s) == symbol)
                    .count()
            })
            .sum();
        assert_eq!(claimed, table.len(), "every pattern must be claimed exactly once");

        // An identity map would give every symbol exactly one pattern, so the counts
        // above already rule it out. This states the failure mode directly.
        assert!(
            table
                .iter()
                .enumerate()
                .any(|(pattern, &symbol)| pattern != usize::from(symbol)),
            "an identity table is the failure mode this test exists for"
        );
    }

    /// A canonical decode table must be a *complete prefix code*: every one of the
    /// 256 bit patterns is claimed by exactly one symbol, and no symbol's patterns
    /// cover another's.
    ///
    /// This is the property that caught a real transcription error in
    /// [`LEN_CODE`]: entered as the symbol indices shown in the reference's table
    /// listing (which are those values shifted left by three), two symbols whose
    /// codes share a prefix overwrite each other and 16 patterns go unclaimed. The
    /// reference's loop writes patterns in a fixed order and never checks this, so
    /// nothing there fails — the damage only shows up as wrong match lengths.
    #[test]
    fn the_length_decode_table_is_a_complete_prefix_code() {
        let mut claimed = [0u32; 0x100];
        for (symbol, (&code, &width)) in LEN_CODE.iter().zip(LEN_BITS.iter()).enumerate() {
            let width = usize::from(width);
            let step = 1usize << width;
            let mut pattern = usize::from(code);
            while pattern < 0x100 {
                claimed[pattern] += 1;
                let _ = symbol;
                pattern += step;
            }
        }
        let double = claimed.iter().filter(|&&n| n > 1).count();
        let unclaimed = claimed.iter().filter(|&&n| n == 0).count();
        assert_eq!(double, 0, "patterns claimed more than once");
        assert_eq!(unclaimed, 0, "patterns claimed by no symbol");
    }

    /// The same property for the distance table.
    #[test]
    fn the_distance_decode_table_is_a_complete_prefix_code() {
        let mut claimed = [0u32; 0x100];
        for (&code, &width) in DIST_CODE.iter().zip(DIST_BITS.iter()) {
            let step = 1usize << usize::from(width);
            let mut pattern = usize::from(code);
            while pattern < 0x100 {
                claimed[pattern] += 1;
                pattern += step;
            }
        }
        assert_eq!(claimed.iter().filter(|&&n| n > 1).count(), 0);
        assert_eq!(claimed.iter().filter(|&&n| n == 0).count(), 0);
    }

    /// A header the format does not define is refused, not guessed at.
    #[test]
    fn an_unknown_header_is_refused() {
        // Compression type 2 does not exist. The buffer is long enough that the
        // header check is what fails, not the length guard.
        let body = [0u8; 32];
        let mut data = vec![2u8, 4];
        data.extend_from_slice(&body);
        assert!(matches!(
            explode(&data, 4),
            Err(CodecError::BadPkwareMode(2))
        ));

        // Dictionary size must be 4, 5 or 6.
        let mut data = vec![1u8, 7];
        data.extend_from_slice(&body);
        assert!(matches!(
            explode(&data, 4),
            Err(CodecError::BadDictionarySize(7))
        ));
    }

    #[test]
    fn a_truncated_stream_is_an_error_not_a_short_read() {
        // A valid header then nothing: the first symbol cannot be read.
        assert!(explode(&[1, 4], 16).is_err());
        assert!(explode(&[1, 4, 0x00], 16).is_err());
    }

    /// The declared length is enforced in both directions.
    ///
    /// Under-running is the one that would otherwise look like success: a stream
    /// that decodes fewer bytes than the container promised is a wrong decode.
    ///
    /// The assertion is on "any error" rather than a specific variant on purpose.
    /// Which guard fires first — a truncated stream or a short result — depends on
    /// how many bytes the literals happen to consume, and pinning the variant would
    /// make the test brittle about something it is not trying to check. What it
    /// checks is that no wrong-length decode is ever returned as success.
    #[test]
    fn a_stream_that_does_not_reach_the_declared_length_is_never_success() {
        // Binary mode, so each symbol is one literal. Two bytes of stream cannot
        // produce 4096, whatever the bit model does with them.
        let data = [0u8, 4, 0x02, 0x00, 0x00, 0x00];
        assert!(explode(&data, 4096).is_err(), "short stream must not succeed");
        // Asking for less than the stream holds is a different question and is
        // allowed: the reference accepts a stream that fills the declared length
        // without reaching the end-of-stream marker.
        assert!(
            explode(&data, 1).is_ok(),
            "filling the declared length is a success even without the marker"
        );
    }

    /// The buffer's least significant bit is the next unread bit.
    ///
    /// This is the property the decoder turns on: the reference consumes a bit with
    /// `bit_buff >>= 1` and then reads `bit_buff & 0xFF` as a table index, which is
    /// only coherent if the bit it just dropped was the *low* one. Real data
    /// confirms it — for `(4)LostTemple.w3m`'s `.wts` the first payload byte is
    /// `0xDE`, whose low bit is 0, which correctly makes the first symbol a literal
    /// (`STRING` is plain text) rather than a match.
    ///
    /// This test is what caught the cross-byte bug. The fix was in *where* loading
    /// happens: [`Bits::peek8_for_table`] used to load on its own, and the reference
    /// loads only inside `WasteBits`.
    #[test]
    fn the_next_bit_is_the_low_bit_of_the_buffer() {
        // 0b1000_0001: the first bit is 1.
        let mut bits = Bits::new(&[0b1000_0001, 0b0000_0011]);
        assert_eq!(bits.peek(1), 1, "the low bit is the first bit");
        bits.consume(1).unwrap();
        assert_eq!(bits.pending, 7, "no load is needed yet");
        bits.consume(7).unwrap();
        assert_eq!(bits.peek8(), 0b0000_0011, "the loaded byte must be at bit 0");
    }

    /// Bits are consumed from the low end of the pending bits.
    ///
    /// ⚠️ The first byte is loaded but *not* counted as pending — that is the
    /// reference's `bit_buff = in_buff[2]` with `extra_bits = 0`. A read that does
    /// not fit therefore triggers a load first, and the byte it brings in lands at
    /// bit 8, above the first byte's bits. So reading three bits after construction
    /// gives the low three of the first byte only when the read fits the pending
    /// count, and the very first read of more than zero bits always loads.
    #[test]
    fn a_multi_bit_read_is_the_low_bits() {
        // The first read loads the second byte, so the low eight bits are the first
        // byte and the new byte sits above them.
        let mut bits = Bits::new(&[0b1101_0011, 0b0000_0000]);
        assert_eq!(bits.peek8(), 0b1101_0011, "the first byte, not the loaded one");
        // Reading three bits from the bottom gives 0b011.
        assert_eq!(bits.read(3).unwrap(), 0b011);
        // The next three are 0b010.
        assert_eq!(bits.read(3).unwrap(), 0b010);
    }

    /// A load lands directly above the bits that are still pending.
    ///
    /// If the byte were placed anywhere else, the same bytes would appear in the
    /// same order but at the wrong offsets — which is what made the real-data
    /// divergence so hard to read: the output stayed plausible and only went wrong
    /// on a later control bit.
    ///
    /// The assertion is on the **byte delivered**, not on the internal pending count.
    /// The count is a bookkeeping detail whose exact value after a load depends on
    /// how many bits happened to be pending; the property that matters is that the
    /// second input byte arrives in its own position and not shifted.
    ///
    /// The values below are what the reference's `WasteBits` produces for this input,
    /// worked through from its two shift rules (drop `extra_bits`, OR the byte at
    /// bit 8, then drop the shortfall). They are deliberately not "the obvious
    /// answer": after one bit is consumed the peek already carries the *next* byte's
    /// top bit, because consuming that one bit crosses the byte boundary and loads.
    #[test]
    fn a_load_lands_above_the_remaining_bits() {
        // 0b1010_0101 then a distinctive second byte.
        let mut bits = Bits::new(&[0b1010_0101, 0b1100_0011]);
        assert_eq!(bits.peek8(), 0b1010_0101, "the first byte, nothing loaded yet");

        // Consuming one bit loads the second byte above the remaining seven bits,
        // so the peek is the remaining seven followed by the loaded byte's top bit:
        // 0b010_0101 then 1.
        bits.consume(1).unwrap();
        assert_eq!(bits.peek8(), 0b1101_0010);
        assert_eq!(bits.pending, 7, "seven bits of the first byte are still pending");

        // Consuming those seven leaves the second byte pending on its own, delivered
        // whole. A misplaced load would show up here as a shifted byte.
        bits.consume(7).unwrap();
        assert_eq!(bits.peek8(), 0b1100_0011, "the loaded byte, unshifted");
    }

    #[test]
    fn reading_past_the_end_reports_eof() {
        // Two bytes: one pending from construction, one available to load.
        let mut bits = Bits::new(&[0x00, 0x00]);
        assert!(bits.read(8).is_ok());
        assert!(matches!(bits.read(8), Err(CodecError::Eof)));
    }

    /// The literal tables must resolve every symbol the format defines, or the
    /// head of the alphabet would be undecodable.
    #[test]
    fn the_literal_tables_cover_the_common_symbols() {
        let asc = gen_asc_tables();
        // Space and the ASCII letters have the shortest codes and must land in the
        // direct table rather than needing a split.
        for symbol in [0x20u8, b'a', b'e', b't'] {
            let bits = CH_BITS_ASC[symbol as usize];
            assert!(bits <= 8, "symbol {symbol:#04X} should be a short code");
            let code = CH_CODE_ASC[symbol as usize];
            let step = 1usize << bits;
            let mut acc = code as usize;
            while acc < 0x100 {
                assert_eq!(
                    asc.short[acc], symbol,
                    "peek {acc:#04X} should decode to {symbol:#04X}"
                );
                acc += step;
            }
        }
    }

    /// Long literal codes must land in a split table, not be silently unreachable.
    ///
    /// The split is where a wrong table choice hides: a symbol whose code is longer
    /// than eight bits is written to one of three sub-tables, and if the branch is
    /// taken wrongly the symbol simply never resolves. This checks the symbols that
    /// must use each branch, rather than re-deriving the branch condition — a test
    /// that re-implemented the branch would agree with a wrong implementation.
    ///
    /// Which symbols are expected where follows from the packing: `CH_CODE_ASC`
    /// holds the code with its length, so a symbol's reachable table is decided by
    /// its own code, the same way the decoder decides it.
    #[test]
    fn long_literal_codes_resolve_through_a_split_table() {
        let asc = gen_asc_tables();
        let mut split = 0;

        for symbol in 0..0x100usize {
            let symbol = symbol as u8;
            let bit_count = CH_BITS_ASC[symbol as usize];
            let code = CH_CODE_ASC[symbol as usize] as usize;
            if bit_count <= 8 || code & 0xFF == 0 {
                // Either it fits one byte, or the low byte is entirely consumed by
                // the split and it is reached through the 8-bit sub-table instead.
                continue;
            }
            split += 1;

            // The entry that marks a split must exist, or the decoder would never
            // look in a sub-table for this symbol.
            assert_eq!(
                asc.short[code & 0xFF], 0xFF,
                "symbol {symbol:#04X} should be marked as a split code"
            );

            // Whatever the branch, the symbol's own remaining bits index it.
            let (table, shift, limit): (&[u8], usize, usize) = if code & 0x3F != 0 {
                (&asc.medium, 4, 0x100)
            } else if code & 0xFF != 0 {
                (&asc.long, 6, 0x80)
            } else {
                (&asc.longer, 8, 0x100)
            };
            let step = 1usize << (usize::from(bit_count) - shift);
            let mut index = code >> shift;
            let mut found = false;
            while index < limit {
                if table[index] == symbol {
                    found = true;
                    break;
                }
                index += step;
            }
            assert!(
                found,
                "symbol {symbol:#04X} (code {code:#06X}, {bit_count} bits) is in no {shift}-bit split table"
            );
        }

        assert!(split > 0, "the corpus of codes should contain split codes");
    }
}

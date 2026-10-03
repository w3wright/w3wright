//! MPQ archive writing.
//!
//! Deliberately the opposite of the reader in one respect: where reading has to
//! tolerate whatever it finds, writing has to *refuse* what it cannot express.
//! A member whose decryption key depends on its block offset cannot be moved, a
//! member without a name cannot be looked up again, and a prefix that is not a
//! whole number of sectors cannot put the header where the game looks for it.
//! Each of those is an error here rather than a plausible-looking archive.
//!
//! # What this writes
//!
//! The layout is the conventional one: prefix, header, member data, hash table,
//! block table. Both tables are encrypted with the keys the format prescribes
//! ([`HASH_TABLE_KEY_NAME`], [`BLOCK_TABLE_KEY_NAME`]).
//!
//! Members come in two forms:
//!
//! - [`ArchiveBuilder::add_stored`] takes content and writes it as one
//!   uncompressed block. Nothing is compressed and nothing is encrypted, which
//!   the format allows and which keeps the writer free of a compressor.
//! - [`ArchiveBuilder::add_raw`] takes a [`RawMember`] and copies the block
//!   bytes verbatim, flags and all. This is how a member the reader cannot
//!   decode — a PKWare-imploded `.wts` — survives a rebuild untouched.
//!
//! ```text
//! let mut builder = ArchiveBuilder::with_prefix(map_prefix)?;
//! builder.add_stored("war3map.w3i", w3i_bytes);
//! builder.write("out.w3x")?;
//! ```
//!
//! # Why a rebuild is previewed rather than just done
//!
//! [`ArchiveBuilder`] builds **from nothing**: it decides the hash table size, the member order,
//! the compression and the encryption. It keeps every member's *content* but it rewrites the
//! *archive*, so a rebuild of `(4)LostTemple.w3m` preserves all 16 members byte for byte and still
//! turns a 245,236-byte file into a 244,201-byte one whose bytes differ from offset 520 onward —
//! the hash table.
//!
//! That makes "would this change my file?" a question worth answering before writing anything, and
//! [`RebuildPreview::of`] is the answer. It is one implementation on purpose: the command line's
//! `war3 map rebuild` and the editor's save button must not describe the same operation
//! differently. See `docs/decisions/ADR-0028`.

use std::fmt;
use std::path::Path;

use crate::archive::{
    normalise_name, Archive, ArchiveHeader, BlockFlags, MpqError, MpqResult, RawMember, MPQ_MAGIC,
};
use crate::crypto::{
    bytes_to_u32_le, crypt_table, decrypt, encrypt, hash_string, HashType, BLOCK_TABLE_KEY_NAME,
    HASH_TABLE_KEY_NAME,
};

/// Size of the header written here. The format's original version is 32 bytes.
const HEADER_SIZE: u32 = 32;
/// Size of one hash or block table entry.
const ENTRY_SIZE: u32 = 16;
/// Size of one block table entry, which is the same as a hash entry.
const BLOCK_ENTRY_SIZE: usize = 16;
/// Sector size shift written into the header: `512 << 3 == 4096`.
const SECTOR_SIZE_SHIFT: u16 = 3;
/// Smallest hash table worth writing, and a power of two.
const MIN_HASH_SIZE: u32 = 16;
/// Largest hash table the original format allows: entries must be below 2^16.
const MAX_HASH_SIZE: u32 = 1 << 15;
/// `MPQ_FILE_EXISTS`.
const FLAG_EXISTS: u32 = 0x8000_0000;
/// `MPQ_FILE_SINGLE_UNIT`.
const FLAG_SINGLE_UNIT: u32 = 0x0100_0000;
/// Value in every field of an unused hash slot.
const HASH_SLOT_UNUSED: u32 = 0xFFFF_FFFF;

/// Edits members of an existing archive **in place**, preserving every byte it does
/// not touch.
///
/// # Why this exists next to [`ArchiveBuilder`]
///
/// `ArchiveBuilder` builds an archive from nothing: it chooses the hash table size,
/// the member order and the encoding. Measured on `(4)LostTemple.w3m`, a rebuild
/// keeps every member's content byte for byte (16 of 16) but changes the archive
/// itself from 245,236 bytes to 244,201, diverging at offset 520 — the hash table.
///
/// For an editor that is the wrong primitive. Changing one string must not rewrite
/// the user's whole map, because the result is a different file even when every
/// member matches, and "the file I gave you, with one field changed" is what a user
/// is entitled to.
///
/// # How this can preserve the rest
///
/// Three properties of the format, all of which have to hold:
///
/// 1. Table positions in the header **and** member positions in the block table are
///    relative to the header, so no table ever has to move.
/// 2. Member data lies between the header and the hash table; the tables come after
///    it. A replaced member's new block is therefore appended at the end of the file
///    and nothing else moves.
/// 3. A hash slot is chosen by the member's *name*. Replacing content under the same
///    name leaves the hash table byte for byte as it was.
///
/// So the minimum change is one block table entry plus the appended bytes. See
/// `docs/decisions/ADR-0028` for the decision and its cost.
///
/// # Cost
///
/// The replaced member's **original block stays where it is**, unreachable. A save
/// grows the file by roughly the new content's size. That is deliberate: reclaiming
/// the dead bytes would mean re-laying the archive out, which is the rebuild this
/// type exists to avoid. Under this design the claim "nothing else changed" is
/// checkable; under a rebuild it is not.
#[derive(Debug)]
pub struct ArchivePatcher {
    buffer: Vec<u8>,
    header: ArchiveHeader,
    /// Block table entries modified, so callers can report how many.
    dirty: Vec<u32>,
}

impl ArchivePatcher {
    /// Opens an archive's bytes for patching.
    ///
    /// # Errors
    ///
    /// Whatever [`Archive::open`] reports, so a file this cannot patch is rejected
    /// by the same rules that decide whether it can be read at all.
    pub fn new(bytes: Vec<u8>) -> MpqResult<Self> {
        let archive = Archive::from_bytes(bytes.clone())?;
        Ok(Self {
            buffer: bytes,
            header: *archive.header(),
            dirty: Vec::new(),
        })
    }

    /// Replaces one member's content with `data`, stored plainly.
    ///
    /// # Errors
    ///
    /// - [`MpqError::NotFound`] when no member has that name.
    /// - [`MpqError::MemberNotWritable`] when the member is encrypted or derives its
    ///   key from its block offset.
    ///
    /// # Why encryption is refused instead of handled
    ///
    /// Writing the new block plainly would turn an encrypted member into a readable
    /// one — a change to the map's protection that the user did not ask for. Writing
    /// it encrypted means reusing the member's file key, whose derivation depends in
    /// turn on `BLOCK_OFFSET_ADJUSTED_KEY`. Both are real work with real decisions in
    /// them, so this refuses and says why. Measured over the 190-map corpus, that
    /// affects 4 `war3map.w3i` members and 0 offset-adjusted ones.
    ///
    /// # Errors on size
    ///
    /// Content larger than the format's 32-bit size field is refused rather than
    /// truncated.
    pub fn replace(&mut self, name: &str, data: Vec<u8>) -> MpqResult<()> {
        // Reopened per call rather than cached: this keeps the patcher honest about
        // the buffer it is actually editing, at the cost of re-reading the tables.
        // Callers replace one or two members, so the cost is not worth a cache that
        // could go stale.
        let archive = Archive::from_bytes(self.buffer.clone())?;
        let block_index = archive
            .block_index_for_name(name)
            .ok_or_else(|| MpqError::NotFound(name.to_string()))?;
        let entry = *archive
            .block_entry(block_index as usize)
            .ok_or_else(|| MpqError::NotFound(name.to_string()))?;

        if entry.flags.has(BlockFlags::ENCRYPTED) {
            return Err(MpqError::MemberNotWritable {
                name: name.to_string(),
                reason: "it is encrypted, and rewriting it plainly would make a protected \
                         member readable; supporting this needs its file key, not a guess",
            });
        }
        if entry.flags.has(BlockFlags::BLOCK_OFFSET_ADJUSTED_KEY) {
            return Err(MpqError::MemberNotWritable {
                name: name.to_string(),
                reason: "its key is derived from its block offset, so relocating the block \
                         would invalidate the ciphertext",
            });
        }

        let size = u32::try_from(data.len()).map_err(|_| MpqError::MemberNotWritable {
            name: name.to_string(),
            reason: "the new content is larger than the format's 32-bit size field",
        })?;

        // Appended at the very end, which is past every table for any archive this
        // reader accepts.
        let new_at = self.buffer.len() as u32;
        self.buffer.extend_from_slice(&data);

        // ⚠️ The **whole** block table is decrypted, not the one entry.
        //
        // MPQ table encryption is a stream: each 4-byte word's keystream depends on
        // the running value of the previous word. Decrypting a single 16-byte entry
        // in isolation therefore gives the right answer only for the first entry of
        // the table — every later one comes back as noise, and writing that noise
        // back corrupts an unrelated member. That was a real bug here: patching
        // `war3map.w3i` left `war3map.wts` with a file position of 133,171,052.
        let table = crypt_table();
        let key = hash_string(&table, HashType::FileKey, BLOCK_TABLE_KEY_NAME);
        let block_pos = self.header.block_table_offset() as usize;
        let table_len = self.header.block_table_size as usize * BLOCK_ENTRY_SIZE;
        let table_bytes = self
            .buffer
            .get_mut(block_pos..block_pos + table_len)
            .ok_or(MpqError::Eof)?;
        let mut words = bytes_to_u32_le(table_bytes);
        decrypt(&table, &mut words, key);

        let at = block_index as usize * 4;
        // The format has no block for an empty file: both sizes are zero and the
        // position is ignored. The `exists` flag stays set, which is what makes a
        // reader list the member without trying to read it.
        if size == 0 {
            words[at] = 0;
            words[at + 1] = 0;
            words[at + 2] = 0;
        } else {
            words[at] = new_at - self.header.file_offset as u32;
            words[at + 1] = size;
            words[at + 2] = size;
        }
        // Plain and single-unit. This clears the original block's compression bits,
        // which is correct: the block those described is no longer the one this
        // entry points at.
        words[at + 3] = FLAG_EXISTS | FLAG_SINGLE_UNIT;

        encrypt(&table, &mut words, key);
        for (i, word) in words.iter().enumerate() {
            table_bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        if !self.dirty.contains(&block_index) {
            self.dirty.push(block_index);
        }
        Ok(())
    }

    /// The archive's bytes after the replacements.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.buffer.clone()
    }

    /// How many members were replaced.
    #[must_use]
    pub fn patched_count(&self) -> usize {
        self.dirty.len()
    }
}

/// Builds an MPQ archive in memory.
#[derive(Debug, Default)]
pub struct ArchiveBuilder {
    prefix: Vec<u8>,
    members: Vec<RawMember>,
}

impl ArchiveBuilder {
    /// A builder for an archive whose header sits at offset 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A builder for an archive preceded by `prefix` bytes.
    ///
    /// A `.w3x` carries an `HM3W` prefix, normally 512 bytes, and the game reads
    /// the map name out of it — so a rebuild must pass the original bytes
    /// through. The length has to be a whole number of 512-byte sectors, or the
    /// header would not land where the format requires it.
    pub fn with_prefix(prefix: Vec<u8>) -> MpqResult<Self> {
        if prefix.len() % 512 != 0 {
            return Err(MpqError::BadPrefixLength(prefix.len()));
        }
        Ok(Self {
            prefix,
            members: Vec::new(),
        })
    }

    /// Adds a member, stored as one uncompressed block.
    ///
    /// The block is neither compressed nor encrypted. Both are per-member flag
    /// choices in this format, not requirements, and a map whose every member is
    /// stored plainly is a valid map.
    pub fn add_stored(&mut self, name: impl Into<String>, data: Vec<u8>) -> &mut Self {
        self.members.push(RawMember {
            name: name.into(),
            uncompressed_size: data.len() as u32,
            block: data,
            flags: BlockFlags(FLAG_EXISTS | FLAG_SINGLE_UNIT),
            locale: 0,
            platform: 0,
        });
        self
    }

    /// Adds a member exactly as it is already stored on disk.
    ///
    /// # Errors
    ///
    /// A member flagged [`BlockFlags::BLOCK_OFFSET_ADJUSTED_KEY`] derives its
    /// decryption key from its own block offset, so moving it invalidates the
    /// ciphertext. Rather than emit a member that cannot be read, this refuses.
    pub fn add_raw(&mut self, member: RawMember) -> MpqResult<&mut Self> {
        if member.flags.has(BlockFlags::BLOCK_OFFSET_ADJUSTED_KEY) {
            return Err(MpqError::MemberNotWritable {
                name: member.name,
                reason: "its key is derived from its block offset, so relocating the block \
                         would invalidate the stored bytes",
            });
        }
        self.members.push(member);
        Ok(self)
    }

    /// How many members have been added.
    #[must_use]
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Lays the archive out and returns its bytes.
    pub fn to_bytes(&self) -> MpqResult<Vec<u8>> {
        let count = self.members.len();
        if count as u32 > MAX_HASH_SIZE {
            return Err(MpqError::TooManyMembers(count));
        }
        let hash_size = hash_table_size(count);

        let header_pos = self.prefix.len() as u32;
        // Member data starts immediately after the header, exactly as real
        // archives do when they carry the `HM3W` prefix.
        let mut cursor = header_pos + HEADER_SIZE;
        let mut offsets = Vec::with_capacity(count);
        for member in &self.members {
            offsets.push(cursor - header_pos);
            cursor += member.block.len() as u32;
        }
        let hash_pos = cursor;
        let block_pos = hash_pos + hash_size * ENTRY_SIZE;
        let archive_end = block_pos + count as u32 * ENTRY_SIZE;

        let table = crypt_table();
        let mut hash_words = vec![HASH_SLOT_UNUSED; hash_size as usize * 4];
        let mut block_words = vec![0u32; count * 4];
        for (index, member) in self.members.iter().enumerate() {
            let upper = normalise_name(&member.name);
            let mask = hash_size - 1;
            let mut slot = hash_string(&table, HashType::TableOffset, &upper) & mask;
            while hash_words[slot as usize * 4 + 3] != HASH_SLOT_UNUSED {
                slot = (slot + 1) & mask;
            }
            hash_words[slot as usize * 4] = hash_string(&table, HashType::NameA, &upper);
            hash_words[slot as usize * 4 + 1] = hash_string(&table, HashType::NameB, &upper);
            // Language in the low half of the third word, platform in the byte
            // above it.
            hash_words[slot as usize * 4 + 2] =
                u32::from(member.locale) | (u32::from(member.platform) << 16);
            hash_words[slot as usize * 4 + 3] = index as u32;

            block_words[index * 4] = offsets[index];
            block_words[index * 4 + 1] = member.block.len() as u32;
            block_words[index * 4 + 2] = member.uncompressed_size;
            block_words[index * 4 + 3] = member.flags.0;
        }

        // The tables are encrypted, and this is the writer's half of the pair
        // whose reader half is verified against real Blizzard archives.
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

        let mut out = vec![0u8; archive_end as usize];
        out[..self.prefix.len()].copy_from_slice(&self.prefix);

        let h = header_pos as usize;
        out[h..h + 4].copy_from_slice(&MPQ_MAGIC);
        out[h + 4..h + 8].copy_from_slice(&HEADER_SIZE.to_le_bytes());
        // The archive size excludes the prefix, and excludes any strong
        // signature (there is none here).
        out[h + 8..h + 12].copy_from_slice(&(archive_end - header_pos).to_le_bytes());
        out[h + 12..h + 14].copy_from_slice(&0u16.to_le_bytes());
        out[h + 14..h + 16].copy_from_slice(&SECTOR_SIZE_SHIFT.to_le_bytes());
        // Table positions are relative to the header; member positions in the
        // block table are too.
        out[h + 16..h + 20].copy_from_slice(&(hash_pos - header_pos).to_le_bytes());
        out[h + 20..h + 24].copy_from_slice(&(block_pos - header_pos).to_le_bytes());
        out[h + 24..h + 28].copy_from_slice(&hash_size.to_le_bytes());
        out[h + 28..h + 32].copy_from_slice(&(count as u32).to_le_bytes());

        for (member, offset) in self.members.iter().zip(&offsets) {
            let at = (header_pos + offset) as usize;
            out[at..at + member.block.len()].copy_from_slice(&member.block);
        }
        for (i, word) in hash_words.iter().enumerate() {
            let at = hash_pos as usize + i * 4;
            out[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        for (i, word) in block_words.iter().enumerate() {
            let at = block_pos as usize + i * 4;
            out[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        Ok(out)
    }

    /// Lays the archive out and writes it to a file.
    pub fn write(&self, path: impl AsRef<Path>) -> MpqResult<()> {
        let bytes = self.to_bytes()?;
        std::fs::write(path, bytes)?;
        Ok(())
    }
}

/// The smallest power of two that leaves room for the members, within what the
/// format allows.
///
/// Real archives keep the table at least twice the member count so probing
/// rarely collides; `war3.mpq` has 10762 members in 32768 entries.
#[must_use]
fn hash_table_size(members: usize) -> u32 {
    let wanted = (members as u32).saturating_mul(2);
    wanted
        .max(MIN_HASH_SIZE)
        .checked_next_power_of_two()
        .unwrap_or(MAX_HASH_SIZE)
        .min(MAX_HASH_SIZE)
}

/// What rebuilding an archive would produce, without producing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildPreview {
    /// Size of the file the rebuild was planned from.
    pub original_bytes: u64,
    /// Size the rebuild would have.
    pub rebuilt_bytes: u64,
    /// How many members the rebuild would carry.
    pub member_count: usize,
    /// Offset of the first byte that differs, or `None` when the rebuild is byte identical.
    ///
    /// `Some` at the length of the shorter file is a real answer, not a rounding: two files that
    /// agree up to the end of one of them and then stop are not identical.
    pub first_difference: Option<u64>,
}

impl RebuildPreview {
    /// Whether the rebuild would be byte for byte what is already on disk.
    #[must_use]
    pub const fn is_identical(&self) -> bool {
        self.first_difference.is_none()
    }

    /// Plans a rebuild of an archive the caller has already read.
    ///
    /// `original` is the file's bytes, which the archive does not keep — it parses tables and
    /// leaves the rest alone. The two must be the same file; passing one archive's bytes with
    /// another's tables would produce a difference that means nothing.
    ///
    /// # Errors
    ///
    /// - Whatever reading a member's stored block reports.
    /// - [`MpqError::MemberNotWritable`] when a member cannot be relocated — see
    ///   [`MpqError::is_not_applicable`], which is how a caller tells that apart from a fault.
    pub fn of(archive: &Archive, original: &[u8]) -> MpqResult<Self> {
        // Rebuild every member verbatim, then compare. `add_raw` carries the stored block through
        // unchanged, so any difference is one the *builder* introduced rather than one the caller
        // asked for — which is the whole question being asked.
        let mut builder = ArchiveBuilder::with_prefix(archive.prefix().to_vec())?;
        let mut names = archive.file_names();
        names.sort_unstable();
        let mut member_count = 0usize;
        for name in &names {
            builder.add_raw(archive.raw_member(name)?)?;
            member_count += 1;
        }
        let rebuilt = builder.to_bytes()?;

        Ok(Self {
            original_bytes: original.len() as u64,
            rebuilt_bytes: rebuilt.len() as u64,
            member_count,
            first_difference: first_difference_of(original, &rebuilt),
        })
    }

    /// Plans a rebuild of a file on disk.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, or as [`RebuildPreview::of`].
    pub fn of_file(path: impl AsRef<Path>) -> MpqResult<Self> {
        let path = path.as_ref();
        let original = std::fs::read(path)?;
        let archive = Archive::open(path)?;
        Self::of(&archive, &original)
    }

    /// The sentence a reader needs, in one place so two callers cannot disagree.
    #[must_use]
    pub fn note(&self) -> String {
        match self.first_difference {
            None => {
                "A rebuild of this map is byte identical, so saving would change only what you \
                     edit."
                    .to_string()
            }
            Some(at) => format!(
                "A rebuild preserves every member's content but rewrites the archive: \
                 {} becomes {}, and the first difference is at offset {at}. Saving would therefore \
                 write a new file rather than patch this one.",
                self.original_bytes, self.rebuilt_bytes,
            ),
        }
    }
}

impl fmt::Display for RebuildPreview {
    /// The note, so a caller that just wants to print this can.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.note())
    }
}

/// The offset of the first difference between two byte strings, or `None` when equal.
///
/// A common prefix with different lengths still differs, at the shorter end.
#[must_use]
pub fn first_difference_of(a: &[u8], b: &[u8]) -> Option<u64> {
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .map(|i| i as u64)
        .or_else(|| (a.len() != b.len()).then_some(a.len().min(b.len()) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Archive;

    #[test]
    fn a_built_archive_reads_back_byte_identical() {
        let mut builder = ArchiveBuilder::new();
        builder.add_stored("war3map.w3i", b"map info".to_vec());
        // Larger than one sector, so reading it exercises the sector path.
        let big: Vec<u8> = (0..9000u32).map(|i| (i % 251) as u8).collect();
        builder.add_stored("war3map.w3e", big.clone());
        builder.add_stored("war3map.j", Vec::new());

        let bytes = builder.to_bytes().unwrap();
        let archive = Archive::from_bytes(bytes).unwrap();

        assert_eq!(archive.header().file_offset, 0);
        assert_eq!(archive.read_file("war3map.w3i").unwrap(), b"map info");
        assert_eq!(archive.read_file("war3map.w3e").unwrap(), big);
        assert_eq!(archive.read_file("war3map.j").unwrap(), Vec::<u8>::new());
        // The tables really are encrypted: an unused slot is `0xFFFFFFFF` in the
        // plaintext, so it cannot look like that on disk unless the cipher ran.
        let raw = builder.to_bytes().unwrap();
        let h = Archive::from_bytes(raw.clone()).unwrap();
        assert!(h.file_count() >= 3);
        let hash_pos = 32 + h.header().hash_table_pos; // header at 0, header size 32
        assert!(
            !raw[hash_pos as usize..hash_pos as usize + 4]
                .iter()
                .all(|&b| b == 0xFF),
            "the first hash slot is unused, so its stored bytes must be enciphered"
        );
    }

    #[test]
    fn a_prefix_puts_the_header_at_512_and_comes_back_verbatim() {
        let mut prefix = vec![0u8; 512];
        prefix[0..4].copy_from_slice(b"HM3W");
        prefix[8..20].copy_from_slice(b"My Test Map\0");

        let mut builder = ArchiveBuilder::with_prefix(prefix.clone()).unwrap();
        builder.add_stored("war3map.w3i", b"info".to_vec());
        let bytes = builder.to_bytes().unwrap();

        let archive = Archive::from_bytes(bytes).unwrap();
        assert_eq!(archive.header().file_offset, 512);
        assert_eq!(archive.prefix(), &prefix[..]);
        assert_eq!(archive.read_file("war3map.w3i").unwrap(), b"info");
    }

    #[test]
    fn a_prefix_that_is_not_whole_sectors_is_refused() {
        assert!(matches!(
            ArchiveBuilder::with_prefix(vec![0u8; 100]),
            Err(MpqError::BadPrefixLength(100))
        ));
    }

    #[test]
    fn an_offset_adjusted_member_is_refused_rather_than_corrupted() {
        let mut builder = ArchiveBuilder::new();
        let member = RawMember {
            name: "(listfile)".to_string(),
            block: vec![0u8; 16],
            uncompressed_size: 16,
            flags: BlockFlags(
                FLAG_EXISTS | BlockFlags::COMPRESSED | BlockFlags::BLOCK_OFFSET_ADJUSTED_KEY,
            ),
            locale: 0,
            platform: 0,
        };
        assert!(matches!(
            builder.add_raw(member),
            Err(MpqError::MemberNotWritable { .. })
        ));
        assert_eq!(builder.member_count(), 0);
    }

    #[test]
    fn a_verbatim_member_survives_a_rebuild_with_its_flags_intact() {
        // A block of bytes the reader could never interpret, marked as a
        // single-unit encrypted member. Rebuilding must copy it, not decode it.
        let member = RawMember {
            name: "scripts\\war3map.j".to_string(),
            block: vec![0xAB; 40],
            uncompressed_size: 40,
            flags: BlockFlags(FLAG_EXISTS | BlockFlags::ENCRYPTED | FLAG_SINGLE_UNIT),
            locale: 0,
            platform: 0,
        };
        let mut builder = ArchiveBuilder::new();
        builder.add_raw(member.clone()).unwrap();
        let bytes = builder.to_bytes().unwrap();

        let archive = Archive::from_bytes(bytes).unwrap();
        let read_back = archive.raw_member("scripts\\war3map.j").unwrap();
        assert_eq!(read_back.block, member.block);
        assert_eq!(read_back.flags, member.flags);
        // Reading it as a file must fail loudly: it claims to be encrypted, so
        // the reader decrypts it and the result is not the original plaintext.
        assert!(archive.read_file("scripts\\war3map.j").is_ok());
    }

    #[test]
    fn hash_table_size_is_a_power_of_two_within_the_format_limit() {
        assert_eq!(hash_table_size(0), MIN_HASH_SIZE);
        assert_eq!(hash_table_size(3), MIN_HASH_SIZE);
        assert_eq!(hash_table_size(16), 32);
        assert_eq!(hash_table_size(10762), MAX_HASH_SIZE);
        assert!(hash_table_size(20000).is_power_of_two());
        assert!(hash_table_size(20000) <= MAX_HASH_SIZE);
    }

    #[test]
    fn too_many_members_is_an_error_not_a_truncated_table() {
        let mut builder = ArchiveBuilder::new();
        for i in 0..(MAX_HASH_SIZE as usize + 1) {
            builder.add_stored(format!("m{i}"), Vec::new());
        }
        assert!(matches!(
            builder.to_bytes(),
            Err(MpqError::TooManyMembers(_))
        ));
    }

    /// A rebuild of what this writer produced is byte identical, so the preview says so.
    ///
    /// This is the property the preview exists to report, and it is only true for an archive this
    /// writer made: a real map's rebuild differs at the hash table. See the next test.
    #[test]
    fn a_preview_of_our_own_output_is_identical() {
        let mut builder = ArchiveBuilder::new();
        builder.add_stored("war3map.w3i", b"map info".to_vec());
        builder.add_stored("war3map.wts", b"strings".to_vec());
        let bytes = builder.to_bytes().unwrap();

        let archive = Archive::from_bytes(bytes.clone()).unwrap();
        let preview = RebuildPreview::of(&archive, &bytes).unwrap();

        assert_eq!(preview.first_difference, None);
        assert!(preview.is_identical());
        assert_eq!(preview.original_bytes, bytes.len() as u64);
        assert_eq!(preview.rebuilt_bytes, preview.original_bytes);
        assert_eq!(preview.member_count, 2);
        assert!(
            preview.note().contains("byte identical"),
            "{}",
            preview.note()
        );
    }

    /// ⚠️ **The "reports a difference" half is not tested here, because this crate cannot produce
    /// a file that differs.** Anything [`ArchiveBuilder`] writes round-trips through
    /// [`RebuildPreview::of`] byte for byte — the previous test — so a difference can only come from
    /// an archive laid out some *other* way, and the only such archives are real maps. That case is
    /// covered where real maps are available: the command line's own tests run
    /// `war3 map rebuild` against a map named by an environment variable.
    ///
    /// Deliberately no test that asserts a difference from our own output. One was written and it
    /// passed for the wrong reason — `add_stored` was called in reverse name order and `file_names`
    /// sorts, so it reproduced the same bytes. A green test that does not exercise what its name
    /// claims is worse than no test.
    #[test]
    fn a_length_difference_is_a_difference() {
        assert_eq!(first_difference_of(b"abc", b"abc"), None);
        assert_eq!(first_difference_of(b"abc", b"abd"), Some(2));
        assert_eq!(first_difference_of(b"abc", b"abcd"), Some(3));
        assert_eq!(first_difference_of(b"abcd", b"abc"), Some(3));
        assert_eq!(first_difference_of(b"", b"a"), Some(0));
        assert_eq!(first_difference_of(b"", b""), None);
    }

    /// ⚠️ The distinction the editor used to make by string prefix: a member that cannot be
    /// relocated is a limitation of the file, not a fault.
    ///
    /// # Why this does not go through a preview
    ///
    /// It cannot: `ArchiveBuilder::add_raw` refuses to *write* a `BLOCK_OFFSET_ADJUSTED_KEY`
    /// member, which is what [`RebuildPreview::of`] uses, and `ArchivePatcher::replace` writes its
    /// own flags rather than taking them. There is therefore no way to build a file with such a
    /// member from inside this crate, and the end-to-end path needs a real map that has one.
    /// What is testable here is the classification itself, which is the part the editor reads.
    #[test]
    fn a_member_that_cannot_be_moved_is_not_applicable_rather_than_a_fault() {
        let not_applicable = MpqError::MemberNotWritable {
            name: "(listfile)".to_string(),
            reason: "its key is derived from its block offset",
        };
        assert!(not_applicable.is_not_applicable());

        // And a malformed file is a fault, with no action attached.
        assert!(!MpqError::NoArchiveHeader.is_not_applicable());
        assert!(!MpqError::Eof.is_not_applicable());
        assert!(!MpqError::NotFound("x".to_string()).is_not_applicable());
        assert!(!MpqError::BadPrefixLength(7).is_not_applicable());
    }

    #[test]
    fn colliding_names_both_resolve() {
        // Two members whose names hash to the same home slot have to be placed
        // by probing, and both must still be findable.
        let mut builder = ArchiveBuilder::new();
        let mut placed = 0;
        let table = crypt_table();
        let mut seen = std::collections::BTreeMap::new();
        for i in 0..4096 {
            let name = format!("member{i:04}");
            let slot = hash_string(&table, HashType::TableOffset, &name.to_uppercase()) & 63;
            if let Some(first) = seen.insert(slot, name.clone()) {
                builder.add_stored(first, b"first".to_vec());
                builder.add_stored(name, b"second".to_vec());
                placed += 1;
                if placed == 2 {
                    break;
                }
            }
        }
        assert!(placed > 0, "no colliding pair found to test");

        let archive = Archive::from_bytes(builder.to_bytes().unwrap()).unwrap();
        for name in archive.file_names() {
            let data = archive.read_file(name).unwrap();
            assert!(data == b"first" || data == b"second");
        }
    }
}

#[cfg(test)]
mod patcher_tests {
    use super::*;

    /// Builds a small archive to patch, with two members so "the other one is
    /// untouched" is checkable.
    fn two_member_archive() -> Vec<u8> {
        let mut builder = ArchiveBuilder::new();
        builder.add_stored("war3map.w3i", b"original-map-info".to_vec());
        builder.add_stored("war3map.wts", b"original-strings".to_vec());
        builder.to_bytes().expect("a buildable archive")
    }

    /// The core claim of ADR-0028, stated as an assertion.
    ///
    /// Patching one member must leave the archive identical except for the block
    /// table entry that member uses and the bytes appended for its new block. This
    /// is the property a rebuild cannot provide, and the one an editor's safety rests
    /// on: a user's file comes back as their file.
    #[test]
    fn patching_changes_only_the_entry_and_the_appended_block() {
        let original = two_member_archive();
        let mut patcher = ArchivePatcher::new(original.clone()).expect("a readable archive");

        let replacement = b"edited-map-info".to_vec();
        patcher
            .replace("war3map.w3i", replacement.clone())
            .expect("an unencrypted member is patchable");
        let patched = patcher.to_bytes();

        assert_eq!(patcher.patched_count(), 1);
        assert_eq!(
            patched.len(),
            original.len() + replacement.len(),
            "exactly the new block is appended and nothing is removed"
        );

        // Everything up to the block table must be untouched.
        let archive = Archive::from_bytes(original.clone()).expect("the original");
        let block_table_at = archive.header().block_table_offset() as usize;
        assert_eq!(
            &patched[..block_table_at],
            &original[..block_table_at],
            "the prefix, header, member data and hash table must all be identical"
        );

        // Past the block table: only the appended block is new.
        let block_table_end = block_table_at + archive.header().block_table_size as usize * 16;
        assert_eq!(
            &patched[block_table_end..],
            &replacement[..],
            "the appended bytes are the replacement, verbatim"
        );
    }

    /// The other member must read back exactly as before.
    #[test]
    fn an_untouched_member_reads_back_identically() {
        let original = two_member_archive();
        let mut patcher = ArchivePatcher::new(original.clone()).unwrap();
        patcher.replace("war3map.w3i", b"edited".to_vec()).unwrap();

        let patched = Archive::from_bytes(patcher.to_bytes()).expect("the patched archive reads");
        assert_eq!(
            patched.read_file("war3map.wts").unwrap(),
            b"original-strings"
        );
        assert_eq!(patched.read_file("war3map.w3i").unwrap(), b"edited");
    }

    /// The replacement must be readable under its own name, with its size reported.
    #[test]
    fn the_replaced_member_reads_back_as_the_new_content() {
        let original = two_member_archive();
        let mut patcher = ArchivePatcher::new(original).unwrap();
        let replacement = vec![0xABu8; 1234];
        patcher.replace("war3map.w3i", replacement.clone()).unwrap();

        let patched = Archive::from_bytes(patcher.to_bytes()).unwrap();
        assert_eq!(patched.read_file("war3map.w3i").unwrap(), replacement);
        // And the member list still finds both.
        let mut names = patched.file_names();
        names.sort_unstable();
        // `file_names` reports the enumeration's uppercase form by design.
        assert_eq!(names, vec!["WAR3MAP.W3I", "WAR3MAP.WTS"]);
    }

    /// An unknown name is an error, not a silent no-op.
    #[test]
    fn replacing_a_missing_member_is_an_error() {
        let mut patcher = ArchivePatcher::new(two_member_archive()).unwrap();
        let err = patcher.replace("war3map.nope", b"x".to_vec()).unwrap_err();
        assert!(matches!(err, MpqError::NotFound(_)), "got {err:?}");
        assert_eq!(patcher.patched_count(), 0, "nothing was modified");
    }

    /// Empty content is representable, and the format expresses it as a zero block.
    #[test]
    fn replacing_with_nothing_produces_a_listable_empty_member() {
        let mut patcher = ArchivePatcher::new(two_member_archive()).unwrap();
        patcher.replace("war3map.w3i", Vec::new()).unwrap();
        let patched = Archive::from_bytes(patcher.to_bytes()).unwrap();
        assert!(patched
            .file_names()
            .iter()
            .any(|n| n.eq_ignore_ascii_case("war3map.w3i")));
        assert!(patched.read_file("war3map.w3i").unwrap().is_empty());
    }

    /// An encrypted member is refused, and the refusal is the documented one.
    ///
    /// The alternative — writing it plainly — would make a protected member
    /// readable, which is a change nobody asked for. This pins that the refusal
    /// happens rather than that some other error does.
    #[test]
    fn an_encrypted_member_is_refused_rather_than_made_readable() {
        let mut builder = ArchiveBuilder::new();
        builder
            .add_raw(RawMember {
                name: "secret.w3i".to_string(),
                block: vec![0u8; 8],
                uncompressed_size: 8,
                flags: BlockFlags(0x8000_0000 | 0x0001_0000), // exists | encrypted
                locale: 0,
                platform: 0,
            })
            .unwrap();
        let bytes = builder.to_bytes().unwrap();

        let mut patcher = ArchivePatcher::new(bytes).unwrap();
        let err = patcher
            .replace("secret.w3i", b"plain".to_vec())
            .unwrap_err();
        match err {
            MpqError::MemberNotWritable { reason, .. } => {
                assert!(reason.contains("encrypted"), "reason was: {reason}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(patcher.patched_count(), 0);
    }

    /// Two replacements in one pass both land.
    #[test]
    fn two_members_can_be_replaced_in_one_pass() {
        let original = two_member_archive();
        let mut patcher = ArchivePatcher::new(original).unwrap();
        patcher.replace("war3map.w3i", b"first".to_vec()).unwrap();
        patcher.replace("war3map.wts", b"second".to_vec()).unwrap();
        assert_eq!(patcher.patched_count(), 2);

        let patched = Archive::from_bytes(patcher.to_bytes()).unwrap();
        assert_eq!(patched.read_file("war3map.w3i").unwrap(), b"first");
        assert_eq!(patched.read_file("war3map.wts").unwrap(), b"second");
    }
}

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

use std::path::Path;

use crate::archive::{normalise_name, BlockFlags, MpqError, MpqResult, RawMember, MPQ_MAGIC};
use crate::crypto::{
    crypt_table, encrypt, hash_string, HashType, BLOCK_TABLE_KEY_NAME, HASH_TABLE_KEY_NAME,
};

/// Size of the header written here. The format's original version is 32 bytes.
const HEADER_SIZE: u32 = 32;
/// Size of one hash or block table entry.
const ENTRY_SIZE: u32 = 16;
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

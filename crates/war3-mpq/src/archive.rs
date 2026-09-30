//! MPQ archive reading.
//!
//! Three properties of the container are easy to get wrong and all of them fail
//! silently:
//!
//! 1. **The header is not necessarily at offset 0.** Map files carry an `HM3W`
//!    prefix and the archive starts on the next 0x200 boundary, usually at 512.
//!    The magic has to be searched for in 0x200 steps.
//! 2. **Offsets inside the header are relative to the header**, while a block
//!    entry's `file_pos` is relative to the start of the file. Two coordinate
//!    systems in one structure.
//! 3. **Table positions must be read from the header**, never assumed: the
//!    tables may sit before, after or inside the data region.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use war3_core::diag::{DiagnosticCode, Diagnostics};
use war3_core::ParseError;

use crate::codec::{self, CodecError};
use crate::crypto::{bytes_to_u32_le, crypt_table, decrypt, hash_string, HashType};

/// MPQ magic bytes.
pub const MPQ_MAGIC: [u8; 4] = *b"MPQ\x1a";
/// Length of the magic.
pub const MPQ_MAGIC_LEN: usize = 4;
/// Step used when searching for the header.
pub const MPQ_SEARCH_STEP: usize = 0x200;
/// Size of one hash table entry.
const HASH_ENTRY_SIZE: usize = 16;
/// Size of one block table entry.
const BLOCK_ENTRY_SIZE: usize = 16;
/// Block index value marking an unused hash slot.
const HASH_ENTRY_EMPTY: u32 = 0xFFFF_FFFF;

/// Errors returned while reading an archive.
#[derive(Debug)]
pub enum MpqError {
    /// The file ended while a table was being read.
    Eof,
    /// No `MPQ\x1a` was found anywhere in the file.
    NoArchiveHeader,
    /// A table lies outside the file.
    TablesOutOfRange {
        /// Which table.
        table: &'static str,
        /// Offset recorded in the header, relative to the header.
        offset: u32,
        /// Absolute offset of the header.
        header_offset: u64,
        /// Total file length.
        file_len: usize,
    },
    /// The hash table size is not a power of two.
    ///
    /// The format uses `size - 1` as a mask, so any other size computes wrong
    /// slot indices.
    HashTableSizeNotPowerOfTwo(u32),
    /// `(listfile)` was not valid UTF-8.
    BadListfileEncoding,
    /// A sector offset table is inconsistent with the block it describes.
    SectorTableOutOfRange,
    /// The decompressed length disagrees with what the block table declared.
    ///
    /// Reported rather than tolerated: a short result means the wrong branch was
    /// taken or the data is damaged, and it would surface much later.
    SizeMismatch {
        /// Length declared in the block table.
        expected: usize,
        /// Length actually produced.
        got: usize,
    },
    /// A member file does not exist.
    NotFound(String),
    /// Decompression failed.
    Codec(CodecError),
    /// A structural parse failure.
    Parse(ParseError),
    /// A filesystem failure.
    Io(std::io::Error),
}

impl fmt::Display for MpqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof => f.write_str("file ended while reading a table"),
            Self::NoArchiveHeader => {
                f.write_str("no MPQ header found (searched the whole file in 0x200 steps)")
            }
            Self::TablesOutOfRange { table, offset, header_offset, file_len } => write!(
                f,
                "{table} is outside the file: header at {header_offset}, table at +{offset}, file length {file_len}"
            ),
            Self::HashTableSizeNotPowerOfTwo(n) => {
                write!(f, "hash table size {n} is not a power of two")
            }
            Self::BadListfileEncoding => f.write_str("(listfile) is not valid UTF-8"),
            Self::SectorTableOutOfRange => f.write_str("sector offset table is out of range"),
            Self::SizeMismatch { expected, got } => write!(
                f,
                "decompressed length mismatch: block table declared {expected}, got {got}"
            ),
            Self::NotFound(name) => write!(f, "archive has no {name}"),
            Self::Codec(e) => write!(f, "decompression failed: {e}"),
            Self::Parse(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MpqError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            Self::Parse(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ParseError> for MpqError {
    fn from(e: ParseError) -> Self {
        Self::Parse(e)
    }
}

impl From<CodecError> for MpqError {
    fn from(e: CodecError) -> Self {
        Self::Codec(e)
    }
}

impl From<std::io::Error> for MpqError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<MpqError> for war3_core::Error {
    fn from(e: MpqError) -> Self {
        Self::new(e)
    }
}

/// Result alias for this crate.
pub type MpqResult<T> = Result<T, MpqError>;

// ---------------------------------------------------------------------------
// Header and tables
// ---------------------------------------------------------------------------

/// The MPQ header, as shared by format versions 0 and 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveHeader {
    /// Absolute offset of the magic in the file. Rarely zero.
    pub file_offset: u64,
    /// Size of the header itself, usually 32.
    pub header_size: u32,
    /// Total archive size, including any prefix.
    pub archive_size: u32,
    /// Container format version. 0 is the classic layout.
    pub format_version: u16,
    /// Sector size expressed as a shift: `sector_size = 512 << shift`.
    ///
    /// The stored value is small (3 in practice) and the base is 512, not 1.
    pub sector_size_shift: u16,
    /// Hash table offset, relative to the header.
    pub hash_table_pos: u32,
    /// Block table offset, relative to the header.
    pub block_table_pos: u32,
    /// Number of hash table entries; must be a power of two.
    pub hash_table_size: u32,
    /// Number of block table entries.
    pub block_table_size: u32,
}

impl ArchiveHeader {
    /// Sector size in bytes.
    ///
    /// The base is 512: a stored shift of 3 means 4096-byte sectors, which is
    /// what real archives use and what their block sizes imply.
    #[must_use]
    pub const fn sector_size(&self) -> u32 {
        512u32 << (self.sector_size_shift & 0x0F)
    }

    /// Absolute offset of the hash table.
    #[must_use]
    pub const fn hash_table_offset(&self) -> u64 {
        self.file_offset + self.hash_table_pos as u64
    }

    /// Absolute offset of the block table.
    #[must_use]
    pub const fn block_table_offset(&self) -> u64 {
        self.file_offset + self.block_table_pos as u64
    }
}

/// One decrypted hash table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HashEntry {
    /// `NameA` hash of the file's name.
    pub name_a: u32,
    /// `NameB` hash, used to derive the decryption key.
    pub name_b: u32,
    /// Locale, in the low 16 bits.
    pub locale: u16,
    /// Platform, in the high 16 bits.
    pub platform: u16,
    /// Index into the block table, or `0xFFFFFFFF` for an unused slot.
    pub block_index: u32,
}

impl HashEntry {
    /// Whether this slot is unused.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.block_index == HASH_ENTRY_EMPTY
    }
}

/// A block entry's flag word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockFlags(pub u32);

impl BlockFlags {
    /// The member is compressed. Carries no algorithm information.
    pub const COMPRESSED: u32 = 0x0000_0200;
    /// The compressed size is stored inside the block.
    pub const SIZE_IN_BLOCK: u32 = 0x0000_0100;
    /// The member is encrypted.
    pub const ENCRYPTED: u32 = 0x0001_0000;
    /// Encrypted with a fixed key rather than one derived from the file name.
    pub const FIX_KEY: u32 = 0x0002_0000;
    /// The member is one block, with no sector offset table.
    pub const SINGLE_UNIT: u32 = 0x0100_0000;
    /// The member has a sector offset table and independently compressed sectors.
    pub const MULTI_BLOCK: u32 = 0x0400_0000;
    /// A checksum entry exists.
    pub const EXISTS: u32 = 0x8000_0000;

    /// Whether a bit is set.
    #[must_use]
    pub const fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }

    /// Whether the member is compressed.
    #[must_use]
    pub const fn is_compressed(self) -> bool {
        self.has(Self::COMPRESSED)
    }

    /// Whether the member is encrypted.
    #[must_use]
    pub const fn is_encrypted(self) -> bool {
        self.has(Self::ENCRYPTED)
    }
}

impl fmt::Display for BlockFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.is_compressed() {
            parts.push("compressed");
        }
        if self.is_encrypted() {
            parts.push("encrypted");
        }
        if self.has(Self::FIX_KEY) {
            parts.push("fix-key");
        }
        if self.has(Self::SINGLE_UNIT) {
            parts.push("single-unit");
        }
        if self.has(Self::MULTI_BLOCK) {
            parts.push("multi-block");
        }
        if self.has(Self::SIZE_IN_BLOCK) {
            parts.push("size-in-block");
        }
        if self.has(Self::EXISTS) {
            parts.push("exists");
        }
        if parts.is_empty() {
            f.write_str("none")
        } else {
            f.write_str(&parts.join("|"))
        }
    }
}

/// One decrypted block table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockEntry {
    /// Member offset, **relative to the start of the file**.
    pub file_pos: u32,
    /// Stored size on disk.
    pub compressed_size: u32,
    /// Size after decompression.
    pub uncompressed_size: u32,
    /// Flag word.
    pub flags: BlockFlags,
}

impl BlockEntry {
    /// The compression algorithm bits declared in the low byte.
    ///
    /// Display only. The presence of compression is decided by
    /// [`BlockFlags::is_compressed`]; a real zlib member has flags like
    /// `0x04000200` whose low byte is zero, so this often returns 0 even though
    /// the member is compressed. Decompression should be handed the full flags.
    #[must_use]
    pub const fn compression_method_bits(&self) -> u32 {
        self.flags.0 & crate::codec::COMPRESSION_METHOD_MASK
    }

    /// A human-readable name for the compression, for CLI output.
    ///
    /// When the low byte names nothing — which is the normal case — this says so
    /// rather than guessing.
    #[must_use]
    pub const fn compression_name(&self) -> &'static str {
        use crate::codec as c;
        if !self.flags.is_compressed() {
            return "none";
        }
        let m = self.compression_method_bits();
        if m & c::COMPRESSION_ZLIB != 0 {
            "zlib"
        } else if m & c::COMPRESSION_BZIP2 != 0 {
            "bzip2"
        } else if m & c::COMPRESSION_PKWARE != 0 {
            "pkware"
        } else {
            "compressed(algorithm not declared)"
        }
    }
}

/// A summary of an archive's structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    /// The header.
    pub header: ArchiveHeader,
    /// How many block table entries are in use.
    pub used_blocks: usize,
}

// ---------------------------------------------------------------------------
// Archive
// ---------------------------------------------------------------------------

/// An opened MPQ archive.
///
/// The whole file is held in memory. Maps are small enough for that to be
/// simpler than streaming, and it avoids needing synchronous I/O on the WASM
/// side.
#[derive(Debug)]
pub struct Archive {
    /// Path, for diagnostics. `None` when opened from memory.
    path: Option<PathBuf>,
    /// The **entire** file, from offset 0, including any `HM3W` prefix.
    ///
    /// Keeping the whole file rather than a slice starting at the MPQ header is
    /// deliberate: the header's table offsets are relative to the header while a
    /// block entry's `file_pos` is relative to the file, so both coordinate
    /// systems are in play. Slicing off the prefix would mean remembering to
    /// subtract `file_offset` at every block read, and forgetting once reads
    /// garbage without any error.
    buffer: Vec<u8>,
    header: ArchiveHeader,
    hash_table: Vec<HashEntry>,
    block_table: Vec<BlockEntry>,
    /// Normalised name to block index. Only enumerated files have names: the
    /// format does not store them, so anything not named by `(listfile)`, the
    /// known-names list or `war3map.imp` stays anonymous.
    by_name: BTreeMap<String, u32>,
    /// Diagnostics collected while opening.
    diagnostics: Diagnostics,
}

impl Archive {
    /// Opens an archive from a file.
    pub fn open(path: impl AsRef<Path>) -> MpqResult<Self> {
        let path = path.as_ref().to_path_buf();
        let bytes = std::fs::read(&path)?;
        let mut archive = Self::from_bytes(bytes)?;
        archive.path = Some(path);
        Ok(archive)
    }

    /// Opens an archive from memory, taking ownership.
    pub fn from_bytes(bytes: Vec<u8>) -> MpqResult<Self> {
        let mut diagnostics = Diagnostics::new();

        let file_offset = find_header(&bytes).ok_or(MpqError::NoArchiveHeader)?;
        if file_offset != 0 {
            diagnostics.push(war3_core::Diagnostic::info(
                DiagnosticCode::MpqHeaderOffset,
                format!(
                    "MPQ header is not at offset 0 but at {file_offset} ({file_offset:#X}); \
                     the prefix is an HM3W map header"
                ),
            ));
        }

        let header = read_header(&bytes, file_offset)?;
        if !header.hash_table_size.is_power_of_two() {
            return Err(MpqError::HashTableSizeNotPowerOfTwo(header.hash_table_size));
        }

        let table = crypt_table();
        let hash_table = read_hash_table(&bytes, &header, &table)?;
        let block_table = read_block_table(&bytes, &header, &table)?;

        let mut archive = Self {
            path: None,
            buffer: bytes,
            header,
            hash_table,
            block_table,
            by_name: BTreeMap::new(),
            diagnostics,
        };
        archive.build_name_index();
        Ok(archive)
    }

    /// A summary of the archive.
    #[must_use]
    pub fn info(&self) -> ArchiveInfo {
        ArchiveInfo {
            header: self.header,
            used_blocks: self
                .block_table
                .iter()
                .filter(|b| b.uncompressed_size > 0)
                .count(),
        }
    }

    /// The header.
    #[must_use]
    pub const fn header(&self) -> &ArchiveHeader {
        &self.header
    }

    /// Diagnostics collected while opening.
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// The file path, or `None` when opened from memory.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The member names that were enumerated, uppercase and sorted.
    ///
    /// The format does not store names, so this list comes from the enumeration
    /// ladder and may be incomplete.
    #[must_use]
    pub fn file_names(&self) -> Vec<&str> {
        self.by_name.keys().map(String::as_str).collect()
    }

    /// How many members have names.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.by_name.len()
    }

    /// Whether a member with this name exists, case-insensitively.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.find_block_index_by_name(name).is_some()
    }

    /// The block index for a name, used by the CLI to show compression flags.
    #[must_use]
    pub fn block_index_for_name(&self, name: &str) -> Option<u32> {
        self.find_block_index_by_name(name)
    }

    /// How many block table entries are in use.
    ///
    /// Different from [`Archive::file_count`]: this counts members that exist,
    /// the other counts names that were worked out.
    #[must_use]
    pub fn used_block_count(&self) -> usize {
        self.block_table
            .iter()
            .filter(|b| b.uncompressed_size > 0)
            .count()
    }

    /// A block table entry by index.
    #[must_use]
    pub fn block_entry(&self, index: usize) -> Option<&BlockEntry> {
        self.block_table.get(index)
    }

    /// Reads and decompresses a member.
    ///
    /// The name is matched case-insensitively and `/` is equivalent to `\`.
    pub fn read_file(&self, name: &str) -> MpqResult<Vec<u8>> {
        let index = self
            .find_block_index_by_name(name)
            .ok_or_else(|| MpqError::NotFound(name.to_string()))?;
        self.read_block(index, name)
    }

    /// Reads a member by block index.
    pub fn read_block(&self, block_index: u32, name: &str) -> MpqResult<Vec<u8>> {
        let entry = *self
            .block_table
            .get(block_index as usize)
            .ok_or_else(|| MpqError::NotFound(name.to_string()))?;
        self.read_entry(&entry, name)
    }

    /// Parses `(listfile)`.
    fn read_listfile_names(&self) -> MpqResult<Vec<String>> {
        let raw = self.read_file("(listfile)")?;
        let text = String::from_utf8(raw).map_err(|_| MpqError::BadListfileEncoding)?;
        Ok(text
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Builds the name index using the enumeration ladder.
    ///
    /// `(listfile)`, then the known-names list, then `war3map.imp`. Deduplicated
    /// case-insensitively, in that order of precedence.
    fn build_name_index(&mut self) {
        let mut diagnostics = std::mem::take(&mut self.diagnostics);

        // Step 1: (listfile)
        match self.read_listfile_names() {
            Ok(names) => {
                for name in names {
                    self.insert_name(&name);
                }
            }
            Err(_) => {
                diagnostics.push(war3_core::Diagnostic::error(
                    DiagnosticCode::MpqNoListfile,
                    "archive has no usable (listfile); enumeration falls back to the known-names \
                     list and war3map.imp (maps saved with the World Editor's listfile suppressed \
                     omit it deliberately)",
                ));
            }
        }

        // Step 2: names that are always worth probing for.
        for name in KNOWN_MEMBER_NAMES {
            if self.lookup_raw(name).is_some() {
                self.insert_name(name);
            }
        }

        // Step 3: war3map.imp
        if let Ok(raw) = self.read_file("war3map.imp") {
            if let Ok(imports) = parse_imports(&raw) {
                for entry in imports {
                    let mut candidate = entry.path.clone();
                    if self.lookup_raw(&candidate).is_none() {
                        // A path listed in `.imp` that is not present is retried
                        // with a `war3mapImported\` prefix.
                        candidate = format!("war3mapImported\\{}", entry.path);
                    }
                    if self.lookup_raw(&candidate).is_some() {
                        self.insert_name(&candidate);
                    }
                }
            }
        }

        self.diagnostics = diagnostics;
    }

    /// Inserts a name under its normalised key.
    fn insert_name(&mut self, name: &str) {
        let key = normalise_name(name);
        if key.is_empty() || self.by_name.contains_key(&key) {
            return;
        }
        if let Some(index) = self.lookup_raw(name) {
            self.by_name.insert(key, index);
        }
    }

    /// Looks up a block index by probing the hash table.
    fn lookup_raw(&self, name: &str) -> Option<u32> {
        let table = crypt_table();
        let upper = normalise_name(name);
        let mask = self.header.hash_table_size - 1;

        let start = hash_string(&table, HashType::TableOffset, &upper) & mask;
        let want_a = hash_string(&table, HashType::NameA, &upper);

        for probe in 0..self.header.hash_table_size {
            let slot = (start + probe) & mask;
            let entry = &self.hash_table[slot as usize];
            if entry.is_empty() {
                // An unused slot means the name is absent, so probing can stop.
                return None;
            }
            if entry.name_a == want_a && (entry.block_index as usize) < self.block_table.len() {
                return Some(entry.block_index);
            }
        }
        None
    }

    /// Looks up by normalised key first, then by hashing.
    fn find_block_index_by_name(&self, name: &str) -> Option<u32> {
        let key = normalise_name(name);
        if let Some(index) = self.by_name.get(&key) {
            return Some(*index);
        }
        self.lookup_raw(name)
    }

    /// Reads one block.
    fn read_entry(&self, entry: &BlockEntry, name: &str) -> MpqResult<Vec<u8>> {
        let start = entry.file_pos as usize;
        let size = entry.compressed_size as usize;
        let end = start.checked_add(size).ok_or(MpqError::Eof)?;
        let uncompressed = entry.uncompressed_size as usize;

        if uncompressed == 0 {
            return Ok(Vec::new());
        }
        if end > self.buffer.len() {
            return Err(MpqError::Eof);
        }

        // Single-block member: no sector offset table, decompressed in one go.
        if entry.flags.has(BlockFlags::SINGLE_UNIT) {
            let mut data = self.buffer[start..end].to_vec();
            if entry.flags.is_encrypted() {
                let key = self.file_key(entry, name);
                decrypt_in_place(&mut data, key);
            }
            if entry.flags.is_compressed() {
                // Decompression failure must be an error. Falling back to the raw
                // bytes would hand the caller data that looks plausible but is
                // completely misplaced.
                let out = codec::decompress(&data, entry.flags.0, uncompressed)?;
                return Ok(out);
            }
            data.truncate(uncompressed.min(data.len()));
            return Ok(data);
        }

        // Multi-block member: read the sector offset table first. It holds one
        // more entry than there are sectors, the last being the end of the data.
        let sector_size = self.header.sector_size() as usize;
        let sector_count = uncompressed.div_ceil(sector_size);
        let table_bytes = (sector_count + 1) * 4;
        if size < table_bytes {
            return Err(MpqError::SectorTableOutOfRange);
        }
        let mut offsets = bytes_to_u32_le(&self.buffer[start..start + table_bytes]);
        if entry.flags.is_encrypted() {
            // The sector offset table is keyed with `file_key - 1`, not `file_key`.
            let key = self.file_key(entry, name);
            decrypt(&crypt_table(), &mut offsets, key.wrapping_sub(1));
        }

        let mut out = Vec::with_capacity(uncompressed);
        for i in 0..sector_count {
            let sector_start = start + offsets[i] as usize;
            let sector_end = start + offsets[i + 1] as usize;
            if sector_end < sector_start || sector_end > end {
                return Err(MpqError::SectorTableOutOfRange);
            }
            if sector_start == sector_end {
                // An empty sector means the whole span is zeroes.
                out.resize(out.len() + sector_size, 0);
                continue;
            }
            let mut data = self.buffer[sector_start..sector_end].to_vec();
            if entry.flags.is_encrypted() {
                let key = self.file_key(entry, name).wrapping_add(i as u32);
                decrypt_in_place(&mut data, key);
            }
            // After decryption the first byte says whether this sector is
            // compressed: `0xFF` means yes.
            let compressed_flag = data.first().copied().unwrap_or(0);
            if entry.flags.is_compressed() && compressed_flag == 0xFF {
                let payload = data.get(1..).ok_or(MpqError::SectorTableOutOfRange)?;
                // The last sector is usually shorter than a full sector, so its
                // expected length comes from the member's total size. Checking
                // every sector against the full sector size would reject almost
                // every real file.
                let sector_expected = if i + 1 == sector_count {
                    uncompressed.saturating_sub(i * sector_size)
                } else {
                    sector_size
                };
                let sector = codec::decompress(payload, entry.flags.0, sector_expected)?;
                out.extend_from_slice(&sector);
            } else {
                out.extend_from_slice(&data);
            }
        }
        if out.len() != uncompressed {
            return Err(MpqError::SizeMismatch {
                expected: uncompressed,
                got: out.len(),
            });
        }
        out.truncate(uncompressed);
        Ok(out)
    }

    /// Derives the key used to decrypt a member's contents.
    ///
    /// Ordinary members use the `FileKey` hash of the file name with the
    /// directory part removed. Members with the fixed-key flag use their offset
    /// in the archive instead.
    fn file_key(&self, entry: &BlockEntry, name: &str) -> u32 {
        if entry.flags.has(BlockFlags::FIX_KEY) {
            return entry.file_pos;
        }
        let table = crypt_table();
        let upper = normalise_name(name);
        let base = upper.rsplit('\\').next().unwrap_or(&upper);
        hash_string(&table, HashType::FileKey, base)
    }
}

/// Searches for the MPQ header.
///
/// Steps through the file in 0x200 increments, then falls back to a byte-wise
/// scan for archives whose alignment is not a multiple of 0x200.
#[must_use]
pub fn find_header(bytes: &[u8]) -> Option<u64> {
    let mut offset = 0usize;
    while offset + MPQ_MAGIC_LEN <= bytes.len() {
        if bytes[offset..offset + MPQ_MAGIC_LEN] == MPQ_MAGIC {
            return Some(offset as u64);
        }
        offset += MPQ_SEARCH_STEP;
    }
    bytes
        .windows(MPQ_MAGIC_LEN)
        .position(|w| w == MPQ_MAGIC)
        .map(|p| p as u64)
}

fn read_header(bytes: &[u8], file_offset: u64) -> MpqResult<ArchiveHeader> {
    let base = usize::try_from(file_offset).map_err(|_| MpqError::Eof)?;
    let raw = read_at(bytes, base, 32)?;
    if raw[0..4] != MPQ_MAGIC {
        return Err(MpqError::NoArchiveHeader);
    }
    Ok(ArchiveHeader {
        file_offset,
        header_size: u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
        archive_size: u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]),
        format_version: u16::from_le_bytes([raw[12], raw[13]]),
        sector_size_shift: u16::from_le_bytes([raw[14], raw[15]]),
        hash_table_pos: u32::from_le_bytes([raw[16], raw[17], raw[18], raw[19]]),
        block_table_pos: u32::from_le_bytes([raw[20], raw[21], raw[22], raw[23]]),
        hash_table_size: u32::from_le_bytes([raw[24], raw[25], raw[26], raw[27]]),
        block_table_size: u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]),
    })
}

fn read_hash_table(
    buffer: &[u8],
    header: &ArchiveHeader,
    table: &[u32; 0x500],
) -> MpqResult<Vec<HashEntry>> {
    // Table positions are relative to the header, so add `file_offset`.
    let offset = usize::try_from(header.hash_table_offset()).map_err(|_| MpqError::Eof)?;
    let size = header.hash_table_size as usize * HASH_ENTRY_SIZE;
    let raw = read_at_checked(buffer, offset, size).ok_or(MpqError::TablesOutOfRange {
        table: "hash table",
        offset: header.hash_table_pos,
        header_offset: header.file_offset,
        file_len: buffer.len(),
    })?;
    let mut words = bytes_to_u32_le(raw);
    let key = u32::from_le_bytes(*b"HASH");
    decrypt(table, &mut words, key);

    Ok(words
        .chunks_exact(4)
        .map(|c| HashEntry {
            name_a: c[0],
            name_b: c[1],
            locale: (c[2] & 0xFFFF) as u16,
            platform: (c[2] >> 16) as u16,
            block_index: c[3],
        })
        .collect())
}

fn read_block_table(
    buffer: &[u8],
    header: &ArchiveHeader,
    table: &[u32; 0x500],
) -> MpqResult<Vec<BlockEntry>> {
    // Same as above: relative to the header.
    let offset = usize::try_from(header.block_table_offset()).map_err(|_| MpqError::Eof)?;
    // Only the first `block_table_size` entries are decrypted. The table region
    // is usually reserved at the hash table's size, so decrypting that many
    // entries would read into unrelated data.
    let size = header.block_table_size as usize * BLOCK_ENTRY_SIZE;
    let raw = read_at_checked(buffer, offset, size).ok_or(MpqError::TablesOutOfRange {
        table: "block table",
        offset: header.block_table_pos,
        header_offset: header.file_offset,
        file_len: buffer.len(),
    })?;
    let mut words = bytes_to_u32_le(raw);
    let key = u32::from_le_bytes(*b"BLK#");
    decrypt(table, &mut words, key);

    Ok(words
        .chunks_exact(4)
        .map(|c| BlockEntry {
            file_pos: c[0],
            compressed_size: c[1],
            uncompressed_size: c[2],
            flags: BlockFlags(c[3]),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Uppercases a name and maps `/` to `\`, keeping separators.
///
/// This is the form used both for hashing and for deriving decryption keys.
#[must_use]
pub fn normalise_name(name: &str) -> String {
    name.chars()
        .map(|c| if c == '/' { '\\' } else { c })
        .collect::<String>()
        .to_uppercase()
}

fn decrypt_in_place(data: &mut [u8], key: u32) {
    // A trailing partial word is left alone: contents are encrypted as a stream
    // of words and the remainder is stored as is.
    let usable = data.len() & !3;
    if usable == 0 {
        return;
    }
    let mut words = bytes_to_u32_le(&data[..usable]);
    decrypt(&crypt_table(), &mut words, key);
    for (chunk, word) in data[..usable].chunks_exact_mut(4).zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
}

fn read_at(bytes: &[u8], offset: usize, len: usize) -> MpqResult<&[u8]> {
    read_at_checked(bytes, offset, len).ok_or(MpqError::Eof)
}

fn read_at_checked(bytes: &[u8], offset: usize, len: usize) -> Option<&[u8]> {
    let end = offset.checked_add(len)?;
    bytes.get(offset..end)
}

/// One entry of `war3map.imp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEntry {
    /// Path inside the archive.
    pub path: String,
    /// Import flags, preserved verbatim.
    pub flags: u32,
}

/// Parses `war3map.imp`.
///
/// Layout: `int32 version`, `int32 count`, then `count` records of
/// `int32 flags` followed by a NUL-terminated path.
pub fn parse_imports(bytes: &[u8]) -> MpqResult<Vec<ImportEntry>> {
    let mut cursor = 0usize;
    let _version = take_u32(bytes, &mut cursor)?;
    let count = take_u32(bytes, &mut cursor)?;
    let mut out = Vec::with_capacity(count.min(1 << 20) as usize);
    for _ in 0..count {
        let flags = take_u32(bytes, &mut cursor)?;
        let path = take_cstr(bytes, &mut cursor)?;
        out.push(ImportEntry { path, flags });
    }
    Ok(out)
}

fn take_u32(bytes: &[u8], cursor: &mut usize) -> MpqResult<u32> {
    let raw = read_at_checked(bytes, *cursor, 4).ok_or(MpqError::Eof)?;
    *cursor += 4;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn take_cstr(bytes: &[u8], cursor: &mut usize) -> MpqResult<String> {
    let rest = bytes.get(*cursor..).ok_or(MpqError::Eof)?;
    let end = rest.iter().position(|&b| b == 0).ok_or(MpqError::Eof)?;
    let s = String::from_utf8(rest[..end].to_vec()).map_err(|_| MpqError::BadListfileEncoding)?;
    *cursor += end + 1;
    Ok(s)
}

/// Names worth probing for when an archive does not list its own contents.
///
/// The World Editor can save a map with its `(listfile)` suppressed, in which
/// case these and `war3map.imp` are the only ways to find anything.
pub const KNOWN_MEMBER_NAMES: &[&str] = &[
    // Metadata
    "war3map.w3i",
    "war3map.wts",
    "war3map.imp",
    "war3mapMisc.txt",
    "war3mapSkin.txt",
    // Terrain
    "war3map.w3e",
    "war3map.wpm",
    "war3map.shd",
    "war3map.mmp",
    // Objects
    "war3map.w3u",
    "war3map.w3t",
    "war3map.w3a",
    "war3map.w3b",
    "war3map.w3d",
    "war3map.w3h",
    "war3map.w3q",
    // Placement
    "war3map.doo",
    "war3mapUnits.doo",
    // Triggers
    "war3map.wtg",
    "war3map.wct",
    // Regions, cameras, sounds
    "war3map.w3r",
    "war3map.w3c",
    "war3map.w3s",
    // Scripts
    "war3map.j",
    "war3map.lua",
    "scripts\\war3map.j",
    "scripts\\war3map.lua",
    // Archive metadata
    "(listfile)",
    "(attributes)",
    "(signature)",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_header_scans_by_512_step() {
        let mut bytes = vec![0u8; 1536];
        bytes[1024..1028].copy_from_slice(&MPQ_MAGIC);
        assert_eq!(find_header(&bytes), Some(1024));
    }

    #[test]
    fn find_header_returns_none_when_absent() {
        assert_eq!(find_header(&[0u8; 2048]), None);
    }

    #[test]
    fn find_header_prefers_offset_zero() {
        let mut bytes = vec![0u8; 2048];
        bytes[0..4].copy_from_slice(&MPQ_MAGIC);
        assert_eq!(find_header(&bytes), Some(0));
    }

    #[test]
    fn sector_size_bases_on_512() {
        let header = ArchiveHeader {
            file_offset: 512,
            header_size: 32,
            archive_size: 0,
            format_version: 0,
            sector_size_shift: 3,
            hash_table_pos: 0x3B5E0,
            block_table_pos: 0x3B9E0,
            hash_table_size: 64,
            block_table_size: 17,
        };
        assert_eq!(header.sector_size(), 4096);
        // Table offsets are relative to the header, so the absolute position
        // adds `file_offset`.
        assert_eq!(header.hash_table_offset(), 512 + 0x3B5E0);
        assert_eq!(header.block_table_offset(), 512 + 0x3B9E0);
    }

    #[test]
    fn normalise_keeps_separators_and_uppercases() {
        assert_eq!(normalise_name("ui/triggerdata.txt"), "UI\\TRIGGERDATA.TXT");
    }

    #[test]
    fn block_flags_report_set_bits() {
        let f = BlockFlags(BlockFlags::COMPRESSED | BlockFlags::SINGLE_UNIT);
        assert!(f.is_compressed());
        assert!(!f.is_encrypted());
        assert!(f.has(BlockFlags::SINGLE_UNIT));
        assert_eq!(f.to_string(), "compressed|single-unit");
        assert_eq!(BlockFlags(0).to_string(), "none");
    }

    #[test]
    fn compression_name_does_not_guess_from_a_zero_low_byte() {
        // Real zlib members look like this: compressed bit set, low byte zero.
        let entry = BlockEntry {
            flags: BlockFlags(0x0400_0200),
            ..Default::default()
        };
        assert!(entry.flags.is_compressed());
        assert_eq!(entry.compression_method_bits(), 0);
        assert_eq!(entry.compression_name(), "compressed(algorithm not declared)");
    }

    #[test]
    fn hash_entry_empty_detection() {
        assert!(HashEntry { block_index: HASH_ENTRY_EMPTY, ..Default::default() }.is_empty());
        assert!(!HashEntry { block_index: 0, ..Default::default() }.is_empty());
    }

    #[test]
    fn parse_imports_reads_version_count_and_entries() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes()); // version
        bytes.extend_from_slice(&2u32.to_le_bytes()); // count
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        bytes.extend_from_slice(b"war3mapImported\\a.blp\0");
        bytes.extend_from_slice(&8u32.to_le_bytes()); // flags
        bytes.extend_from_slice(b"b.mdx\0");

        let imports = parse_imports(&bytes).unwrap();
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].path, "war3mapImported\\a.blp");
        assert_eq!(imports[0].flags, 0);
        assert_eq!(imports[1].path, "b.mdx");
        assert_eq!(imports[1].flags, 8);
    }

    #[test]
    fn parse_imports_rejects_truncated_input() {
        let bytes = vec![1, 0, 0, 0];
        assert!(parse_imports(&bytes).is_err());
    }

    #[test]
    fn known_member_names_cover_the_enumerated_formats() {
        for name in ["war3map.w3i", "war3map.w3e", "war3map.wts", "war3map.imp"] {
            assert!(KNOWN_MEMBER_NAMES.contains(&name), "{name} should be in the list");
        }
    }

    #[test]
    fn opening_a_non_archive_reports_no_header_not_panic() {
        let err = Archive::from_bytes(vec![0u8; 4096]).unwrap_err();
        assert!(matches!(err, MpqError::NoArchiveHeader));
    }
}

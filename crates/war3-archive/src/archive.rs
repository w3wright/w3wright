//! MPQ archive reading.
//!
//! Four properties of the container are easy to get wrong and all of them fail
//! silently:
//!
//! 1. **The header is not necessarily at offset 0.** Map files carry an `HM3W`
//!    prefix and the archive starts on the next 0x200 boundary, usually at 512.
//!    The magic has to be searched for in 0x200 steps.
//! 2. **Every offset is relative to the archive header**, including a block
//!    entry's `file_pos`. For an archive at offset 0 the two readings agree; for
//!    a map they differ by 512, and reading member data without adding the
//!    header offset yields 512 bytes of the wrong data with no error at all.
//! 3. **Table positions must be read from the header**, never assumed: the
//!    tables may sit before, after or inside the data region.
//! 4. **The tables are keyed by the hash of `(hash table)` and `(block table)`**,
//!    not by an ASCII constant. See [`HASH_TABLE_KEY_NAME`].

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use war3_core::diag::{DiagnosticCode, Diagnostics};
use war3_core::ParseError;

use crate::codec::{self, CodecError};
use crate::crypto::{
    bytes_to_u32_le, crypt_table, decrypt, hash_string, HashType, BLOCK_TABLE_KEY_NAME,
    HASH_TABLE_KEY_NAME,
};

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
/// Block index value marking a slot whose file was deleted.
///
/// Such a slot does **not** terminate a probe: the file may have been moved
/// further along the overflow chain.
const HASH_ENTRY_DELETED: u32 = 0xFFFF_FFFE;

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
    /// A member cannot be written the way it was asked to be.
    MemberNotWritable {
        /// The member's name.
        name: String,
        /// Why it cannot be relocated.
        reason: &'static str,
    },
    /// The archive has more members than the format can address.
    ///
    /// The original format caps the hash table at 32768 entries.
    TooManyMembers(usize),
    /// The bytes before the header are neither empty nor a whole number of
    /// 512-byte sectors, so the header would not land on a sector boundary.
    BadPrefixLength(usize),
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
            Self::MemberNotWritable { name, reason } => {
                write!(f, "cannot write member {name:?}: {reason}")
            }
            Self::TooManyMembers(n) => write!(
                f,
                "{n} members exceed the 32768 hash table entries the original format allows"
            ),
            Self::BadPrefixLength(n) => write!(
                f,
                "a prefix of {n} bytes does not put the header on a 512-byte sector boundary"
            ),
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
    /// Platform, a single byte at 0x0A.
    pub platform: u8,
    /// Index into the block table, or `0xFFFFFFFF` for an unused slot.
    pub block_index: u32,
}

impl HashEntry {
    /// Whether this slot is unused.
    ///
    /// An unused slot terminates a lookup: the name is not in the table.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.block_index == HASH_ENTRY_EMPTY
    }

    /// Whether this slot held a file that was deleted.
    ///
    /// Unlike [`HashEntry::is_empty`] this does **not** end a probe — the file
    /// may have overflowed further along the chain.
    #[must_use]
    pub const fn is_deleted(&self) -> bool {
        self.block_index == HASH_ENTRY_DELETED
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
    /// The member's key is adjusted by its block offset and file size.
    ///
    /// Only meaningful together with [`BlockFlags::ENCRYPTED`]. This is not a
    /// "fixed key": it selects a *different derivation* of the key, namely
    /// `(HashString(name, FileKey) + block_offset) ^ file_size`.
    pub const BLOCK_OFFSET_ADJUSTED_KEY: u32 = 0x0002_0000;
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
        if self.has(Self::BLOCK_OFFSET_ADJUSTED_KEY) {
            parts.push("offset-adjusted-key");
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

/// One member exactly as it is stored, ready to be written back.
///
/// This is the hand-off type between reading and writing: it carries the block
/// bytes verbatim plus the table metadata that describes them, so a rebuild can
/// copy a member the reader cannot even decompress (a PKWare-imploded `.wts`,
/// for instance) without understanding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMember {
    /// The member's name, as it will be hashed for lookup.
    pub name: String,
    /// The bytes as they appear on disk, including any sector table.
    pub block: Vec<u8>,
    /// Size after decompression.
    pub uncompressed_size: u32,
    /// The block table's flag word, preserved verbatim.
    pub flags: BlockFlags,
    /// Language from the hash table entry.
    pub locale: u16,
    /// Platform from the hash table entry.
    pub platform: u8,
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
    /// deliberate: every offset in the format is relative to the header, so the
    /// header's own position has to be added at each table and member read.
    /// Slicing the prefix off would mean either adjusting once and trusting that
    /// no path forgot, or forgetting — and forgetting reads 512 bytes of the
    /// wrong data with no error at all.
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

    /// The bytes before the header: the `HM3W` map prefix, or empty.
    ///
    /// Rewriting a map has to carry this over verbatim. It holds the map name
    /// the game shows in its map list, and its length is what puts the header at
    /// offset 512 rather than 0.
    #[must_use]
    pub fn prefix(&self) -> &[u8] {
        let end = usize::try_from(self.header.file_offset)
            .unwrap_or(0)
            .min(self.buffer.len());
        &self.buffer[..end]
    }

    /// One member's stored bytes and table metadata, for rewriting it verbatim.
    pub fn raw_member(&self, name: &str) -> MpqResult<RawMember> {
        let missing = || MpqError::NotFound(name.to_string());
        let slot = self.find_slot(name).ok_or_else(missing)?;
        let entry = self.hash_table.get(slot).ok_or_else(missing)?;
        let block = self
            .block_table
            .get(entry.block_index as usize)
            .ok_or_else(missing)?;

        let base = usize::try_from(self.header.file_offset).map_err(|_| MpqError::Eof)?;
        let start = base + block.file_pos as usize;
        let end = start
            .checked_add(block.compressed_size as usize)
            .ok_or(MpqError::Eof)?;
        let bytes = self.buffer.get(start..end).ok_or(MpqError::Eof)?;

        Ok(RawMember {
            name: name.to_string(),
            block: bytes.to_vec(),
            uncompressed_size: block.uncompressed_size,
            flags: block.flags,
            locale: entry.locale,
            platform: entry.platform,
        })
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
    ///
    /// Returns the names, and whether any byte sequence had to be replaced. A
    /// listfile written by a non-English World Editor is often in a local code
    /// page; rejecting the whole file because one name is not UTF-8 would lose
    /// every other name with it.
    fn read_listfile_names(&self) -> MpqResult<(Vec<String>, bool)> {
        let raw = self.read_file("(listfile)")?;
        let lossy = std::str::from_utf8(&raw).is_err();
        // Entries are separated by ';', CR, LF, or some combination.
        Ok((
            String::from_utf8_lossy(&raw)
                .split([';', '\r', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            lossy,
        ))
    }

    /// Builds the name index using the enumeration ladder.
    ///
    /// `(listfile)`, then the known-names list, then `war3map.imp`. Deduplicated
    /// case-insensitively, in that order of precedence.
    fn build_name_index(&mut self) {
        let mut diagnostics = std::mem::take(&mut self.diagnostics);

        // Step 1: (listfile)
        match self.read_listfile_names() {
            Ok((names, lossy)) => {
                if lossy {
                    diagnostics.push(war3_core::Diagnostic::warn(
                        DiagnosticCode::MpqListfileNotUtf8,
                        "(listfile) is not valid UTF-8; the undecodable bytes were replaced, so \
                         the affected member names will not resolve",
                    ));
                }
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

    /// The hash table slot holding a name, found by probing.
    ///
    /// The slot is needed, not just the block index: the entry also carries the
    /// member's locale and platform, which a rebuild has to preserve.
    fn find_slot(&self, name: &str) -> Option<usize> {
        let table = crypt_table();
        let upper = normalise_name(name);
        let mask = self.header.hash_table_size - 1;

        let start = hash_string(&table, HashType::TableOffset, &upper) & mask;
        let want_a = hash_string(&table, HashType::NameA, &upper);
        let want_b = hash_string(&table, HashType::NameB, &upper);

        for probe in 0..self.header.hash_table_size {
            let slot = (start + probe) & mask;
            let entry = self.hash_table[slot as usize];
            if entry.is_empty() {
                // An unused slot means the name is absent, so probing can stop.
                return None;
            }
            if entry.is_deleted() {
                // A deleted slot does not end the search: the file may have
                // overflowed past it.
                continue;
            }
            if entry.name_a == want_a
                && entry.name_b == want_b
                && (entry.block_index as usize) < self.block_table.len()
            {
                return Some(slot as usize);
            }
        }
        None
    }

    /// Looks up a block index by probing the hash table.
    fn lookup_raw(&self, name: &str) -> Option<u32> {
        let slot = self.find_slot(name)?;
        Some(self.hash_table[slot].block_index)
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
        // A member's position is relative to the archive header, exactly like
        // the table positions. For an archive at file offset 0 the two readings
        // coincide, which is why this is easy to get wrong: for a map the header
        // sits at 512 and every member would come back shifted by 512 bytes.
        let archive_base = usize::try_from(self.header.file_offset).map_err(|_| MpqError::Eof)?;
        let start = archive_base + entry.file_pos as usize;
        let size = entry.compressed_size as usize;
        let end = start.checked_add(size).ok_or(MpqError::Eof)?;
        let uncompressed = entry.uncompressed_size as usize;

        if uncompressed == 0 {
            return Ok(Vec::new());
        }
        if end > self.buffer.len() {
            return Err(MpqError::Eof);
        }

        let sector_size = self.header.sector_size() as usize;
        let sector_count = uncompressed.div_ceil(sector_size);
        let compressed = entry.flags.is_compressed();
        let key = if entry.flags.is_encrypted() {
            Some(self.file_key(entry, name))
        } else {
            None
        };

        // Single-block member: one "sector" holding the whole file, and no
        // sector offset table.
        if entry.flags.has(BlockFlags::SINGLE_UNIT) {
            let mut data = self.buffer[start..end].to_vec();
            if let Some(key) = key {
                decrypt_in_place(&mut data, key);
            }
            if !compressed {
                data.truncate(uncompressed.min(data.len()));
                return Ok(data);
            }
            // Decompression failure must be an error. Falling back to the raw
            // bytes would hand the caller data that looks plausible but is
            // completely misplaced.
            return self.decode_sector(&data, uncompressed);
        }

        // Multi-block. A **compressed** member carries a sector offset table
        // with one more entry than there are sectors, the last being the end of
        // the data. An uncompressed one carries none at all: its sector
        // boundaries follow from the sector size, so reading a table here would
        // parse member data as offsets.
        let bounds: Vec<(usize, usize)> = if compressed {
            let table_bytes = (sector_count + 1) * 4;
            if size < table_bytes {
                return Err(MpqError::SectorTableOutOfRange);
            }
            let mut offsets = bytes_to_u32_le(&self.buffer[start..start + table_bytes]);
            if let Some(key) = key {
                // The sector offset table is keyed with `file_key - 1`, not
                // `file_key`.
                decrypt(&crypt_table(), &mut offsets, key.wrapping_sub(1));
            }
            (0..sector_count)
                .map(|i| (offsets[i] as usize, offsets[i + 1] as usize))
                .collect()
        } else {
            (0..sector_count)
                .map(|i| {
                    let from = i * sector_size;
                    (from, (from + sector_size).min(size))
                })
                .collect()
        };

        let mut out = Vec::with_capacity(uncompressed);
        for (i, (from, to)) in bounds.into_iter().enumerate() {
            if to < from || start + to > end {
                return Err(MpqError::SectorTableOutOfRange);
            }
            if from == to {
                // An empty sector means the whole span is zeroes.
                out.resize(out.len() + sector_size, 0);
                continue;
            }
            let mut data = self.buffer[start + from..start + to].to_vec();
            if let Some(key) = key {
                decrypt_in_place(&mut data, key.wrapping_add(i as u32));
            }
            if !compressed {
                out.extend_from_slice(&data);
                continue;
            }
            // The last sector is usually shorter than a full sector, so its
            // expected length comes from the member's total size. Checking
            // every sector against the full sector size would reject almost
            // every real file.
            let expected = if i + 1 == sector_count {
                uncompressed.saturating_sub(i * sector_size)
            } else {
                sector_size
            };
            out.extend_from_slice(&self.decode_sector(&data, expected)?);
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

    /// Turns one stored sector of a compressed member into its data.
    ///
    /// Such a sector is stored either raw or compressed, and the format decides
    /// by size: a raw sector occupies exactly as many bytes as the data it
    /// holds, while a compressed one is smaller and carries a leading
    /// compression mask byte (`0x02` deflate, `0x08` PKWare implode, ...).
    ///
    /// Blizzard writes the raw form whenever compressing the sector would not
    /// have saved at least two bytes, so both cases occur in real files.
    fn decode_sector(&self, data: &[u8], expected: usize) -> MpqResult<Vec<u8>> {
        if data.len() >= expected {
            let mut raw = data.to_vec();
            raw.truncate(expected);
            return Ok(raw);
        }
        let (mask, body) = data.split_first().ok_or(MpqError::SectorTableOutOfRange)?;
        Ok(codec::decompress(body, u32::from(*mask), expected)?)
    }

    /// Derives the key used to decrypt a member's contents.
    ///
    /// Ordinary members use the `FileKey` hash of the file name with the
    /// directory part removed. A member flagged
    /// [`BlockFlags::BLOCK_OFFSET_ADJUSTED_KEY`] has that key adjusted by its
    /// block offset and file size instead.
    fn file_key(&self, entry: &BlockEntry, name: &str) -> u32 {
        let table = crypt_table();
        let upper = normalise_name(name);
        let base = upper.rsplit('\\').next().unwrap_or(&upper);
        let key = hash_string(&table, HashType::FileKey, base);
        if entry.flags.has(BlockFlags::BLOCK_OFFSET_ADJUSTED_KEY) {
            return key.wrapping_add(entry.file_pos) ^ entry.uncompressed_size;
        }
        key
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
    // The key is `HashString("(hash table)", MPQ_HASH_FILE_KEY)`, not an ASCII
    // constant such as `HASH`. See `HASH_TABLE_KEY_NAME`.
    let key = hash_string(table, HashType::FileKey, HASH_TABLE_KEY_NAME);
    decrypt(table, &mut words, key);

    Ok(words
        .chunks_exact(4)
        .map(|c| HashEntry {
            name_a: c[0],
            name_b: c[1],
            locale: (c[2] & 0xFFFF) as u16,
            // Field layout is `int16 Language` at 0x08 followed by `int8
            // Platform` at 0x0A, so the platform is the low byte of the upper
            // half — the top byte is padding.
            platform: ((c[2] >> 16) & 0xFF) as u8,
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
    // Same as the hash table: `HashString("(block table)", MPQ_HASH_FILE_KEY)`.
    let key = hash_string(table, HashType::FileKey, BLOCK_TABLE_KEY_NAME);
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
/// This is the form used both for hashing and for deriving decryption keys, so
/// the uppercasing must match [`crypto::hash_string`](crate::crypto::hash_string)'s
/// exactly: **ASCII only**.
/// Unicode uppercasing can change a name's byte length (`ß` becomes `SS`), which
/// silently changes its hash and therefore makes imported files with non-ASCII
/// names unresolvable.
#[must_use]
pub fn normalise_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            let c = if c == '/' { '\\' } else { c };
            c.to_ascii_uppercase()
        })
        .collect()
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
    use crate::crypto::encrypt;

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
        assert_eq!(
            entry.compression_name(),
            "compressed(algorithm not declared)"
        );
    }

    #[test]
    fn hash_entry_empty_detection() {
        assert!(HashEntry {
            block_index: HASH_ENTRY_EMPTY,
            ..Default::default()
        }
        .is_empty());
        assert!(!HashEntry {
            block_index: 0,
            ..Default::default()
        }
        .is_empty());
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
            assert!(
                KNOWN_MEMBER_NAMES.contains(&name),
                "{name} should be in the list"
            );
        }
    }

    #[test]
    fn opening_a_non_archive_reports_no_header_not_panic() {
        let err = Archive::from_bytes(vec![0u8; 4096]).unwrap_err();
        assert!(matches!(err, MpqError::NoArchiveHeader));
    }

    /// The plaintext of the compressed member in [`tiny_archive`].
    fn compressed_payload() -> Vec<u8> {
        b"war3wright".repeat(40)
    }

    /// A `zlib` stream for `b"war3wright" * 40`, from an independent
    /// implementation (`python -c "import zlib;print(zlib.compress(b'war3wright'*40,9).hex())"`).
    ///
    /// It is 23 bytes, so with its mask byte it is comfortably below the 400
    /// bytes the sector must hold — which is what makes the reader take its
    /// compressed branch rather than treat the sector as stored.
    const COMPRESSED_SECTOR: &[u8] = &[
        0x78, 0xDA, 0x2B, 0x4F, 0x2C, 0x32, 0x2E, 0x2F, 0xCA, 0x4C, 0xCF, 0x28, 0x29, 0x1F, 0x65,
        0x0D, 0x02, 0x16, 0x00, 0x83, 0x39, 0xA2, 0xD1,
    ];

    /// A `zlib` stream for `b"a" * 4096`, from the same independent
    /// implementation. A full sector, so it can be the first sector of a
    /// multi-block member.
    const COMPRESSED_FULL_SECTOR: &[u8] = &[
        0x78, 0xDA, 0xED, 0xC1, 0x01, 0x0D, 0x00, 0x00, 0x00, 0xC2, 0xA0, 0xAC, 0xEF, 0x5F, 0xC2,
        0x1E, 0x0E, 0x28, 0x00, 0x00, 0x00, 0xE0, 0xDD, 0x00, 0xEF, 0xCB, 0x10, 0x5B,
    ];

    /// A whole archive built here rather than read from disk.
    ///
    /// The map samples are gitignored and the generated fixture is a build
    /// artifact, so neither can guard the read path on its own. This one pins
    /// all of it in a plain `cargo test`: the header behind a 512-byte `HM3W`
    /// prefix, the two table keys, the cipher, **archive-relative** member
    /// offsets, hash lookup, the known-names fallback, and both choices the
    /// reader makes between a stored and a compressed sector.
    fn tiny_archive() -> Vec<u8> {
        // Absolute offset of the header, and of the first member.
        const HEADER: usize = 0x200;
        const DATA: usize = HEADER + 32;
        const SECTOR: usize = 4096;

        let stored = b"stored member, not compressed at all".to_vec();
        // A multi-block member with two full sectors: the first deflated, the
        // second stored raw. Which is which is decided by stored size alone.
        let raw_sector = vec![0x5Au8; SECTOR];
        let multi_first = {
            let mut b = vec![0x02];
            b.extend_from_slice(COMPRESSED_FULL_SECTOR);
            b
        };
        let multi = {
            // Two sectors, so the offset table has three entries.
            let table_bytes = 3 * 4;
            let mut b = Vec::new();
            let end_of_first = table_bytes + multi_first.len();
            b.extend_from_slice(&(table_bytes as u32).to_le_bytes());
            b.extend_from_slice(&(end_of_first as u32).to_le_bytes());
            b.extend_from_slice(&((end_of_first + raw_sector.len()) as u32).to_le_bytes());
            b.extend_from_slice(&multi_first);
            b.extend_from_slice(&raw_sector);
            b
        };

        // (name, block on disk, uncompressed length, block flags)
        let members: [(&str, Vec<u8>, u32, u32); 3] = [
            (
                "war3map.w3i",
                {
                    let mut b = vec![0x02];
                    b.extend_from_slice(COMPRESSED_SECTOR);
                    b
                },
                compressed_payload().len() as u32,
                0x8100_0200, // exists | compressed | single-unit
            ),
            (
                "war3map.j",
                stored.clone(),
                stored.len() as u32,
                0x8100_0000, // exists | single-unit
            ),
            (
                "war3map.w3e",
                multi,
                (SECTOR * 2) as u32,
                0x8400_0200, // exists | compressed | multi-block
            ),
        ];

        // Lay the members out, remembering archive-relative offsets.
        let mut cursor = DATA;
        let mut placed: Vec<(u32, usize, u32, u32)> = Vec::new();
        for (_, block, uncompressed, flags) in &members {
            placed.push(((cursor - HEADER) as u32, block.len(), *uncompressed, *flags));
            cursor += block.len();
        }
        let hash_pos = cursor;
        let hash_size = 16u32;
        let block_pos = hash_pos + hash_size as usize * 16;
        let archive_end = block_pos + members.len() * 16;

        let table = crypt_table();
        let mut hash_words = vec![0xFFFF_FFFFu32; hash_size as usize * 4];
        let mut block_words = vec![0u32; members.len() * 4];
        for (index, (name, _, _, _)) in members.iter().enumerate() {
            let (pos, size, uncompressed, flags) = placed[index];
            let upper = name.to_uppercase();
            let mut slot = hash_string(&table, HashType::TableOffset, &upper) & (hash_size - 1);
            let want_a = hash_string(&table, HashType::NameA, &upper);
            while hash_words[slot as usize * 4 + 3] != 0xFFFF_FFFF {
                slot = (slot + 1) & (hash_size - 1);
            }
            hash_words[slot as usize * 4] = want_a;
            hash_words[slot as usize * 4 + 1] = hash_string(&table, HashType::NameB, &upper);
            hash_words[slot as usize * 4 + 2] = 0;
            hash_words[slot as usize * 4 + 3] = index as u32;

            block_words[index * 4] = pos;
            block_words[index * 4 + 1] = size as u32;
            block_words[index * 4 + 2] = uncompressed;
            block_words[index * 4 + 3] = flags;
        }
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

        let mut out = vec![0u8; archive_end];
        out[0..4].copy_from_slice(b"HM3W");
        out[HEADER..HEADER + 4].copy_from_slice(&MPQ_MAGIC);
        out[HEADER + 4..HEADER + 8].copy_from_slice(&32u32.to_le_bytes());
        out[HEADER + 8..HEADER + 12]
            .copy_from_slice(&((archive_end - HEADER) as u32).to_le_bytes());
        out[HEADER + 12..HEADER + 14].copy_from_slice(&0u16.to_le_bytes());
        out[HEADER + 14..HEADER + 16].copy_from_slice(&3u16.to_le_bytes());
        // Table and member positions are all relative to the header.
        out[HEADER + 16..HEADER + 20].copy_from_slice(&((hash_pos - HEADER) as u32).to_le_bytes());
        out[HEADER + 20..HEADER + 24].copy_from_slice(&((block_pos - HEADER) as u32).to_le_bytes());
        out[HEADER + 24..HEADER + 28].copy_from_slice(&hash_size.to_le_bytes());
        out[HEADER + 28..HEADER + 32].copy_from_slice(&(members.len() as u32).to_le_bytes());

        let mut at = DATA;
        for (_, block, _, _) in &members {
            out[at..at + block.len()].copy_from_slice(block);
            at += block.len();
        }
        for (i, word) in hash_words.iter().enumerate() {
            out[hash_pos + i * 4..hash_pos + i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        for (i, word) in block_words.iter().enumerate() {
            out[block_pos + i * 4..block_pos + i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }

    #[test]
    fn reads_a_whole_archive_whose_header_is_not_at_offset_zero() {
        let archive = Archive::from_bytes(tiny_archive()).unwrap();
        assert_eq!(archive.header().file_offset, 512);

        // Members are found through the known-names fallback: this archive has
        // no (listfile).
        assert_eq!(
            archive.read_file("war3map.w3i").unwrap(),
            compressed_payload(),
            "a compressed single-unit member must inflate"
        );
        assert_eq!(
            archive.read_file("war3map.j").unwrap(),
            b"stored member, not compressed at all",
            "a stored member must come back as is"
        );

        // Multi-block, with sector 0 deflated and sector 1 stored raw. The
        // reader tells them apart by stored size, and getting that wrong here
        // yields the deflate stream itself rather than the data.
        let terrain = archive.read_file("war3map.w3e").unwrap();
        assert_eq!(terrain.len(), 8192);
        assert!(terrain[..4096].iter().all(|&b| b == b'a'));
        assert!(terrain[4096..].iter().all(|&b| b == 0x5A));
    }

    #[test]
    fn a_header_away_from_zero_is_reported_as_a_diagnostic() {
        let archive = Archive::from_bytes(tiny_archive()).unwrap();
        assert!(archive
            .diagnostics()
            .items()
            .iter()
            .any(|d| d.code == DiagnosticCode::MpqHeaderOffset));
    }
}

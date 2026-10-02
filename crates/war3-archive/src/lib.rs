//! MPQ (MoPaQ) container reading.
//!
//! Implements the format from its published specification: header discovery,
//! hash and block tables, zlib decompression and the member enumeration ladder.
//!
//! Three container properties shape the API and are documented on the items
//! that deal with them: the header is not at offset 0, every offset in the
//! format — table positions in the header and member positions in the block
//! table alike — is relative to the archive header rather than to the file, and
//! table positions must be read from the header rather than assumed.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod archive;
pub mod build;
pub mod codec;
pub mod crypto;

pub use archive::{
    find_header, normalise_name, parse_imports, Archive, ArchiveHeader, ArchiveInfo, BlockEntry,
    BlockFlags, HashEntry, ImportEntry, RawMember, KNOWN_MEMBER_NAMES, MPQ_MAGIC, MPQ_MAGIC_LEN,
    MPQ_SEARCH_STEP,
};
pub use build::{ArchiveBuilder, ArchivePatcher};
pub use codec::{decompress, inflate, zlib_decompress};
pub use crypto::{
    bytes_to_u32_le, crypt_table, decrypt, encrypt, hash_string, HashType, BLOCK_TABLE_KEY_NAME,
    HASH_TABLE_KEY_NAME,
};

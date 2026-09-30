//! MPQ (MoPaQ) container reading.
//!
//! Implements the format from its published specification: header discovery,
//! hash and block tables, zlib decompression and the member enumeration ladder.
//!
//! Three container properties shape the API and are documented on the items
//! that deal with them: the header is not at offset 0, header offsets are
//! relative to the header while member offsets are relative to the file, and
//! table positions must be read from the header rather than assumed.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod archive;
pub mod codec;
pub mod crypto;

pub use archive::{
    find_header, normalise_name, parse_imports, Archive, ArchiveHeader, ArchiveInfo, BlockEntry,
    BlockFlags, HashEntry, ImportEntry, KNOWN_MEMBER_NAMES, MPQ_MAGIC, MPQ_MAGIC_LEN,
    MPQ_SEARCH_STEP,
};
pub use codec::{decompress, inflate, zlib_decompress};
pub use crypto::{bytes_to_u32_le, crypt_table, decrypt, hash_string, HashType};

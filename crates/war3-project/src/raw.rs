//! The container for a member this workspace cannot decode.
//!
//! # Why a member sometimes needs a container of its own
//!
//! A block that cannot be decompressed (`0x08`, PKWare implode, is the one that
//! occurs in real maps) has **no content to store** — all that exists is the
//! block's stored bytes plus the four numbers the block table carried.
//!
//! Writing those bytes back as if they were content would hand the game
//! compressed data and corrupt the member *silently*, so the file keeps them
//! behind a magic of its own and is written back with
//! [`ArchiveBuilder::add_raw`](war3_archive::ArchiveBuilder::add_raw) rather than
//! `add_stored`. The name is not stored: the manifest is the authority for it.

use war3_archive::RawMember;
use war3_core::{Error, Result};

/// Marks a file as a stored block rather than a member's content.
pub const MAGIC: [u8; 8] = *b"W3RAWMEM";
/// `MAGIC` + flags + uncompressed size + locale + platform + one reserved byte.
pub const HEADER_LEN: usize = 8 + 4 + 4 + 2 + 1 + 1;

/// Wraps a member's stored block in the container.
#[must_use]
pub fn encode(member: &RawMember) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + member.block.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&member.flags.0.to_le_bytes());
    out.extend_from_slice(&member.uncompressed_size.to_le_bytes());
    out.extend_from_slice(&member.locale.to_le_bytes());
    out.push(member.platform);
    out.push(0);
    out.extend_from_slice(&member.block);
    out
}

/// Unwraps a member stored by [`encode`].
///
/// `name` is supplied by the caller because the container does not carry it.
///
/// # Errors
///
/// A file that is too short, or whose magic does not match. Both mean the file
/// was not written by this code — a hand-edited or truncated member — and both
/// are reported rather than interpreted.
pub fn decode(name: &str, bytes: &[u8]) -> Result<RawMember> {
    if bytes.len() < HEADER_LEN {
        return Err(Error::msg(format!(
            "{name}: a stored-block file is at least {HEADER_LEN} bytes, this one is {}",
            bytes.len()
        )));
    }
    if bytes[..MAGIC.len()] != MAGIC {
        return Err(Error::msg(format!(
            "{name}: not a stored-block file (expected magic W3RAWMEM)"
        )));
    }
    let flags = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let uncompressed_size = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    let locale = u16::from_le_bytes([bytes[16], bytes[17]]);
    let platform = bytes[18];
    Ok(RawMember {
        name: name.to_string(),
        uncompressed_size,
        block: bytes[HEADER_LEN..].to_vec(),
        flags: war3_archive::BlockFlags(flags),
        locale,
        platform,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use war3_archive::BlockFlags;

    fn member() -> RawMember {
        RawMember {
            name: "war3map.wts".to_string(),
            uncompressed_size: 4096,
            block: vec![0x08, 0x1F, 0x2E, 0x00, 0xFF],
            flags: BlockFlags(0x8000_0000 | 0x0000_0100 | 0x0000_0200),
            locale: 0,
            platform: 0,
        }
    }

    #[test]
    fn encode_then_decode_keeps_every_field() {
        let original = member();
        let back = decode("war3map.wts", &encode(&original)).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn the_name_comes_from_the_caller_not_the_file() {
        let back = decode("Renamed.wts", &encode(&member())).unwrap();
        assert_eq!(back.name, "Renamed.wts");
    }

    #[test]
    fn the_block_is_not_interpreted() {
        let encoded = encode(&member());
        // The block bytes survive byte for byte, including the 0x08 mask that
        // says "this is compressed and you cannot read it".
        assert_eq!(&encoded[HEADER_LEN..], &[0x08, 0x1F, 0x2E, 0x00, 0xFF]);
    }

    #[test]
    fn a_truncated_file_is_reported_not_guessed() {
        let err = decode("x", &[0u8; 4]).unwrap_err().to_string();
        assert!(err.contains("at least"), "{err}");
    }

    #[test]
    fn a_wrong_magic_is_reported() {
        let mut bytes = encode(&member());
        bytes[0] = b'X';
        let err = decode("x", &bytes).unwrap_err().to_string();
        assert!(err.contains("W3RAWMEM"), "{err}");
    }
}

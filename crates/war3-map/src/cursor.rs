//! A bounds-checked little-endian read cursor.
//!
//! Every read reports the offset at which it ran out of data. Slicing a `&[u8]`
//! directly would either panic or produce an `UnexpectedEof` with no position,
//! and when a format will not parse the offset is usually the first thing worth
//! knowing.

use war3_core::ParseError;

/// A little-endian read cursor.
///
/// Reads return [`ParseError::UnexpectedEof`] rather than panicking or silently
/// returning zero, since a zero would let a damaged file parse far beyond the
/// actual damage.
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Wraps a byte slice.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Current offset.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.pos
    }

    /// Total length.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the underlying slice is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Bytes left.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    /// Whether the cursor has reached the end.
    #[must_use]
    pub const fn is_at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Takes `len` bytes and advances.
    pub fn take(&mut self, len: usize) -> Result<&'a [u8], ParseError> {
        let end = self.pos.checked_add(len).ok_or(ParseError::UnexpectedEof {
            offset: self.pos,
            needed: len,
            available: 0,
        })?;
        let slice = self.bytes.get(self.pos..end).ok_or(ParseError::UnexpectedEof {
            offset: self.pos,
            needed: len,
            available: self.remaining(),
        })?;
        self.pos = end;
        Ok(slice)
    }

    /// Moves to an absolute offset. Forward only.
    pub fn seek(&mut self, pos: usize) -> Result<(), ParseError> {
        if pos > self.bytes.len() {
            return Err(ParseError::UnexpectedEof {
                offset: pos,
                needed: 0,
                available: self.bytes.len(),
            });
        }
        self.pos = pos;
        Ok(())
    }

    /// Reads a `u8`.
    pub fn u8(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }

    /// Reads an `i16`.
    pub fn i16(&mut self) -> Result<i16, ParseError> {
        let b = self.take(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }

    /// Reads a `u16`.
    ///
    /// Bit-field values must go through this rather than being read as `i16` and
    /// masked afterwards, which hits sign extension.
    pub fn u16(&mut self) -> Result<u16, ParseError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// Reads an `i32`.
    pub fn i32(&mut self) -> Result<i32, ParseError> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Reads a `u32`.
    pub fn u32(&mut self) -> Result<u32, ParseError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Reads an `f32`.
    ///
    /// Finiteness is not checked: some formats use values such as `0x4F800000`
    /// as sentinels, so callers that need to reject NaN do so themselves.
    pub fn f32(&mut self) -> Result<f32, ParseError> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// Reads a NUL-terminated string.
    ///
    /// Strings in these files are ASCII in practice, so invalid UTF-8 usually
    /// means the read position has drifted rather than that another encoding is
    /// in use.
    pub fn cstr(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        let rest = self.bytes.get(start..).ok_or(ParseError::UnexpectedEof {
            offset: start,
            needed: 1,
            available: 0,
        })?;
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(ParseError::UnexpectedEof {
                offset: start,
                needed: rest.len() + 1,
                available: rest.len(),
            })?;
        let text = std::str::from_utf8(&rest[..end])
            .map_err(|_| ParseError::BadString { offset: start })?;
        self.pos = start + end + 1;
        Ok(text.to_string())
    }

    /// Reads a four-byte identifier.
    pub fn fourcc(&mut self) -> Result<war3_core::FourCC, ParseError> {
        let b = self.take(4)?;
        Ok(war3_core::FourCC([b[0], b[1], b[2], b[3]]))
    }

    /// Reads `count` `f32`s.
    pub fn f32s(&mut self, count: usize) -> Result<Vec<f32>, ParseError> {
        (0..count).map(|_| self.f32()).collect()
    }

    /// Reads `count` `u32`s.
    pub fn u32s(&mut self, count: usize) -> Result<Vec<u32>, ParseError> {
        (0..count).map(|_| self.u32()).collect()
    }

    /// Whether the next four bytes are `magic`, without consuming them.
    pub fn peek_magic(&self, magic: &[u8; 4]) -> bool {
        self.bytes.get(self.pos..self.pos + 4) == Some(magic.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_scalars() {
        let data = [0x01, 0x02, 0x03, 0x04, 0xFF, 0xFF];
        let mut c = Cursor::new(&data);
        assert_eq!(c.u16().unwrap(), 0x0201);
        assert_eq!(c.i16().unwrap(), 0x0403);
        assert_eq!(c.i16().unwrap(), -1);
        assert!(c.is_at_end());
    }

    #[test]
    fn eof_reports_offset_needed_and_available() {
        let data = [1, 2, 3];
        let mut c = Cursor::new(&data);
        c.take(2).unwrap();
        let err = c.u32().unwrap_err();
        assert_eq!(
            err,
            ParseError::UnexpectedEof { offset: 2, needed: 4, available: 1 }
        );
    }

    #[test]
    fn cstr_stops_at_nul_and_advances_past_it() {
        let data = b"abc\0def\0";
        let mut c = Cursor::new(data);
        assert_eq!(c.cstr().unwrap(), "abc");
        assert_eq!(c.position(), 4);
        assert_eq!(c.cstr().unwrap(), "def");
        assert!(c.is_at_end());
    }

    #[test]
    fn cstr_without_terminator_is_eof() {
        let data = b"abc";
        let mut c = Cursor::new(data);
        assert!(matches!(c.cstr(), Err(ParseError::UnexpectedEof { .. })));
    }

    #[test]
    fn cstr_with_invalid_utf8_reports_bad_string() {
        let data = [0xB0, 0xA1, 0x00];
        let mut c = Cursor::new(&data);
        assert_eq!(c.cstr(), Err(ParseError::BadString { offset: 0 }));
    }

    #[test]
    fn peek_magic_does_not_advance() {
        let data = b"W3E!rest";
        let c = Cursor::new(data);
        assert!(c.peek_magic(b"W3E!"));
        assert_eq!(c.position(), 0);
        assert!(!c.peek_magic(b"MPQ\x1a"));
    }

    #[test]
    fn seek_forward_only_within_bounds() {
        let data = [0u8; 8];
        let mut c = Cursor::new(&data);
        assert!(c.seek(4).is_ok());
        assert_eq!(c.remaining(), 4);
        assert!(c.seek(99).is_err());
    }
}

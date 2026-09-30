//! Four-byte identifiers, as used throughout the Warcraft III file formats.

use std::fmt;

use crate::error::ParseError;

/// A four-byte identifier.
///
/// A single `FourCC` has three distinct representations, and mixing them up
/// produces bugs that are hard to trace back:
///
/// | Form | Example | Used by |
/// | --- | --- | --- |
/// | raw bytes | `[b'L', b'T', b'l', b't']` | on-disk layout |
/// | text abbreviation | `"LTlt"` | humans, logs |
/// | integer | `0x746C544C` | object files (`.w3u` etc.) |
///
/// The integer form is little-endian: the first byte is the least significant.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct FourCC(pub [u8; 4]);

impl FourCC {
    /// The all-zero identifier, which means "none" in object files.
    pub const ZERO: Self = Self([0, 0, 0, 0]);

    /// Builds an identifier from its four bytes.
    #[inline]
    #[must_use]
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    /// Builds an identifier from a string, padding with NUL or truncating.
    ///
    /// Truncation loses information, so parsing code should prefer
    /// [`FourCC::from_bytes_exact`].
    #[must_use]
    pub fn from_str_lossy(s: &str) -> Self {
        let mut b = [0u8; 4];
        for (dst, src) in b.iter_mut().zip(s.as_bytes()) {
            *dst = *src;
        }
        Self(b)
    }

    /// Builds an identifier from exactly four bytes, rejecting other lengths.
    pub fn from_bytes_exact(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() != 4 {
            return Err(ParseError::BadField {
                field: "FourCC",
                reason: format!("expected 4 bytes, got {}", bytes.len()),
            });
        }
        let mut b = [0u8; 4];
        b.copy_from_slice(bytes);
        Ok(Self(b))
    }

    /// The raw four bytes.
    #[inline]
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 4] {
        self.0
    }

    /// The little-endian integer form, as stored in object files.
    #[inline]
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        u32::from_le_bytes(self.0)
    }

    /// Rebuilds from the little-endian integer form.
    #[inline]
    #[must_use]
    pub const fn from_u32(v: u32) -> Self {
        Self(v.to_le_bytes())
    }

    /// Whether all four bytes are zero.
    #[inline]
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.to_u32() == 0
    }

    /// Whether all four bytes are printable ASCII.
    ///
    /// Used to decide whether an identifier can be shown as text; map files do
    /// contain identifiers that would otherwise garble the output.
    #[must_use]
    pub const fn is_printable(self) -> bool {
        let mut i = 0;
        while i < 4 {
            let b = self.0[i];
            // Written as explicit comparisons because `Range::contains` is not
            // a `const fn`.
            if b < 0x20 || b >= 0x7F {
                return false;
            }
            i += 1;
        }
        true
    }

    /// The text abbreviation, or `None` when a byte is not printable ASCII.
    #[must_use]
    pub fn as_str(self) -> Option<String> {
        if self.is_printable() {
            // All four bytes are < 0x7F, so this is valid UTF-8 by construction.
            String::from_utf8(self.0.to_vec()).ok()
        } else {
            None
        }
    }
}

impl fmt::Display for FourCC {
    /// Prints the abbreviation when printable, otherwise the integer in hex.
    ///
    /// No decorative wrapping, so the output can be pasted back into a command
    /// line or into an SLK file.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_printable() {
            for b in self.0 {
                write!(f, "{}", b as char)?;
            }
            Ok(())
        } else {
            write!(f, "0x{:08X}", self.to_u32())
        }
    }
}

impl fmt::Debug for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_printable() {
            write!(f, "FourCC({})", self)
        } else {
            write!(f, "FourCC({:02X?})", self.0)
        }
    }
}

impl From<[u8; 4]> for FourCC {
    fn from(v: [u8; 4]) -> Self {
        Self(v)
    }
}

impl From<FourCC> for [u8; 4] {
    fn from(v: FourCC) -> Self {
        v.0
    }
}

impl From<u32> for FourCC {
    fn from(v: u32) -> Self {
        Self::from_u32(v)
    }
}

impl From<FourCC> for u32 {
    fn from(v: FourCC) -> Self {
        v.to_u32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_bytes_and_u32() {
        let id = FourCC::new(*b"LTlt");
        assert_eq!(id.to_bytes(), *b"LTlt");
        // Little-endian: `L` is the low byte.
        assert_eq!(id.to_u32(), 0x746C_544C);
        assert_eq!(FourCC::from_u32(id.to_u32()), id);
        assert_eq!(FourCC::from_u32(id.to_u32()).to_string(), "LTlt");
    }

    #[test]
    fn display_prints_abbreviation_when_printable() {
        assert_eq!(FourCC::new(*b"hfoo").to_string(), "hfoo");
    }

    #[test]
    fn display_falls_back_to_hex_when_not_printable() {
        assert_eq!(
            FourCC::new([0x00, 0x01, 0xFF, 0x7F]).to_string(),
            "0x7FFF0100"
        );
        assert_eq!(
            FourCC::new([0xFF, 0x00, 0x00, 0x00]).to_string(),
            "0x000000FF"
        );
    }

    #[test]
    fn zero_is_detected() {
        assert!(FourCC::ZERO.is_zero());
        assert!(!FourCC::new(*b"\0\0\0a").is_zero());
    }

    #[test]
    fn from_str_lossy_pads_and_truncates() {
        assert_eq!(FourCC::from_str_lossy("ab"), FourCC::new(*b"ab\0\0"));
        assert_eq!(FourCC::from_str_lossy("abcde"), FourCC::new(*b"abcd"));
    }

    #[test]
    fn from_bytes_exact_rejects_wrong_length() {
        assert!(FourCC::from_bytes_exact(b"abc").is_err());
        assert_eq!(
            FourCC::from_bytes_exact(b"abcd").unwrap(),
            FourCC::new(*b"abcd")
        );
    }

    #[test]
    fn as_str_rejects_non_printable() {
        assert_eq!(FourCC::new(*b"Ldrt").as_str().as_deref(), Some("Ldrt"));
        assert!(FourCC::new([0xFF, 0, 0, 0]).as_str().is_none());
    }
}

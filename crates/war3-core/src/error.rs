//! Error types.

use std::fmt;

/// A recoverable parse failure: the problem is known, and parsing cannot continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Needed N bytes, but only M were available.
    UnexpectedEof {
        /// Read position where the failure happened.
        offset: usize,
        /// Bytes the parser wanted.
        needed: usize,
        /// Bytes actually left.
        available: usize,
    },

    /// The leading magic bytes did not match.
    BadMagic {
        /// Expected magic, in human-readable form.
        expected: &'static str,
        /// Bytes found instead.
        found: [u8; 4],
    },

    /// The file is well-formed but its version is not supported.
    ///
    /// This is reported rather than degrading to an empty result: silently
    /// returning nothing for an unknown version makes the failure show up far
    /// away from its cause.
    UnsupportedVersion {
        /// Which format, e.g. `"war3map.w3e"`.
        format: &'static str,
        /// Version found in the file.
        found: i64,
        /// Versions this build accepts.
        supported: &'static [i64],
    },

    /// A structural cross-check failed: the individual fields are valid, but
    /// their combination is impossible.
    Validation {
        /// Short name of the check.
        check: &'static str,
        /// Human-readable detail, including the numbers involved.
        detail: String,
    },

    /// A single field held an illegal value.
    BadField {
        /// Field name.
        field: &'static str,
        /// Why it is illegal.
        reason: String,
    },

    /// A string was not valid UTF-8.
    ///
    /// Strings in these files are ASCII in practice, so invalid UTF-8 usually
    /// means the read position drifted rather than that another encoding is in
    /// use.
    BadString {
        /// Offset of the offending string.
        offset: usize,
    },

    /// A requested member file does not exist.
    NotFound(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof {
                offset,
                needed,
                available,
            } => write!(
                f,
                "truncated at offset {offset}: needed {needed} bytes, {available} available"
            ),
            Self::BadMagic { expected, found } => {
                write!(f, "bad magic: expected {expected:?}, found {found:02X?}")
            }
            Self::UnsupportedVersion {
                format,
                found,
                supported,
            } => {
                write!(
                    f,
                    "{format} version {found} is not supported (supported: {supported:?})"
                )
            }
            Self::Validation { check, detail } => {
                write!(f, "validation failed [{check}]: {detail}")
            }
            Self::BadField { field, reason } => write!(f, "field {field} is invalid: {reason}"),
            Self::BadString { offset } => {
                write!(f, "string at offset {offset} is not valid UTF-8")
            }
            Self::NotFound(name) => write!(f, "member file {name:?} not found"),
        }
    }
}

impl std::error::Error for ParseError {}

/// The boxed error type used across crates.
///
/// There is deliberately no blanket `impl<E: std::error::Error> From<E> for
/// Error`: it would conflict with `From<ParseError>`, since `ParseError`
/// intentionally does not implement `std::error::Error` (this crate has no
/// dependencies and does not pull in an error-deriving crate).
///
/// The cost is that each crate with its own error enum writes one `From` impl.
/// That is cheaper than the ambiguity a blanket impl introduces at `?` sites.
#[derive(Debug)]
pub struct Error(Box<dyn std::error::Error + Send + Sync + 'static>);

impl Error {
    /// Wraps an error.
    pub fn new<E>(err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self(Box::new(err))
    }

    /// Wraps a message. Handy for `ok_or_else` on `Option`s.
    pub fn msg(message: impl Into<String>) -> Self {
        Self(Box::new(MessageError(message.into())))
    }

    /// The wrapped error.
    #[must_use]
    pub fn as_dyn(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        &*self.0
    }
}

#[derive(Debug)]
struct MessageError(String);

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MessageError {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.0)
    }
}

impl From<ParseError> for Error {
    fn from(err: ParseError) -> Self {
        Self::new(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::new(err)
    }
}

/// Result alias used across the workspace.
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_display_mentions_numbers() {
        let e = ParseError::UnsupportedVersion {
            format: "war3map.w3e",
            found: 13,
            supported: &[11, 12],
        };
        let s = e.to_string();
        assert!(s.contains("13"), "{s}");
        assert!(s.contains("11"), "{s}");
    }

    #[test]
    fn error_wraps_io_error_directly() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let e: Error = io.into();
        assert!(e.to_string().contains("nope"));
    }

    #[test]
    fn error_wraps_parse_error_directly() {
        let e: Error = ParseError::NotFound("war3map.w3i".into()).into();
        assert!(e.to_string().contains("war3map.w3i"));
    }
}

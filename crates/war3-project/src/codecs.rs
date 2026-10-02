//! The text forms a source-project member can have.
//!
//! One place that answers "does this member have a text form, and which one", so
//! adding a format does not mean editing every caller. Each format keeps its own
//! module and its own mapping; this is only the table plus the shape of a codec.

use war3_core::{Error, FourCC, Result};

use crate::{doodads_text, objects_text, units_text, w3i_text};

/// How a member's bytes become text and back.
#[derive(Debug, Clone, Copy)]
pub struct Codec {
    /// Member bytes → the text form.
    pub to_text: fn(&str, &[u8]) -> Result<String>,
    /// The text form → member bytes.
    ///
    /// It takes the member name, not just the text: the seven object members share
    /// one format whose *kind* comes from the name, so a text file that disagrees
    /// with the member it was loaded from has to be refused rather than silently
    /// reinterpreted as something else.
    pub from_text: fn(&str, &str) -> Result<Vec<u8>>,
}

/// The codec for a member, if it has a text form yet.
#[must_use]
pub fn codec_for(member: &str) -> Option<Codec> {
    doodads_text::codec_for(member)
        .or_else(|| units_text::codec_for(member))
        .or_else(|| objects_text::codec_for(member))
        .or_else(|| w3i_text::codec_for(member))
}

/// A four-character id as text: its characters when printable, else `0x…`.
///
/// Shared by every text form, because the two spellings have to be decided the
/// same way everywhere or an id that one format writes literally would be read
/// back differently by another.
pub(crate) fn fourcc_text(id: FourCC) -> String {
    match id.as_str() {
        Some(text) => text,
        None => format!("0x{}", hex(&id.to_bytes())),
    }
}

/// A four-character id from [`fourcc_text`].
pub(crate) fn parse_fourcc(text: &str) -> Result<FourCC> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x") {
        let bytes: [u8; 4] = parse_hex(hex)?.try_into().map_err(|v: Vec<u8>| {
            Error::msg(format!("{text:?} is {} bytes, not an id", v.len()))
        })?;
        return Ok(FourCC::new(bytes));
    }
    let bytes = text.as_bytes();
    if bytes.len() != 4 {
        return Err(Error::msg(format!(
            "{text:?} is not a four-character id; use 0x… for a non-printable one"
        )));
    }
    Ok(FourCC::new([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Bytes as continuous upper-case hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// Bytes from [`hex`].
pub(crate) fn parse_hex(text: &str) -> Result<Vec<u8>> {
    if text.len() % 2 != 0 {
        return Err(Error::msg(format!(
            "{text:?} has an odd number of hex digits"
        )));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16)
                .map_err(|_| Error::msg(format!("{text:?} is not hex")))
        })
        .collect()
}

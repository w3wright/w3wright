//! Where a member would end up if the map were extracted.
//!
//! # Why this is the core's answer and not the interface's
//!
//! "Would this member become text, be kept as binary content, or be kept as its
//! stored block?" looks like a presentation question and is not. The answer decides
//! what `extract` writes, what `build` reads back, and whether a round trip is
//! lossless — and the rule has a strict part: a member is only textified when its
//! text form **reproduces the original bytes exactly**, verified by parsing it back.
//! A member whose serialiser is subtly wrong is kept binary instead.
//!
//! That rule lives here, and [`crate::extract`] asks this module for the answer
//! rather than deciding for itself. An interface that showed a member as
//! "textified" while `extract` kept it binary would be worse than showing nothing,
//! so there is one implementation and both callers share it.

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};
use war3_core::Result;

use crate::codecs;

/// What `extract` would do with one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// Written as text, because a text form was produced **and** proved to
    /// reproduce the member's bytes.
    ///
    /// The text is carried because producing it is the expensive half of the
    /// question; a caller that wants it should not have to ask twice.
    Textified {
        /// The text form, as it would be written.
        text: String,
    },
    /// Written verbatim as content, because it has no text form that round trips.
    ///
    /// `reason` is the diagnostic that was recorded, so a caller can say *why*
    /// rather than only *what*.
    KeptBinary {
        /// Why the text form was rejected.
        reason: String,
    },
    /// Written as its stored block, because this workspace cannot decode it at all.
    ///
    /// This is not a failure of the member: a PKWare-imploded `.wts` or an
    /// unsupported compression mask lands here, and the block bytes are preserved so
    /// that `build` can put them back unchanged.
    KeptRaw,
    /// The member does not exist in the archive.
    Absent,
}

/// Answers [`Disposition`] for one member of an archive.
///
/// # Errors
///
/// Only when the member exists but its stored block cannot be obtained — which for a
/// named member means the container is inconsistent rather than that the member is
/// unusual. A member this workspace cannot *decode* is [`Disposition::KeptRaw`], not
/// an error.
pub fn of(
    archive: &war3_archive::Archive,
    name: &str,
    diagnostics: &mut Diagnostics,
) -> Result<Disposition> {
    // Existence is asked of the name index rather than by attempting a read: for a
    // member this workspace cannot decode, a failed read is the *expected* outcome,
    // and it must not be reported as an error or as absence.
    if archive.block_index_for_name(name).is_none() {
        return Ok(Disposition::Absent);
    }

    let Ok(content) = archive.read_file(name) else {
        // Undecodable by this workspace. The block is what gets stored, which is a
        // legitimate state, so no diagnostic is recorded here — `extract` records the
        // warning when it writes the block, where it has the block's size and flags
        // to report.
        return Ok(Disposition::KeptRaw);
    };

    Ok(match text_form(name, &content, diagnostics)? {
        Some(text) => Disposition::Textified { text },
        None => {
            // `text_form` pushed the specific reason already; the caller-facing
            // reason is the last one recorded, which is this member's.
            let reason = diagnostics
                .items()
                .last()
                .map(|d: &Diagnostic| d.message.clone())
                .unwrap_or_else(|| format!("{name}: no text form"));
            Disposition::KeptBinary { reason }
        }
    })
}

/// The member's text form, but only when it is **proved** to reproduce the file.
///
/// This is where "textify or keep verbatim" stops being a promise and becomes a
/// check: the text is parsed back and compared with the original bytes, and a member
/// that does not come back identical is stored as binary instead. A serialiser that
/// is right about 188 maps out of 190 is useful; one that is quietly wrong about 2
/// is not.
///
/// # Errors
///
/// When the text form cannot be produced at all — an internal parser failure, not an
/// unusual member. A member whose serialiser merely disagrees is `Ok(None)` plus a
/// diagnostic.
pub fn text_form(
    name: &str,
    content: &[u8],
    diagnostics: &mut Diagnostics,
) -> Result<Option<String>> {
    let Some(codec) = codecs::codec_for_member(name, content) else {
        return Ok(None);
    };
    let text = match (codec.to_text)(name, content) {
        Ok(text) => text,
        Err(e) => {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::ProjectMemberKeptBinary,
                format!("{name}: kept as binary, its text form could not be produced: {e}"),
            ));
            return Ok(None);
        }
    };
    match (codec.from_text)(name, &text) {
        Ok(back) if back == content => Ok(Some(text)),
        Ok(back) => {
            let at = back.iter().zip(content).position(|(a, b)| a != b);
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::ProjectMemberKeptBinary,
                format!(
                    "{name}: kept as binary, the text form gives {} bytes against {} and first \
                     differs at {at:?}",
                    back.len(),
                    content.len()
                ),
            ));
            Ok(None)
        }
        Err(e) => {
            diagnostics.push(Diagnostic::warn(
                DiagnosticCode::ProjectMemberKeptBinary,
                format!("{name}: kept as binary, the text form does not parse back: {e}"),
            ));
            Ok(None)
        }
    }
}

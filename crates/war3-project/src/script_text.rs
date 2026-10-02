//! Script members: the one text form that is the identity.
//!
//! `war3map.j` / `war3map.lua` are script source, so their "text form" is the
//! bytes themselves — the member *is* text. There is nothing to decode, nothing
//! to re-encode, and therefore nothing that can drift. The identity is what makes
//! this codec safe: `from_text(to_text(x)) == x` holds by construction, so the
//! proof in [`crate::project`] always passes when the bytes round-trip as UTF-8.
//!
//! # Why this exists at all
//!
//! The script member used to land in `[binary]` — correct, but it meant a map's
//! logic was an opaque file in the source project, which is the opposite of what
//! the source project is for. `docs/02 §8` has always listed scripts as text;
//! this is the module that makes that true.
//!
//! # The UTF-8 gate is deliberate, not incidental
//!
//! The build path reads a `[text]` member back as a `String` (`project::build`
//! calls `String::from_utf8`), so a script that is not UTF-8 **cannot** be
//! promised to round-trip through the text table. Rather than letting such a map
//! fail later during `build`, this codec refuses it here: [`codec_for`] produces
//! no text form, the member stays in `[binary]`, and `extract` reports why. That
//! is the same "textify or keep verbatim, never silently" rule as every other
//! member (ADR-0021), applied to the one member whose text form is not a parse.
//!
//! Measured on this machine's corpus (190 maps, 188 readable): 16 scripts contain
//! non-ASCII bytes and are not all UTF-8, so this gate is load-bearing. The rest
//! become text and their logic is finally diffable.

use war3_core::{Error, Result};

use crate::layout::member_dir;

/// The text form of a script member, if it has one.
///
/// Returns a codec only for the members that *are* source text, and only when
/// `content` can be represented as UTF-8. The returned text is the content
/// unchanged — see the module docs for why nothing is normalised, not even line
/// endings.
#[must_use]
pub fn codec_for_script(name: &str, content: &[u8]) -> Option<crate::codecs::Codec> {
    if !is_script_member(name) {
        return None;
    }
    // The gate: the build path can only carry text that is UTF-8.
    std::str::from_utf8(content).ok()?;
    Some(crate::codecs::Codec {
        to_text: identity_text,
        from_text: identity_bytes,
    })
}

/// Whether this member is script source.
///
/// Matches on the *resolved directory* rather than on a list of names, so the two
/// spellings the game uses (`war3map.j` at the archive root, and
/// `scripts\war3map.j`) are both covered by the rule that already puts them in the
/// same place — see [`member_dir`]. Adding a third spelling to `layout` therefore
/// cannot leave a script behind in `[binary]`.
///
/// The extension test on top of the directory is what keeps this from claiming a
/// member it does not understand: a map could hold `scripts\something.wtg`, and
/// "it is under `scripts/`" alone would make an identity codec treat trigger data
/// as source text. Script source is `.j` or `.lua`; nothing else is claimed.
#[must_use]
pub fn is_script_member(name: &str) -> bool {
    member_dir(name) == "scripts" && has_script_extension(name)
}

/// `.j` or `.lua`, case-insensitively, on the member's final segment.
fn has_script_extension(name: &str) -> bool {
    let last = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let Some((_, ext)) = last.rsplit_once('.') else {
        return false;
    };
    ext.eq_ignore_ascii_case("j") || ext.eq_ignore_ascii_case("lua")
}

/// The content as text, unchanged.
fn identity_text(_name: &str, content: &[u8]) -> Result<String> {
    String::from_utf8(content.to_vec())
        .map_err(|e| Error::msg(format!("script is not UTF-8, so it has no text form: {e}")))
}

/// The text as content, unchanged.
fn identity_bytes(_name: &str, text: &str) -> Result<Vec<u8>> {
    Ok(text.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::codec_for;
    #[test]
    fn a_valid_utf8_script_gets_an_identity_codec() {
        let src = b"function main takes nothing returns nothing\nendfunction\n";
        let codec = codec_for_script("war3map.j", src).expect("a JASS script is text");
        let text = (codec.to_text)("war3map.j", src).unwrap();
        assert_eq!(text.as_bytes(), src, "the text form is the bytes");
        assert_eq!((codec.from_text)("war3map.j", &text).unwrap(), src);
    }

    /// The single property the whole design rests on.
    #[test]
    fn the_text_form_reproduces_the_script_exactly() {
        // CRLF, tabs, a block comment, an escape, a `$` name and non-ASCII text:
        // everything the corpus is known to contain, in one input.
        let src = b"// \xe4\xbd\x9c\xe8\x80\x85\r\n\tcall SetMapName(\"\xe4\xb8\xad\xe6\x96\x87\")\r\n\
                    /* \\\\ */\r\nset $A = 0x1F\r\n";
        let codec = codec_for_script("war3map.j", src).expect("valid UTF-8 script");
        let text = (codec.to_text)("war3map.j", src).unwrap();
        assert_eq!(
            (codec.from_text)("war3map.j", &text).unwrap(),
            src,
            "nothing may be normalised: not CRLF, not tabs, not the BOM-less start"
        );
    }

    /// A UTF-8 BOM is content, not metadata.
    #[test]
    fn a_leading_bom_is_preserved() {
        let src = b"\xEF\xBB\xBFfunction main takes nothing returns nothing\nendfunction\n";
        let codec = codec_for_script("war3map.j", src).expect("a BOM is valid UTF-8");
        let text = (codec.to_text)("war3map.j", src).unwrap();
        assert!(text.starts_with('\u{feff}'), "the BOM survives as a character");
        assert_eq!((codec.from_text)("war3map.j", &text).unwrap(), src);
    }

    /// The gate that keeps a build from failing later.
    #[test]
    fn a_non_utf8_script_has_no_text_form() {
        // 0xFF is not valid UTF-8; such a script must stay binary rather than
        // being promised a text form the build path cannot carry.
        assert!(codec_for_script("war3map.j", b"call Foo()\xFF\n").is_none());
    }

    #[test]
    fn both_spellings_of_the_script_member_are_recognised() {
        assert!(is_script_member("war3map.j"));
        assert!(is_script_member("WAR3MAP.J"));
        assert!(is_script_member("scripts\\war3map.j"));
        assert!(is_script_member("scripts/war3map.lua"));
        assert!(is_script_member("war3map.lua"));
    }

    #[test]
    fn other_members_are_left_alone() {
        assert!(!is_script_member("war3map.w3i"));
        assert!(!is_script_member("war3map.wts"));
        assert!(!is_script_member("assets/foo.blp"));
        assert!(codec_for_script("war3map.w3i", b"x").is_none());
    }

    /// A member that merely *lives* under `scripts/` is not necessarily source.
    ///
    /// This is why the directory test is not enough on its own: an identity codec
    /// that claimed `scripts\anything` would be promising to carry trigger data or
    /// an imported asset as if it were source text.
    #[test]
    fn a_non_script_member_under_scripts_is_not_claimed() {
        for name in [
            "scripts\\war3map.wtg",
            "war3map.wct",
            "scripts\\blizzard.blp",
            "scripts\\noextension",
        ] {
            assert!(
                !is_script_member(name),
                "{name} is not script source and must not be claimed"
            );
        }
        // The two extensions that are source, in either case.
        assert!(is_script_member("scripts\\war3map.LUA"));
        assert!(is_script_member("WAR3MAP.J"));
    }

    /// The codec is registered, so `extract` actually reaches it.
    #[test]
    fn the_registry_finds_the_script_codec() {
        let src = b"function main takes nothing returns nothing\nendfunction\n";
        assert!(
            crate::codecs::codec_for_member("war3map.j", src).is_some(),
            "war3map.j must resolve to the script codec"
        );
        assert!(
            codec_for("war3map.w3i").is_some(),
            "and this must not have displaced the w3i codec"
        );
    }
}

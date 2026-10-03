//! Text markup inside string fields: the `|c` colour codes and friends.
//!
//! # What the codes are
//!
//! Warcraft III draws map names, descriptions, tooltips and unit names with a small
//! in-band markup. It appears in every string field, because it is the game's text and
//! not any one file's:
//!
//! ```text
//! |cAARRGGBB   set the colour of what follows
//! |r          reset to the default colour
//! |n          line break
//! ||          a literal `|`
//! ```
//!
//! The four above are the documented set. **Only the first two occur in this corpus**:
//! reading the resolved name, author and description of 168 local maps turns up 18 `|c`
//! and 16 `|r`, and nothing else. That is why `|n` and `||` are handled but not tested
//! against a real file — they are here so a map that uses them is not mangled, not
//! because they were seen.
//!
//! # Two traps the corpus set
//!
//! - **The colour byte order is `AARRGGBB`, not `RRGGBBAA`.** `|cffffff00` is the
//!   commonest form in the wild and reads as alpha `ff`, white. Read as `RRGGBBAA` it
//!   would be a colour of `ffffff` with alpha `00` — by luck the same colour, which is
//!   why the next point matters.
//! - **The alpha byte is `00` on most real text.** `|c00ff0303` (the Sentinel's red) and
//!   `|c0020c000` (the Scourge's green) both have zero alpha, which would be fully
//!   transparent. The game does not use the byte to fade text, so **a renderer must not
//!   either**: honouring it would make the two commonest faction colours invisible. This
//!   module reports the byte as stored and leaves that decision to the caller.
//!
//! # What is deliberately not here
//!
//! No "close the colour at the end of the string" repair, and no guessing at an
//! unclosed code. A `|c` without its eight hex digits is left as the literal text it
//! appears to be, because inventing a colour is how a field ends up shortened or
//! recoloured by accident.

/// One run of decoded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Markup {
    /// Text with no colour of its own.
    Plain(String),
    /// Text carrying a colour code: alpha, red, green, blue, in that order.
    ///
    /// The alpha is **as stored**, which is `0` for most real text — see the module
    /// documentation before using it.
    Coloured {
        /// Alpha byte, `0` in every sample here.
        alpha: u8,
        /// Red byte.
        red: u8,
        /// Green byte.
        green: u8,
        /// Blue byte.
        blue: u8,
        /// The text the colour applies to.
        text: String,
    },
}

impl Markup {
    /// The text, without being told what colour it was.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Plain(text) | Self::Coloured { text, .. } => text,
        }
    }

    /// The colour as `(red, green, blue)`, or `None` for text with no colour of its own.
    ///
    /// ⚠️ **The alpha byte is deliberately not returned.** It is `0` on most real text and the
    /// game does not use it to fade anything, so a renderer that honoured it would draw the
    /// Sentinel's `|c00ff0303` and the Scourge's `|c0020c000` as nothing at all. Leaving it out
    /// of the API is what stops that being someone's reasonable mistake; a caller that wants the
    /// byte can match on the variant.
    #[must_use]
    pub const fn colour(&self) -> Option<(u8, u8, u8)> {
        match self {
            Self::Plain(_) => None,
            Self::Coloured {
                red, green, blue, ..
            } => Some((*red, *green, *blue)),
        }
    }
}

/// Decodes a string field into the runs it is made of.
///
/// A string with no markup comes back as a single [`Markup::Plain`], so a caller can use
/// this unconditionally.
#[must_use]
pub fn spans(s: &str) -> Vec<Markup> {
    let bytes = s.as_bytes();
    let mut out: Vec<Markup> = Vec::new();
    let mut plain = String::new();
    let mut at = 0;

    while at < bytes.len() {
        if bytes[at] != b'|' {
            // Everything else is taken verbatim: the encoding is not re-derived, because
            // a length-changing repair here would be invisible until it corrupted a name.
            let ch = s[at..].chars().next().expect("at is a char boundary");
            plain.push(ch);
            at += ch.len_utf8();
            continue;
        }

        match bytes.get(at + 1).copied() {
            // `||` is the escape for the marker itself.
            Some(b'|') => {
                plain.push('|');
                at += 2;
            }
            // `|r` resets to the default colour, which is the same as having none.
            Some(b'r') => {
                push(&mut out, &mut plain);
                at += 2;
            }
            Some(b'n') => {
                plain.push('\n');
                at += 2;
            }
            // ⚠️ `|C` as well as `|c`. The World Editor writes the lower-case form and some map
            // makers write the upper-case one — `羊羊快跑4.34|CFF1FBF00最终正式版` is a real name
            // — so a reader that only takes `|c` leaves eight hex digits in the middle of a
            // title. The same tolerance is **not** applied to `|R` or `|N`: those are rare
            // enough that a stray `|R` is more likely to be text, and eating it would delete a
            // character the author typed.
            Some(b'c' | b'C') => match colour(&bytes[at + 2..]) {
                // `AARRGGBB`.
                Some((a, r, g, b)) => {
                    push(&mut out, &mut plain);
                    let text_start = at + 10;
                    let (text, next) = run(s, text_start);
                    out.push(Markup::Coloured {
                        alpha: a,
                        red: r,
                        green: g,
                        blue: b,
                        text,
                    });
                    at = next;
                }
                // Not eight hex digits: this is a literal `|c`.
                None => {
                    plain.push('|');
                    at += 1;
                }
            },
            // An unrecognised code is kept exactly as written. Dropping it would edit a
            // user's text on the strength of a guess about a code this build has not seen.
            _ => {
                plain.push('|');
                at += 1;
            }
        }
    }

    push(&mut out, &mut plain);
    out
}

/// The string with every code removed.
///
/// This is the rendering for a plain-text context — a table cell, a window title, a
/// box that has no way to show a colour. It is lossy **on purpose**: a caller that
/// needs the colours asks for [`spans`] instead, so nothing here has to guess what a
/// caller wanted.
#[must_use]
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for span in spans(s) {
        out.push_str(span.text());
    }
    // Carriage returns come from the map format's `\r\n` and mean nothing to a renderer
    // that treats the text as lines. Dropping them here keeps the one place that knows
    // about the markup also the one place that knows about its line endings.
    out.replace("\r\n", "\n")
}

/// Moves the accumulated plain text into the output, if there is any.
fn push(out: &mut Vec<Markup>, plain: &mut String) {
    if !plain.is_empty() {
        out.push(Markup::Plain(std::mem::take(plain)));
    }
}

/// Reads eight hex digits as `AARRGGBB`.
fn colour(rest: &[u8]) -> Option<(u8, u8, u8, u8)> {
    if rest.len() < 8 {
        return None;
    }
    // Every one of the eight must be a hex digit, not just the first four: `|cffff` followed
    // by a word is a literal, and reading it as a colour with a truncated value would eat
    // eight characters of the user's text.
    let mut digits = [0u8; 8];
    for (i, slot) in digits.iter_mut().enumerate() {
        *slot = hex(rest[i])?;
    }
    let byte = |hi: u8, lo: u8| (hi << 4) | lo;
    Some((
        byte(digits[0], digits[1]),
        byte(digits[2], digits[3]),
        byte(digits[4], digits[5]),
        byte(digits[6], digits[7]),
    ))
}

/// The value a hex digit stands for, or `None` when the byte is not one.
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The text of one coloured run, and where the next code starts.
///
/// A run ends at the next `|c`, `|r` or `|n`, or at the end of the string. Stopping at *every*
/// `|` would split on a literal one; stopping at none would let a colour swallow the reset that
/// follows it.
///
/// `||` is consumed here rather than left for the caller: it is an escape for a literal pipe,
/// so breaking the run at it would turn one coloured phrase into two spans of the same colour,
/// which a colour-aware renderer would then have to rejoin.
fn run(s: &str, from: usize) -> (String, usize) {
    let bytes = s.as_bytes();
    let mut text = String::new();
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] == b'|' {
            match bytes.get(at + 1) {
                Some(b'|') => {
                    text.push('|');
                    at += 2;
                    continue;
                }
                // The same set the main loop treats as codes, `|C` included: a run that did
                // not stop there would swallow the next colour into this one's text.
                Some(b'c' | b'C' | b'r' | b'n') => break,
                _ => {}
            }
        }
        let ch = s[at..].chars().next().expect("at is a char boundary");
        text.push(ch);
        at += ch.len_utf8();
    }
    (text, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two forms that actually occur, both taken from `imba.dota.cc`'s map list.
    #[test]
    fn a_colour_code_is_read_as_aarrggbb() {
        // `|cffffff00` is alpha ff, white — the commonest form in the corpus.
        assert_eq!(
            spans("|cffffff00IMBA 3.83f AI|r"),
            vec![Markup::Coloured {
                alpha: 0xFF,
                red: 0xFF,
                green: 0xFF,
                blue: 0x00,
                text: "IMBA 3.83f AI".to_string(),
            }]
        );
        assert_eq!(plain("|cffffff00IMBA 3.83f AI|r"), "IMBA 3.83f AI");
    }

    /// The hex digits are case-insensitive, and **the reset is optional**.
    ///
    /// `羊羊快跑4.34|CFF1FBF00最终正式版` is a real map name: title-case `|C`, upper-case hex and
    /// no `|r` at all — the colour runs to the end of the string. Reading only lower case would
    /// leave eight hex digits inside a map's name, and requiring the reset would make the two
    /// halves of this title the same colour.
    #[test]
    fn an_unclosed_colour_with_upper_case_hex_applies_to_the_end() {
        let name = "羊羊快跑4.34|CFF1FBF00最终正式版";
        assert_eq!(
            spans(name),
            vec![
                Markup::Plain("羊羊快跑4.34".to_string()),
                Markup::Coloured {
                    alpha: 0xFF,
                    red: 0x1F,
                    green: 0xBF,
                    blue: 0x00,
                    text: "最终正式版".to_string(),
                },
            ]
        );
        // And the colour is reachable without the alpha, which the renderer must not use.
        assert_eq!(spans(name)[1].colour(), Some((0x1F, 0xBF, 0x00)));
        assert_eq!(plain(name), "羊羊快跑4.34最终正式版");
    }

    /// The trap: a real code whose alpha is zero, which must not be read as transparent.
    #[test]
    fn a_zero_alpha_colour_keeps_its_rgb() {
        // The Sentinel's red, as it appears in DotA's force names.
        assert_eq!(
            spans("|c00ff0303近卫军团|r"),
            vec![Markup::Coloured {
                alpha: 0x00,
                red: 0xFF,
                green: 0x03,
                blue: 0x03,
                text: "近卫军团".to_string(),
            }]
        );
        // Reported as stored, not silently normalised to ff: the reader decides how to
        // draw it, and the module documentation says why it must not honour the byte.
        let span = &spans("|c0020c000天灾军团|r")[0];
        assert!(
            matches!(span, Markup::Coloured { alpha: 0, .. }),
            "{span:?}"
        );
    }

    /// Several colours in one string, with plain text between them.
    #[test]
    fn runs_are_split_and_the_plain_text_between_them_is_kept() {
        let text = "before |cffff0000red|r middle |cff00ff00green|r after";
        let spans = spans(text);
        assert_eq!(spans.len(), 5, "{spans:?}");
        assert_eq!(spans[0], Markup::Plain("before ".to_string()));
        assert_eq!(spans[1].text(), "red");
        assert_eq!(spans[2], Markup::Plain(" middle ".to_string()));
        assert_eq!(spans[3].text(), "green");
        assert_eq!(spans[4], Markup::Plain(" after".to_string()));
        assert_eq!(plain(text), "before red middle green after");
    }

    /// A string with no markup is one plain run, so a caller can use this unconditionally.
    #[test]
    fn a_plain_string_comes_back_unchanged() {
        assert_eq!(spans("Mimya"), vec![Markup::Plain("Mimya".to_string())]);
        assert_eq!(plain("Mimya"), "Mimya");
    }

    /// An unclosed colour is still read, because the code is complete even though the
    /// reset is missing. Repairing the *text* is what this module refuses to do.
    #[test]
    fn a_colour_with_no_reset_ends_at_the_end_of_the_string() {
        assert_eq!(
            spans("|cffffff00imba.dota.cc"),
            vec![Markup::Coloured {
                alpha: 0xFF,
                red: 0xFF,
                green: 0xFF,
                blue: 0x00,
                text: "imba.dota.cc".to_string(),
            }]
        );
    }

    /// A `|c` that is not followed by eight hex digits is text, not a truncated colour.
    #[test]
    fn an_incomplete_colour_is_left_as_text() {
        // Too short.
        assert_eq!(spans("|cfff"), vec![Markup::Plain("|cfff".to_string())]);
        // Long enough, but the low half is not hex.
        assert_eq!(
            spans("|cffffzz00not a colour"),
            vec![Markup::Plain("|cffffzz00not a colour".to_string())]
        );
        // The digits are checked in both halves: the first four being valid is not enough.
        assert_eq!(
            spans("|cffff gg00"),
            vec![Markup::Plain("|cffff gg00".to_string())]
        );
    }

    /// An unknown code is preserved verbatim. Dropping it would be editing the user's text
    /// on the strength of a guess.
    #[test]
    fn an_unknown_code_is_kept_as_written() {
        assert_eq!(spans("a|zb"), vec![Markup::Plain("a|zb".to_string())]);
        // A trailing `|` with nothing after it is just a pipe.
        assert_eq!(spans("a|"), vec![Markup::Plain("a|".to_string())]);
    }

    /// The two codes no map in the corpus used, which exist so that a map that does use
    /// them is not mangled.
    #[test]
    fn the_escapes_no_sample_used_are_handled() {
        assert_eq!(plain("line|nline"), "line\nline");
        assert_eq!(plain("a||b"), "a|b");
        // An escaped pipe is not the start of a code.
        assert_eq!(spans("a||b"), vec![Markup::Plain("a|b".to_string())]);
        // A literal pipe inside a coloured run stays literal.
        assert_eq!(
            spans("|cffff0000a||b|r"),
            vec![Markup::Coloured {
                alpha: 0xFF,
                red: 0xFF,
                green: 0x00,
                blue: 0x00,
                text: "a|b".to_string(),
            }]
        );
    }

    /// The map format writes `\r\n`, which a renderer showing the text as lines does not
    /// want to see doubled.
    #[test]
    fn carriage_returns_are_normalised() {
        assert_eq!(plain("a\r\nb"), "a\nb");
        assert_eq!(plain("a\r\n|cffffff00b|r"), "a\nb");
        // A lone `\r` is not a line ending this format writes, so it is left alone.
        assert_eq!(plain("a\rb"), "a\rb");
    }

    /// Byte indices and character indices are not the same thing, and the corpus is full of
    /// text where they differ.
    #[test]
    fn multi_byte_text_survives_the_walk() {
        assert_eq!(
            plain("|c00ff0303近卫军团|r — 天灾军团"),
            "近卫军团 — 天灾军团"
        );
        assert_eq!(spans("天|r"), vec![Markup::Plain("天".to_string())]);
    }
}

//! The two text tables object names come out of.
//!
//! Both are `[section]` then `key=value`, and they are still two parsers because they answer
//! different questions and one of them has a rule the other does not:
//!
//! | File | Sections are | Repeated key means |
//! | --- | --- | --- |
//! | `Units\*Strings.txt` | one object, by ID — `[hfoo]` | nothing; each block is one object, read whole |
//! | `UI\WorldEditStrings.txt`, `UI\WorldEditGameStrings.txt` | one flat namespace | **the first one wins** |
//!
//! # The game splits the `WESTRING_*` text across two files, and both are needed
//!
//! Measured on this installation:
//!
//! | File | Keys | Holds |
//! | --- | --- | --- |
//! | `UI\WorldEditStrings.txt` | 7,607 | most text, including `WESTRING_DOOD_APMS` |
//! | `UI\WorldEditGameStrings.txt` | 319 | `WESTRING_DEST_*`, e.g. `…ASHENVALE_TREE_WALL` |
//!
//! and the two key sets **do not overlap** — 0 keys appear in both — so reading both cannot produce
//! two answers. Reading only the first is what was tried first, and the symptom was exact: doodads
//! resolved and destructables did not, with nothing anywhere reporting an error.
//!
//! ⚠️ **A conflict is possible in repacked data.** YDWE's own copy defines
//! `WESTRING_DEST_ASHENVALE_TREE_WALL` in *both* files with different text ("Ashenvale 树木" against
//! "Ashenvale 树墙"). The game's files do not, but a repack could, and the rule for that case is the
//! one the editor appears to use: the **first** definition wins. Taking the last is what
//! `HashMap::insert` in file order does, and it is a different answer with nothing to show it went
//! wrong — which is why [`WorldStrings::parse`] scans in order instead of reusing
//! [`parse_sections`], whose `HashMap` iteration order is arbitrary.
//!
//! # Syntax that has to be accepted
//!
//! All three of these are in the real files and none is optional:
//!
//! ```text
//! // a comment            — the game's files use `//`, not `;` or `#`
//! Name=步兵              — a bare value, the common case
//! Ubertip="…|n…"          — quoted, because the text may contain `=` or start with a space
//! ```
//!
//! A BOM is handled by [`war3_core::AssetSource::get_text`] before the text gets here.

use std::collections::HashMap;

/// One section's keys.
///
/// ⚠️ **Keys are stored lower-cased**, and a caller looking one up must lower-case it too — or use
/// [`WorldStrings`], which does. Lowering on the way in rather than on the way out is what makes
/// `Name=` and `name=` the same key, which the game's own files require: `*Strings.txt` writes
/// `Name` and other tables write `name`.
pub type Fields = HashMap<String, String>;

/// The section a `key=value` line belongs to when no `[section]` has been seen.
///
/// The real `UI\WorldEditStrings.txt` opens with `[WorldEditStrings]`, so this is not how the game's
/// file is read — but a table whose whole purpose is "these keys are defined here" should not lose
/// a key because a header is missing or spelled differently, and the alternative is silently
/// dropping data.
const IMPLICIT_SECTION: &str = "";

/// The sections of a `[section]` / `key=value` text file.
///
/// The outer map is `HashMap` because callers look sections up by name and never iterate; the
/// *order inside a section* is not kept because nothing here reads a block as a sequence. Order
/// across a whole file **is** kept where it matters, by [`WorldStrings`], which is the one caller
/// that cares.
#[must_use]
pub fn parse_sections(text: &str) -> HashMap<String, Fields> {
    let mut out: HashMap<String, Fields> = HashMap::new();
    let mut current = IMPLICIT_SECTION.to_string();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with(';')
            || line.starts_with('#')
        {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            current = name.trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        out.entry(current.clone())
            .or_default()
            .insert(key.trim().to_lowercase(), unquote(value.trim()));
    }

    out
}

/// `UI\WorldEditStrings.txt`: every `WESTRING_*` key to its text, first definition winning.
#[derive(Debug, Default, Clone)]
pub struct WorldStrings {
    /// Lower-cased key to text.
    values: HashMap<String, String>,
}

impl WorldStrings {
    /// Parses `UI\WorldEditStrings.txt`.
    ///
    /// ⚠️ **Scans the file in order rather than reusing [`parse_sections`].** That function returns a
    /// `HashMap` of sections, and iterating one gives a *random* order — so "the first definition of
    /// a key wins" would depend on the hash seed and could differ between two runs of the same
    /// program. It was written that way first and a test caught it. This is the one caller where
    /// order across the whole file is the answer, so it reads the file itself.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut values: HashMap<String, String> = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with("//")
                || line.starts_with(';')
                || line.starts_with('#')
                || line.starts_with('[')
            {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            // ⚠️ `entry` and not `insert`: the **first** definition wins. See the module
            // documentation — a key defined twice with different text must resolve to the first.
            values
                .entry(key.trim().to_lowercase())
                .or_insert_with(|| unquote(value.trim()));
        }
        Self { values }
    }

    /// Adds another table's keys, without overwriting any already known.
    ///
    /// "First wins" is what makes this safe to call in a fixed order: the caller does not have to
    /// decide which file is authoritative, because whichever it passes first is, and later ones only
    /// fill gaps.
    ///
    /// The keys the game splits across its two string files do **not** overlap on this installation
    /// — 7,607 in one and 319 in the other, none in both — so in practice this only ever adds. It is
    /// written to prefer the first anyway, because "in practice" is not a property to rely on.
    pub fn merge(&mut self, other: &Self) {
        for (key, value) in &other.values {
            self.values
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }

    /// The text for a key.
    ///
    /// Returns `None` rather than the key itself when it is unknown: a caller that gets `None` can
    /// choose to show the ID, while a caller handed `WESTRING_DOOD_APMS` has been given something
    /// that looks like a name and is not.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(&key.to_lowercase()).map(String::as_str)
    }

    /// How many keys are known.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no keys are known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Removes one layer of surrounding double quotes.
///
/// The game's files quote a value when it would otherwise be ambiguous — `Tip="…=|cffffcc00B|r"`
/// — and leave it bare otherwise. Both are the same value to a reader.
fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the unit tables actually have, including the BOM-less first section and a
    /// `Hotkey` line that must not be mistaken for something.
    #[test]
    fn an_object_block_yields_its_name() {
        let text = "[hfoo]\nName=步兵\nHotkey=F\n\n[hpea]\nName=农民\n";
        let sections = parse_sections(text);

        assert_eq!(sections["hfoo"]["name"], "步兵");
        assert_eq!(sections["hfoo"]["hotkey"], "F");
        assert_eq!(sections["hpea"]["name"], "农民");
    }

    /// Keys are lowered on the way in, so `Name` and `name` are one key. A caller looking one up
    /// must lower-case it too — see [`Fields`].
    #[test]
    fn keys_are_lowered_on_the_way_in() {
        let sections = parse_sections("[hfoo]\nNAME=步兵\n");
        assert_eq!(sections["hfoo"]["name"], "步兵");
        assert_eq!(
            sections["hfoo"].get("NAME"),
            None,
            "the table stores lower-cased keys, and a caller must match that"
        );
    }

    /// A quoted value loses exactly one layer of quotes, and keeps inner punctuation.
    #[test]
    fn a_quoted_value_is_unquoted() {
        let sections = parse_sections("[AHhb]\nUbertip=\"Heals (|cffffcc00H|r) a unit\"\n");
        assert_eq!(sections["AHhb"]["ubertip"], "Heals (|cffffcc00H|r) a unit");
    }

    /// `//` is the comment marker the game's files use.
    #[test]
    fn slash_comments_are_skipped() {
        let sections = parse_sections("// a note\n[hfoo]\n// another\nName=步兵\n");
        assert_eq!(sections["hfoo"]["name"], "步兵");
    }

    /// A key before any `[section]` is kept, in an implicit section rather than dropped.
    ///
    /// ⚠️ Dropping it was the first implementation, and it is wrong for a table whose purpose is
    /// "this key is defined here": a missing or differently-spelled header would silently lose
    /// every name under it, and the symptom would be an object that "has no name" in a file that
    /// spells it out.
    #[test]
    fn a_key_outside_a_section_is_kept() {
        let sections = parse_sections("stray=1\n[hfoo]\nName=步兵\n");
        assert_eq!(sections[""]["stray"], "1");
        assert_eq!(sections["hfoo"]["name"], "步兵");
    }

    /// ⚠️ The rule this type exists for: the **first** definition of a key wins.
    ///
    /// Taking the last is what `HashMap::insert` in file order does, and it is a different answer
    /// with nothing to show it went wrong.
    #[test]
    fn the_first_definition_of_a_key_wins() {
        let text = "WESTRING_X=first\nWESTRING_X=second\n";
        assert_eq!(WorldStrings::parse(text).get("WESTRING_X"), Some("first"));
    }

    /// Keys are matched without case and an unknown key is `None`, never the key itself.
    #[test]
    fn lookups_are_case_insensitive_and_unknown_is_none() {
        let ws = WorldStrings::parse("WESTRING_DOOD_APMS=蘑菇\n");
        assert_eq!(ws.get("WESTRING_DOOD_APMS"), Some("蘑菇"));
        assert_eq!(ws.get("westring_dood_apms"), Some("蘑菇"));
        assert_eq!(ws.get("WESTRING_NOPE"), None);
        assert_eq!(ws.len(), 1);
    }

    /// A value containing `=` keeps everything after the first one.
    #[test]
    fn a_value_may_contain_the_separator() {
        let ws = WorldStrings::parse("WESTRING_X=a=b=c\n");
        assert_eq!(ws.get("WESTRING_X"), Some("a=b=c"));
    }
}

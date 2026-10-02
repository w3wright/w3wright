//! A minimal INI document, the text form a source-project member is written in.
//!
//! # Why the order is part of the type
//!
//! The prototype's hard metric is that `extract → build` reproduces every member
//! byte for byte, so the text form has to keep the order it was written in: a
//! `BTreeMap` would sort the keys and change the file. Sections and entries are
//! therefore `Vec`s, and a repeated section name is legal — that is how a list of
//! players or forces is expressed.
//!
//! # Syntax
//!
//! ```text
//! ; a comment ('#' works too)
//! [section]
//! key = "value"        ; strings are always quoted
//! count = 12           ; so are numbers, quoted or bare
//! ```
//!
//! Nothing outside that is accepted. A file a human edited has to fail loudly
//! rather than be half-understood — the same rule the format parsers follow.

use std::fmt::Write as _;

use war3_core::{Error, Result};

/// One `[section]`, with its entries in the order they appeared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Section name, without the brackets.
    pub name: String,
    /// `key = value` pairs, in order.
    pub entries: Vec<(String, String)>,
}

/// A whole text file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    /// Sections, in order.
    pub sections: Vec<Section>,
}

impl Section {
    /// A new, empty section.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: Vec::new(),
        }
    }

    /// Sets an entry, **replacing** any existing one with the same key.
    ///
    /// # Why replace rather than append
    ///
    /// This used to append unconditionally, with a comment saying so. For the writers
    /// that build a fresh section it made no difference, because they set each key
    /// once. It made a critical difference for anyone **editing** a parsed document:
    /// setting a key that already existed produced a section holding that key twice.
    ///
    /// That combination is a trap, because the two halves of a round trip disagreed
    /// about what it meant:
    ///
    /// - [`Self::get`] returns the **first** entry, so the edit looked ignored.
    /// - [`Document::render`] writes **both**, and [`Document::parse`] rejects a
    ///   duplicate key as an error.
    ///
    /// So "open a file, change a field, save it" produced text this crate then
    /// refuses to read back. The edit path was broken by construction, and the
    /// failure surfaced a step later than its cause.
    ///
    /// Replacing in place is what a setter is normally taken to mean, and it keeps the
    /// key's original position, so a re-rendered document differs only on the line
    /// that actually changed.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        let key = key.into();
        let value = value.into();
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => self.entries.push((key, value)),
        }
        self
    }

    /// The raw value of `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The value of `key`, or an error naming the section.
    pub fn require(&self, key: &str) -> Result<&str> {
        self.get(key).ok_or_else(|| {
            Error::msg(format!(
                "[{}] is missing `{key}`; the file is not a complete text form",
                self.name
            ))
        })
    }

    /// An integer entry.
    pub fn i32(&self, key: &str) -> Result<i32> {
        let raw = self.require(key)?;
        raw.parse()
            .map_err(|_| Error::msg(format!("[{}] {key} = {raw:?} is not an integer", self.name)))
    }

    /// An unsigned integer entry.
    pub fn u32(&self, key: &str) -> Result<u32> {
        let raw = self.require(key)?;
        raw.parse().map_err(|_| {
            Error::msg(format!(
                "[{}] {key} = {raw:?} is not an unsigned integer",
                self.name
            ))
        })
    }

    /// A float entry.
    ///
    /// Accepts both the decimal form and the exact bits [`float_text`] falls back
    /// to, because a value that came out of a file has to go back in unchanged.
    pub fn f32(&self, key: &str) -> Result<f32> {
        parse_float(self.require(key)?)
            .map_err(|e| Error::msg(format!("[{}] {key}: {e}", self.name)))
    }

    /// An optional integer entry: `None` when the key is absent, which is
    /// different from a key present but empty.
    pub fn opt_i32(&self, key: &str) -> Result<Option<i32>> {
        match self.get(key) {
            Some(_) => Ok(Some(self.i32(key)?)),
            None => Ok(None),
        }
    }

    /// An optional float entry.
    pub fn opt_f32(&self, key: &str) -> Result<Option<f32>> {
        match self.get(key) {
            Some(_) => Ok(Some(self.f32(key)?)),
            None => Ok(None),
        }
    }

    /// An optional string entry.
    #[must_use]
    pub fn opt_string(&self, key: &str) -> Option<String> {
        self.get(key).map(str::to_string)
    }

    /// Bytes written as continuous lower-case hex.
    pub fn hex(&self, key: &str) -> Result<Vec<u8>> {
        let raw = self.require(key)?;
        let digits: Vec<u8> = raw.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        if digits.len() % 2 != 0 {
            return Err(Error::msg(format!(
                "[{}] {key} has an odd number of hex digits",
                self.name
            )));
        }
        let mut out = Vec::with_capacity(digits.len() / 2);
        for pair in digits.chunks(2) {
            let text = std::str::from_utf8(pair)
                .map_err(|_| Error::msg(format!("[{}] {key} is not hex", self.name)))?;
            out.push(
                u8::from_str_radix(text, 16).map_err(|_| {
                    Error::msg(format!("[{}] {key} = {text:?} is not hex", self.name))
                })?,
            );
        }
        Ok(out)
    }
}

impl Document {
    /// A new, empty document.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a section and returns it for filling in.
    pub fn push(&mut self, name: impl Into<String>) -> &mut Section {
        self.sections.push(Section::new(name));
        // The just-pushed section cannot be missing.
        self.sections.last_mut().expect("just pushed")
    }

    /// The first section with this name.
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// The first section with this name, or an error.
    pub fn require_section(&self, name: &str) -> Result<&Section> {
        self.section(name)
            .ok_or_else(|| Error::msg(format!("the text form has no [{name}] section")))
    }

    /// Every section with this name, in order.
    pub fn sections_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Section> {
        self.sections.iter().filter(move |s| s.name == name)
    }

    /// Renders the document.
    ///
    /// Deterministic: the order is whatever the caller pushed, so writing the same
    /// model twice produces the same bytes.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            let _ = writeln!(out, "[{}]", section.name);
            for (key, value) in &section.entries {
                let _ = writeln!(out, "{key} = \"{}\"", escape(value));
            }
            out.push('\n');
        }
        out
    }

    /// Parses a document.
    ///
    /// # Errors
    ///
    /// Anything outside the subset, naming the line: a key before any section, an
    /// unquoted or unterminated value, a malformed header, a duplicate key inside
    /// one section, or an unknown escape.
    pub fn parse(text: &str) -> Result<Self> {
        let mut document = Document::new();
        let mut current: Option<usize> = None;
        let mut in_block_comment = false;

        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = raw.trim();
            if in_block_comment {
                if line.ends_with("-->") {
                    in_block_comment = false;
                }
                continue;
            }
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with("<!--") {
                in_block_comment = !line.ends_with("-->");
                continue;
            }

            if let Some(rest) = line.strip_prefix('[') {
                let name = rest
                    .strip_suffix(']')
                    .ok_or_else(|| bad(line_no, "section header has no closing `]`"))?
                    .trim();
                if name.is_empty() {
                    return Err(bad(line_no, "section header is empty"));
                }
                if name.contains(['[', ']']) {
                    return Err(bad(line_no, "section header contains a bracket"));
                }
                document.sections.push(Section::new(name));
                current = Some(document.sections.len() - 1);
                continue;
            }

            let Some(section_index) = current else {
                return Err(bad(line_no, "a key appeared before any [section]"));
            };
            let (key, raw_value) = line
                .split_once('=')
                .ok_or_else(|| bad(line_no, "expected `key = \"value\"`"))?;
            let key = key.trim();
            if key.is_empty() {
                return Err(bad(line_no, "the key is empty"));
            }
            let value = parse_string(line_no, raw_value.trim())?;
            let section = &mut document.sections[section_index];
            if section.entries.iter().any(|(k, _)| k == key) {
                return Err(bad(
                    line_no,
                    &format!("`{key}` appears twice in [{}]", section.name),
                ));
            }
            section.entries.push((key.to_string(), value));
        }

        Ok(document)
    }
}

fn bad(line_no: usize, message: &str) -> Error {
    Error::msg(format!("text form line {line_no}: {message}"))
}

/// Parses a quoted value, honouring `\"`, `\\`, `\n` and `\t`.
fn parse_string(line_no: usize, raw: &str) -> Result<String> {
    let Some(rest) = raw.strip_prefix('"') else {
        return Err(bad(
            line_no,
            &format!("`{raw}` is not a quoted value; every value is written as \"...\""),
        ));
    };
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if chars.as_str().trim().is_empty() {
                    return Ok(out);
                }
                return Err(bad(line_no, "text after the closing quote"));
            }
            '\\' => match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => return Err(bad(line_no, &format!("unknown escape `\\{other}`"))),
                None => return Err(bad(line_no, "the value ends in a backslash")),
            },
            other => out.push(other),
        }
    }
    Err(bad(line_no, "the value is never closed"))
}

/// A float as text, exact for every bit pattern.
///
/// Rust's shortest round-tripping decimal is used whenever it really round-trips.
/// `-0.0` and NaN payloads are the exceptions: the sign of zero does not survive a
/// decimal round trip on every path, and a NaN payload is not text at all. Those
/// are written as their bits. Without this, a map whose camera bounds hold a
/// `-0.0` would fail the text round-trip check and stay binary for no reason.
#[must_use]
pub fn float_text(value: f32) -> String {
    let decimal = value.to_string();
    match decimal.parse::<f32>() {
        Ok(back) if back.to_bits() == value.to_bits() => decimal,
        _ => format!("0x{:08X}", value.to_bits()),
    }
}

/// A float from [`float_text`].
pub fn parse_float(text: &str) -> Result<f32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16)
            .map(f32::from_bits)
            .map_err(|_| Error::msg(format!("{text:?} is not a float written as bits")));
    }
    text.parse::<f32>()
        .map_err(|_| Error::msg(format!("{text:?} is not a number")))
}

/// Escapes a value for [`Document::render`].
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
; w3wright map info
[map]
version = \"25\"
name = \"TRIGSTR_010\"
camera_bounds = \"-2048 -2048 2048 2048 -2048 -2048 2048 2048\"

[player]
slot = \"0\"
name = \"TRIGSTR_001\"

[player]
slot = \"1\"
name = \"TRIGSTR_002\"
";

    #[test]
    fn repeated_sections_keep_their_order() {
        let doc = Document::parse(SAMPLE).unwrap();
        let slots: Vec<i32> = doc
            .sections_named("player")
            .map(|s| s.i32("slot").unwrap())
            .collect();
        assert_eq!(slots, vec![0, 1]);
    }

    #[test]
    fn render_round_trips_through_parse() {
        let doc = Document::parse(SAMPLE).unwrap();
        assert_eq!(Document::parse(&doc.render()).unwrap(), doc);
    }

    #[test]
    fn entry_order_is_preserved_not_sorted() {
        let mut doc = Document::new();
        doc.push("map")
            .set("version", "25")
            .set("author", "someone")
            .set("name", "a map");
        let rendered = doc.render();
        let version = rendered.find("version").unwrap();
        let author = rendered.find("author").unwrap();
        let name = rendered.find("name").unwrap();
        assert!(version < author && author < name);
        assert_eq!(Document::parse(&rendered).unwrap(), doc);
    }

    #[test]
    fn numbers_and_floats_survive_the_text_round_trip() {
        // A fresh section per value: `set` appends and `get` returns the first
        // entry, so reusing one would compare every value with the first.
        for value in [0.0f32, -0.0, 1.5, -2048.0, f32::MIN, f32::MAX, 1.0e-30] {
            let mut section = Section::new("map");
            section.set("f", float_text(value));
            assert_eq!(
                section.f32("f").unwrap().to_bits(),
                value.to_bits(),
                "{value}"
            );
        }
        let mut section = Section::new("map");
        section.set("i", i32::MIN.to_string());
        assert_eq!(section.i32("i").unwrap(), i32::MIN);
        section.set("u", u32::MAX.to_string());
        assert_eq!(section.u32("u").unwrap(), u32::MAX);
    }

    #[test]
    fn hex_values_round_trip() {
        let mut section = Section::new("x");
        section.set("blob", "deadBEEF");
        assert_eq!(section.hex("blob").unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
        section.set("odd", "abc");
        assert!(section.hex("odd").is_err());
    }

    #[test]
    fn an_absent_key_is_different_from_an_empty_one() {
        let mut section = Section::new("x");
        assert_eq!(section.opt_i32("nope").unwrap(), None);
        section.set("empty", "");
        assert!(section.opt_i32("empty").is_err(), "present but unparsable");
    }

    #[test]
    fn comments_and_block_comments_are_skipped() {
        let doc = Document::parse(
            "<!-- generated\nstill comment -->\n; comment\n# comment\n[s]\nk = \"v\"\n",
        )
        .unwrap();
        assert_eq!(doc.require_section("s").unwrap().get("k"), Some("v"));
    }

    #[test]
    fn syntax_outside_the_subset_is_refused_with_a_line_number() {
        for (text, expect) in [
            ("k = \"v\"\n", "before any [section]"),
            ("[s\n", "has no closing"),
            ("[s]\nk = v\n", "not a quoted value"),
            ("[s]\nk = \"v\n", "never closed"),
            ("[s]\nk = \"v\" x\n", "after the closing quote"),
            ("[s]\nk = \"a\\qb\"\n", "unknown escape"),
            ("[s]\nk = \"1\"\nk = \"2\"\n", "appears twice"),
            ("[]\n", "is empty"),
        ] {
            let err = Document::parse(text).unwrap_err().to_string();
            assert!(err.contains(expect), "expected {expect:?} in {err:?}");
        }
    }

    #[test]
    fn a_missing_key_names_the_section() {
        let doc = Document::parse("[player]\nslot = \"0\"\n").unwrap();
        let err = doc
            .require_section("player")
            .unwrap()
            .require("name")
            .unwrap_err()
            .to_string();
        assert!(err.contains("[player]") && err.contains("name"), "{err}");
    }
}

#[cfg(test)]
mod set_semantics_tests {
    use super::*;

    /// The bug that broke the edit path: `set` on an existing key must replace it.
    ///
    /// Pinned because the failure mode is indirect. An appended duplicate is not
    /// visible through `get` (which finds the old value, so the edit looks ignored),
    /// and it only becomes an error when the rendered text is parsed again.
    #[test]
    fn setting_an_existing_key_replaces_it_in_place() {
        let mut section = Section::new("info");
        section.set("name", "before");
        section.set("other", "kept");
        section.set("name", "after");

        assert_eq!(section.get("name"), Some("after"), "the edit must be visible");
        assert_eq!(section.entries.len(), 2, "no duplicate was added");
        assert_eq!(
            section.entries[0].0, "name",
            "the key keeps its original position, so a diff stays local"
        );
    }

    /// And the whole round trip has to close: render, parse, same values.
    ///
    /// This is the assertion that would have caught the original bug at the level it
    /// matters — "edit a field and read the result back".
    #[test]
    fn an_edited_document_round_trips() {
        let mut doc = Document::new();
        {
            let s = doc.push("info");
            s.set("version", "25");
            s.set("name", "original");
        }
        doc.sections[0].set("name", "edited");

        let text = doc.render();
        let reparsed = Document::parse(&text).expect("the rendered text must be readable");
        assert_eq!(
            reparsed.sections[0].get("name"),
            Some("edited"),
            "the edit survives the round trip"
        );
        assert_eq!(text.lines().filter(|l| l.starts_with("name")).count(), 1);
    }
}
//! `UI\UnitEditorData.txt`: the vocabulary that field types are written in.
//!
//! # Why this file, and why it is generated rather than listed
//!
//! The `type` column of a `*MetaData.slk` does not hold a binary type code. It
//! holds a word — `int`, `real`, `unreal`, `bool`, `abilList`, `attackBits`,
//! `channelType` and around thirty more. Something has to turn those words into
//! the four codes that appear in a `.w3u`, and three implementations in the wild
//! do it three different ways, two of them with a hand-written list. They
//! disagree, which is the whole problem.
//!
//! `UnitEditorData.txt` is the answer: its section names **are** those words,
//! and each section lists the values the editor offers. The authoritative
//! reading is
//!
//! ```text
//! base = { int: 0, bool: 0, real: 1, unreal: 2 }
//! for every [section]:
//!     if the first field of its `00` entry is a number then 0 else 3
//! every other word: 3
//! ```
//!
//! The intuition behind the test: a numeric first value means the editor offers
//! numbers for that type, so the binary holds a number (code 0); anything else
//! is a name, so the binary holds a string (code 3).
//!
//! # This file has versions too
//!
//! Three copies exist in a 1.27 installation — 12 sections in `war3.mpq`,
//! 31 in `War3x.mpq`, 36 in `War3Patch.mpq` — and the newest one is the one the
//! game uses. `channelType` and `spellDetail` exist only in the later copies, so
//! reading the base archive silently degrades them to the string fallback. The
//! ordering lives in the asset layer, not here; see the module docs of
//! `war3-cli::assets`.

use std::collections::BTreeMap;
use std::fmt;

use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};
use war3_core::AssetSource;

use crate::field::FieldType;

/// The path inside the game's data.
pub const EDITOR_DATA_PATH: &str = "UI\\UnitEditorData.txt";

/// A parsed `UnitEditorData.txt`.
///
/// Sections map to keys map to raw values; the value of the `00` key is the
/// comma-separated list `<data>,<display key>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorData {
    sections: BTreeMap<String, BTreeMap<String, String>>,
    /// Diagnostics from parsing.
    pub diagnostics: Diagnostics,
}

impl EditorData {
    /// An empty file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses the text.
    ///
    /// `//` starts a comment, blank lines are ignored, `[name]` opens a section
    /// and `key=value` sets a value in the current section. A `key=value` line
    /// before any section is reported rather than silently dropped, since it
    /// means the file is not shaped like the ones seen.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut diagnostics = Diagnostics::new();
        let mut current: Option<String> = None;

        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                let name = name.trim().to_string();
                sections.entry(name.clone()).or_default();
                current = Some(name);
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                diagnostics.push(Diagnostic::warn(
                    DiagnosticCode::AssetFallbackUsed,
                    format!(
                        "line {} is neither a section, a comment nor `key=value`: {line:?}",
                        number + 1
                    ),
                ));
                continue;
            };
            match &current {
                Some(section) => {
                    sections
                        .entry(section.clone())
                        .or_default()
                        .insert(key.trim().to_string(), value.trim().to_string());
                }
                None => diagnostics.push(Diagnostic::warn(
                    DiagnosticCode::AssetFallbackUsed,
                    format!("line {} sets {key:?} before any section", number + 1),
                )),
            }
        }

        Self {
            sections,
            diagnostics,
        }
    }

    /// Loads it from an asset source.
    #[must_use]
    pub fn load_from_assets(source: &dyn AssetSource) -> Self {
        match source.get_text(EDITOR_DATA_PATH) {
            Some(text) => Self::parse(&text),
            None => {
                let mut data = Self::new();
                data.diagnostics.push(Diagnostic::info(
                    DiagnosticCode::AssetFallbackUsed,
                    format!(
                        "{EDITOR_DATA_PATH} is unavailable, so field type words cannot be mapped \
                         to binary type codes"
                    ),
                ));
                data
            }
        }
    }

    /// How many sections were read.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sections.len()
    }

    /// Whether nothing was read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// Every section name.
    pub fn section_names(&self) -> impl Iterator<Item = &str> {
        self.sections.keys().map(String::as_str)
    }

    /// A raw value by section and key.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections.get(section)?.get(key).map(String::as_str)
    }

    /// The first comma-separated field of a section's `00` entry.
    ///
    /// That is the value the mapping rule tests for being numeric.
    #[must_use]
    pub fn first_value(&self, section: &str) -> Option<&str> {
        self.get(section, "00")?.split(',').next().map(str::trim)
    }
}

impl fmt::Display for EditorData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EditorData({} sections)", self.sections.len())
    }
}

/// Maps the `type` vocabulary to the binary type codes.
///
/// Built by [`TypeRegistry::from_editor_data`]; see the module docs for the rule
/// and why it is not a hand-written list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRegistry {
    words: BTreeMap<String, FieldType>,
    /// How many words came from a section of `UnitEditorData.txt` rather than
    /// from the four built-in names. Reported by `war3 meta check`.
    from_sections: usize,
}

impl Default for TypeRegistry {
    fn default() -> Self {
        Self::from_editor_data(&EditorData::new())
    }
}

impl TypeRegistry {
    /// Derives the mapping from a parsed `UnitEditorData.txt`.
    #[must_use]
    pub fn from_editor_data(data: &EditorData) -> Self {
        // The four names that are not sections of the file.
        let mut words = BTreeMap::from([
            ("int".to_string(), FieldType::Integer),
            ("bool".to_string(), FieldType::Integer),
            ("real".to_string(), FieldType::Real),
            ("unreal".to_string(), FieldType::Unreal),
        ]);
        let mut from_sections = 0usize;
        for section in data.section_names() {
            let numeric = data
                .first_value(section)
                .is_some_and(|v| v.parse::<f64>().is_ok());
            words.insert(
                section.to_string(),
                if numeric {
                    FieldType::Integer
                } else {
                    FieldType::String
                },
            );
            from_sections += 1;
        }
        Self {
            words,
            from_sections,
        }
    }

    /// The code a word maps to, or `None` if the word is not in the file.
    #[must_use]
    pub fn get(&self, type_name: &str) -> Option<FieldType> {
        self.words.get(type_name).copied()
    }

    /// The code a word maps to, falling back to a string.
    ///
    /// The fallback is the one the corpus supports: of the 55 words the game's
    /// metadata uses, 28 are in no section and not one of the four built-ins,
    /// and every modifier carrying one of them stores code 3.
    #[must_use]
    pub fn expected(&self, type_name: &str) -> FieldType {
        self.get(type_name).unwrap_or(FieldType::String)
    }

    /// How many words are known, built-ins included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether nothing is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// How many words came from a section of the file.
    #[must_use]
    pub const fn from_sections(&self) -> usize {
        self.from_sections
    }

    /// Which words map to a given code.
    pub fn words_mapping_to(&self, code: FieldType) -> impl Iterator<Item = &str> {
        self.words
            .iter()
            .filter(move |(_, v)| **v == code)
            .map(|(k, _)| k.as_str())
    }

    /// Loads and derives in one step.
    #[must_use]
    pub fn load_from_assets(source: &dyn AssetSource) -> Self {
        Self::from_editor_data(&EditorData::load_from_assets(source))
    }
}

impl fmt::Display for TypeRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TypeRegistry({} words, {} of them from sections)",
            self.words.len(),
            self.from_sections
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like the real file: comments, a section whose first value is
    /// numeric, and ones whose first value is a name.
    const SAMPLE: &str = "// Each section corresponds to a field type\n\
[attackBits]\n\
NumValues=3\n\
00=0,WESTRING_UE_ATTACKBITS_NONE\n\
01=1,WESTRING_UE_ATTACKBITS_ONE\n\
\n\
[regenType]\n\
NumValues=2\n\
00=none,WESTRING_UE_REGENTYPE_NONE\n\
01=always,WESTRING_UE_REGENTYPE_ALWAYS\n\
\n\
[missileArt]\n\
00=Abilities\\Weapons\\Arrow\\ArrowMissile.mdl,WESTRING_UE_MISSILE_ARROW\n";

    #[test]
    fn parses_sections_keys_and_comments() {
        let data = EditorData::parse(SAMPLE);
        assert_eq!(data.len(), 3);
        assert_eq!(data.get("attackBits", "NumValues"), Some("3"));
        assert_eq!(data.first_value("attackBits"), Some("0"));
        assert_eq!(data.first_value("regenType"), Some("none"));
        assert!(data.diagnostics.is_empty(), "{:?}", data.diagnostics);
    }

    #[test]
    fn a_numeric_first_value_becomes_integer_and_anything_else_a_string() {
        let registry = TypeRegistry::from_editor_data(&EditorData::parse(SAMPLE));
        assert_eq!(registry.get("attackBits"), Some(FieldType::Integer));
        assert_eq!(registry.get("regenType"), Some(FieldType::String));
        // A path with a comma-free first field is still a string, not a number.
        assert_eq!(
            registry.get("missileArt"),
            Some(FieldType::String),
            "a model path must not be mistaken for a number"
        );
    }

    #[test]
    fn the_four_builtin_names_are_present_without_a_section() {
        let registry = TypeRegistry::from_editor_data(&EditorData::new());
        assert_eq!(registry.get("int"), Some(FieldType::Integer));
        assert_eq!(registry.get("bool"), Some(FieldType::Integer));
        assert_eq!(registry.get("real"), Some(FieldType::Real));
        assert_eq!(registry.get("unreal"), Some(FieldType::Unreal));
        assert_eq!(registry.from_sections(), 0);
    }

    #[test]
    fn a_section_overrides_a_builtin_name() {
        // If the game ever adds a `[real]` section the file must win.
        let data = EditorData::parse("[real]\n00=1.0,WESTRING\n");
        let registry = TypeRegistry::from_editor_data(&data);
        assert_eq!(registry.get("real"), Some(FieldType::Integer));
    }

    #[test]
    fn an_unknown_word_falls_back_to_a_string() {
        let registry = TypeRegistry::from_editor_data(&EditorData::parse(SAMPLE));
        assert_eq!(registry.get("unitList"), None);
        assert_eq!(registry.expected("unitList"), FieldType::String);
    }

    #[test]
    fn a_section_without_a_00_entry_is_a_string_not_a_panic() {
        let registry = TypeRegistry::from_editor_data(&EditorData::parse("[odd]\nSort=1\n"));
        assert_eq!(registry.get("odd"), Some(FieldType::String));
    }

    #[test]
    fn a_key_before_any_section_is_reported() {
        let data = EditorData::parse("Key=1\n[section]\n00=x\n");
        assert!(data.diagnostics.has_problems());
        assert_eq!(data.len(), 1);
    }

    #[test]
    fn loading_without_the_file_is_empty_and_says_so() {
        let data = EditorData::load_from_assets(&war3_core::EmptyAssetSource);
        assert!(data.is_empty());
        assert!(!data.diagnostics.items().is_empty());
        let registry = TypeRegistry::load_from_assets(&war3_core::EmptyAssetSource);
        // Only the four built-ins survive, so every unknown word is a string.
        assert_eq!(registry.len(), 4);
        assert_eq!(registry.from_sections(), 0);
    }

    #[test]
    fn the_registry_reports_which_words_share_a_code() {
        let registry = TypeRegistry::from_editor_data(&EditorData::parse(SAMPLE));
        let integers: Vec<&str> = registry.words_mapping_to(FieldType::Integer).collect();
        assert_eq!(integers, vec!["attackBits", "bool", "int"]);
    }
}

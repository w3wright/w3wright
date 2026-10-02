//! The source project manifest, `war3.toml`.
//!
//! # Why a hand-written subset rather than a TOML crate
//!
//! This workspace carries no external dependencies (README convention 7), partly
//! for WASM size and partly so every crate builds with a plain Rust toolchain.
//! The manifest is a file *this* code writes against a schema *this* code
//! defines, so a subset parser is enough — **provided it refuses everything
//! outside the subset instead of guessing**, which is the same rule the format
//! parsers follow (rule 4 of §3.1 in the `docs/` design set).
//!
//! Accepted: comments, one `[table]` header per line from a fixed set,
//! and `key = "value"` with quoted or bare keys and quoted string values.
//!
//! Rejected **with a line number**: array-of-tables, inline tables, numbers,
//! booleans, unquoted values, unknown tables, unknown escapes, duplicate keys and
//! repeated table headers. Every one of those would otherwise be a silent
//! misinterpretation of a hand-edited file.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use war3_core::{Error, Result};

/// Member name to project-relative path.
pub type Members = BTreeMap<String, String>;

/// A parsed `war3.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Where the `HM3W` prefix lives, relative to the project directory.
    ///
    /// The prefix sits *before* the MPQ header and is not a member, but the game
    /// reads the map name and flags out of it, so a build has to put it back.
    pub prefix: String,
    /// Where `build` writes, relative to the project directory.
    ///
    /// Absent in a hand-written manifest that intends to pass `--out` instead.
    pub output: Option<String>,
    /// Members with a text form.
    pub text: Members,
    /// Members stored as decoded content, written back as one uncompressed block.
    pub binary: Members,
    /// Members stored as their original block bytes plus flags.
    pub raw: Members,
}

/// Which table the parser is currently inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Table {
    Project,
    Build,
    Text,
    Binary,
    Raw,
}

impl Config {
    /// Parses a manifest.
    ///
    /// # Errors
    ///
    /// Any syntax the subset does not cover, naming the line.
    pub fn parse(text: &str) -> Result<Self> {
        let mut prefix: Option<String> = None;
        let mut output: Option<String> = None;
        let (mut text_t, mut binary, mut raw) = (Members::new(), Members::new(), Members::new());
        let mut table: Option<Table> = None;
        let mut seen: Vec<Table> = Vec::new();

        for (index, raw_line) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = strip_comment(raw_line);
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(rest) = line.strip_prefix('[') {
                if rest.starts_with('[') {
                    return Err(bad(
                        line_no,
                        "array-of-tables (`[[...]]`) is not part of a manifest",
                    ));
                }
                let name = rest
                    .strip_suffix(']')
                    .ok_or_else(|| bad(line_no, "table header has no closing `]`"))?
                    .trim();
                let (next, seen_as) = match name {
                    "project" => (Table::Project, "project"),
                    "build" => (Table::Build, "build"),
                    "text" => (Table::Text, "text"),
                    "binary" => (Table::Binary, "binary"),
                    "raw" => (Table::Raw, "raw"),
                    other => {
                        let hint = "project, build, text, binary or raw";
                        return Err(bad(
                            line_no,
                            &format!("unknown table [{other}]; expected {hint}"),
                        ));
                    }
                };
                if seen.contains(&next) {
                    return Err(bad(line_no, &format!("table [{seen_as}] appears twice")));
                }
                seen.push(next);
                table = Some(next);
                continue;
            }

            let Some(table) = table else {
                return Err(bad(line_no, "a key appeared before any table header"));
            };
            let (raw_key, raw_value) =
                split_assignment(line).ok_or_else(|| bad(line_no, "expected `key = \"value\"`"))?;
            let key = parse_key(line_no, raw_key)?;
            let value = parse_string(line_no, raw_value)?;

            match table {
                Table::Project => match key.as_str() {
                    "prefix" => set_once(&mut prefix, value, line_no, "project.prefix")?,
                    other => {
                        return Err(bad(
                            line_no,
                            &format!(
                                "unknown key `{other}` in [project]; only `prefix` is defined"
                            ),
                        ))
                    }
                },
                Table::Build => match key.as_str() {
                    "output" => set_once(&mut output, value, line_no, "build.output")?,
                    other => {
                        return Err(bad(
                            line_no,
                            &format!("unknown key `{other}` in [build]; only `output` is defined"),
                        ))
                    }
                },
                Table::Text => insert_member(&mut text_t, key, value, line_no, "text")?,
                Table::Binary => insert_member(&mut binary, key, value, line_no, "binary")?,
                Table::Raw => insert_member(&mut raw, key, value, line_no, "raw")?,
            }
        }

        let Some(prefix) = prefix else {
            return Err(Error::msg(
                "war3.toml: [project] prefix is missing; a build cannot restore the HM3W prefix without it",
            ));
        };

        Ok(Self {
            prefix,
            output,
            text: text_t,
            binary,
            raw,
        })
    }

    /// Renders a manifest.
    ///
    /// Deterministic: `BTreeMap` puts the members in name order, so writing the
    /// same project twice produces the same bytes.
    #[must_use]
    pub fn render(&self) -> String {
        let mut s = String::new();
        s.push_str(
            "# w3wright source project. Written by `war3 map extract`; safe to edit by hand.\n\
             #\n\
             # Every member is listed in exactly one of the three tables below:\n\
             #   [text]   a text form that can be read and written back\n\
             #   [binary] decoded content, written back as one uncompressed block\n\
             #   [raw]    the original block bytes plus flags, written back verbatim\n\
             #            (a member that cannot be decompressed has no content to store)\n\
             \n",
        );
        let _ = writeln!(s, "[project]");
        let _ = writeln!(s, "prefix = \"{}\"\n", escape(&self.prefix));
        s.push_str("[build]\n");
        if let Some(output) = &self.output {
            let _ = writeln!(s, "output = \"{}\"", escape(output));
        }
        s.push('\n');
        for (title, members) in [
            ("text", &self.text),
            ("binary", &self.binary),
            ("raw", &self.raw),
        ] {
            let _ = writeln!(s, "[{title}]");
            for (name, path) in members {
                let _ = writeln!(s, "\"{}\" = \"{}\"", escape(name), escape(path));
            }
            s.push('\n');
        }
        s
    }

    /// Total number of members the manifest accounts for.
    #[must_use]
    pub fn member_count(&self) -> usize {
        self.text.len() + self.binary.len() + self.raw.len()
    }
}

fn bad(line_no: usize, message: &str) -> Error {
    Error::msg(format!("war3.toml line {line_no}: {message}"))
}

fn set_once(slot: &mut Option<String>, value: String, line_no: usize, what: &str) -> Result<()> {
    if slot.is_some() {
        return Err(bad(line_no, &format!("{what} is set twice")));
    }
    *slot = Some(value);
    Ok(())
}

fn insert_member(
    members: &mut Members,
    name: String,
    path: String,
    line_no: usize,
    table: &str,
) -> Result<()> {
    if members.insert(name.clone(), path).is_some() {
        return Err(bad(
            line_no,
            &format!("member `{name}` appears twice in [{table}]"),
        ));
    }
    Ok(())
}

/// Cuts a line at the first `#` that is not inside a string.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (offset, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..offset],
            _ => {}
        }
    }
    line
}

/// Splits `key = value` at the first `=` that is outside a string.
///
/// Both halves come back raw: unescaping a quoted key is [`parse_key`]'s job,
/// which is also where a malformed key can be reported with its line number.
fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let mut in_string = false;
    let mut escaped = false;
    for (offset, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '=' if !in_string => return Some((line[..offset].trim(), line[offset + 1..].trim())),
            _ => {}
        }
    }
    None
}

/// Parses the key: a bare word, or a quoted string with the escapes a value has.
///
/// # Why the key is unescaped too
///
/// Member names carry separators — `war3mapImported\hero.blp` — so they are
/// written quoted and escaped. Reading the key back *without* unescaping it
/// produces a manifest that re-renders differently from the one that was parsed,
/// which is exactly the drift a round-trip test exists to catch.
fn parse_key(line_no: usize, key: &str) -> Result<String> {
    let key = key.trim();
    if key.starts_with('"') {
        return parse_string(line_no, key);
    }
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(bad(
            line_no,
            &format!("`{key}` is not a usable key; quote it or rename it"),
        ));
    }
    Ok(key.to_string())
}

/// Parses a quoted string value, honouring `\"`, `\\`, `\n` and `\t`.
fn parse_string(line_no: usize, value: &str) -> Result<String> {
    let value = value.trim();
    let Some(rest) = value.strip_prefix('"') else {
        return Err(bad(
            line_no,
            &format!(
                "`{value}` is not a quoted string; this manifest has no numeric or boolean values"
            ),
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
                Some(other) => {
                    return Err(bad(line_no, &format!("unknown escape `\\{other}`")));
                }
                None => return Err(bad(line_no, "the string ends in a backslash")),
            },
            other => out.push(other),
        }
    }
    Err(bad(line_no, "the string is never closed"))
}

/// Escapes a value for [`Config::render`].
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

    const SAMPLE: &str = r#"
# a comment
[project]
prefix = "info/prefix.bin"      # trailing comment

[build]
output = "dist/My Map.w3x"

[text]

[binary]
"war3map.w3e" = "terrain/war3map.w3e"

[raw]
"war3map.wts" = "raw/war3map.wts.w3raw"
"war3mapImported\\hero.blp" = "assets/war3mapImported/hero.blp"
"#;

    #[test]
    fn parses_every_table_and_keeps_the_member_paths() {
        let c = Config::parse(SAMPLE).unwrap();
        assert_eq!(c.prefix, "info/prefix.bin");
        assert_eq!(c.output.as_deref(), Some("dist/My Map.w3x"));
        assert!(c.text.is_empty());
        assert_eq!(
            c.binary.get("war3map.w3e").map(String::as_str),
            Some("terrain/war3map.w3e")
        );
        assert_eq!(c.raw.len(), 2);
        // The escaped backslash in the key survives.
        assert_eq!(
            c.raw.get(r"war3mapImported\hero.blp").map(String::as_str),
            Some("assets/war3mapImported/hero.blp")
        );
    }

    #[test]
    fn render_round_trips_through_parse() {
        let c = Config::parse(SAMPLE).unwrap();
        assert_eq!(Config::parse(&c.render()).unwrap(), c);
    }

    #[test]
    fn render_is_sorted_so_two_runs_agree() {
        let c = Config::parse(SAMPLE).unwrap();
        assert_eq!(c.render(), c.render());
        let rendered = c.render();
        let wts = rendered.find("war3map.wts").unwrap();
        let imported = rendered.find("war3mapImported").unwrap();
        // Byte order, and `.` (0x2E) sorts before `I` (0x49), so the string table
        // comes first. What matters is that the order follows from the names, not
        // from the order the lines happened to be read in.
        assert!(wts < imported, "members should be in name order");
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        let c = Config::parse("[project]\nprefix = \"info/a#b.bin\"\n").unwrap();
        assert_eq!(c.prefix, "info/a#b.bin");
    }

    #[test]
    fn empty_tables_are_accepted() {
        let c = Config::parse("[project]\nprefix = \"p\"\n[binary]\n[raw]\n").unwrap();
        assert!(c.binary.is_empty() && c.raw.is_empty());
        assert_eq!(c.output, None);
    }

    #[test]
    fn a_missing_prefix_is_rejected_with_a_reason() {
        let err = Config::parse("[build]\noutput = \"x\"\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("prefix is missing"), "{err}");
    }

    #[test]
    fn numbers_and_bare_values_are_refused_not_guessed() {
        for (text, expect) in [
            ("[project]\nprefix = 12\n", "not a quoted string"),
            ("[project]\nprefix = info/x\n", "not a quoted string"),
        ] {
            let err = Config::parse(text).unwrap_err().to_string();
            assert!(err.contains(expect), "expected {expect:?} in {err:?}");
            assert!(err.contains("line 2"), "the line number matters: {err}");
        }
    }

    #[test]
    fn unknown_tables_keys_and_duplicates_are_refused() {
        for (text, expect) in [
            ("[nope]\n", "unknown table"),
            ("[[member]]\n", "array-of-tables"),
            ("[project]\nprefix = \"a\"\n[project]\n", "appears twice"),
            ("[project]\nprefix = \"a\"\nprefix = \"b\"\n", "set twice"),
            ("[build]\nnope = \"a\"\n", "unknown key"),
            ("key = \"v\"\n", "before any table header"),
            (
                "[raw]\n\"a\" = \"p\"\n\"a\" = \"q\"\n",
                "appears twice in [raw]",
            ),
        ] {
            let err = Config::parse(text).unwrap_err().to_string();
            assert!(err.contains(expect), "expected {expect:?} in {err:?}");
        }
    }

    #[test]
    fn unknown_escapes_and_unterminated_strings_are_refused() {
        for (text, expect) in [
            ("[project]\nprefix = \"a\\qb\"\n", "unknown escape"),
            ("[project]\nprefix = \"abc\n", "never closed"),
            ("[project]\nprefix = \"abc\" x\n", "after the closing quote"),
        ] {
            let err = Config::parse(text).unwrap_err().to_string();
            assert!(err.contains(expect), "expected {expect:?} in {err:?}");
        }
    }
}

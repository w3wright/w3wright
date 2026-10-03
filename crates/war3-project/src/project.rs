//! `extract` and `build`: the two directions of a source project.
//!
//! # The two directions are not symmetric, and that is the point
//!
//! `extract` has the map and decides a form per member. `build` has only the
//! project and the manifest, so it never guesses: every member it writes comes
//! from exactly one table, in the form that table names.
//!
//! ```text
//! .w3x ──extract──▶ project/ ──build──▶ .w3x
//!                    war3.toml              members byte-identical
//!                    info/prefix.bin        (the archive layout is not)
//!                    [text] [binary] [raw]
//! ```
//!
//! # What is not here yet
//!
//! Nothing is textified in this increment: `[text]` is written empty and `build`
//! **refuses** a manifest that lists text members rather than writing bytes it
//! cannot produce. Everything either decodes (and is stored as content) or does
//! not (and is stored as its block).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use war3_archive::{Archive, ArchiveBuilder};
use war3_core::diag::{Diagnostic, DiagnosticCode, Diagnostics};
use war3_core::{Error, Result};

use crate::codecs;
use crate::config::{Config, Members};
use crate::layout;
use crate::raw;

/// Where the `HM3W` prefix is kept. Not a member, so it needs a fixed home.
pub const PREFIX_PATH: &str = "info/prefix.bin";

/// What an extraction produced.
#[derive(Debug)]
pub struct ExtractReport {
    /// Members written, in total.
    pub members: usize,
    /// Members written as text.
    pub text: usize,
    /// Members stored as decoded content.
    pub binary: usize,
    /// Members stored as their original block.
    pub raw: usize,
    /// Size of the `HM3W` prefix that was carried across.
    pub prefix_bytes: usize,
    /// Where the manifest was written.
    pub manifest: PathBuf,
    /// Everything worth reporting, in the order it was found.
    pub diagnostics: Diagnostics,
}

/// What a build produced.
#[derive(Debug)]
pub struct BuildReport {
    /// Members written into the archive.
    pub members: usize,
    /// Members written as text.
    pub text: usize,
    /// Members written as decoded content.
    pub binary: usize,
    /// Members written from their original block.
    pub raw: usize,
    /// Where the archive was written.
    pub output: PathBuf,
    /// Size of the archive.
    pub output_bytes: u64,
    /// Members whose content did not survive the round trip. Always empty in a
    /// correct build — the check exists so that it is *measured*, not assumed.
    pub mismatched: Vec<String>,
    /// Everything worth reporting, in the order it was found.
    pub diagnostics: Diagnostics,
}

/// Writes a map out as a source project.
///
/// # Errors
///
/// A block in use whose name could not be recovered, unless `drop_unnamed` — an
/// unnamed block cannot be written back, and dropping it quietly would lose part
/// of the map. Also any two members that would need the same file.
pub fn extract(map: &Path, dir: &Path, drop_unnamed: bool) -> Result<ExtractReport> {
    let archive = Archive::open(map)?;
    let mut diagnostics = Diagnostics::new();

    let named = archive.file_count();
    let used = archive.used_block_count();
    if named < used {
        let lost = used - named;
        if !drop_unnamed {
            return Err(Error::msg(format!(
                "{used} blocks are in use but only {named} names could be recovered, so extracting \
                 would drop {lost} member(s); pass --drop-unnamed to accept that"
            )));
        }
        diagnostics.push(Diagnostic::warn(
            DiagnosticCode::MpqNoListfile,
            format!("{lost} member(s) have no recoverable name and were dropped"),
        ));
    }

    let prefix = archive.prefix().to_vec();
    write_project_file(dir, PREFIX_PATH, &prefix)?;

    let mut config = Config {
        prefix: PREFIX_PATH.to_string(),
        output: Some(default_output(map)),
        text: Members::new(),
        binary: Members::new(),
        raw: Members::new(),
    };
    let mut claimed: BTreeMap<String, String> = BTreeMap::new();
    let mut names = archive.file_names();
    names.sort_unstable();

    for name in names {
        if let Ok(content) = archive.read_file(name) {
            let (rel, encoded) = layout::path_for_reported(name);
            claim(&mut claimed, &rel, name)?;
            if encoded {
                diagnostics.push(Diagnostic::info(
                    DiagnosticCode::ProjectNameEncoded,
                    format!("{name}: stored as {rel}, which is not the member name"),
                ));
            }
            // The disposition comes from `disposition`, which is also what an
            // interface asks when it shows a member's fate. One implementation, so a
            // screen cannot claim "textified" for a member this writes as binary.
            match crate::disposition::text_form(name, &content, &mut diagnostics)? {
                Some(text) => {
                    write_project_file(dir, &rel, text.as_bytes())?;
                    config.text.insert(name.to_string(), rel);
                }
                None => {
                    write_project_file(dir, &rel, &content)?;
                    config.binary.insert(name.to_string(), rel);
                }
            }
            continue;
        }

        // Not decodable by this workspace, so the block itself is what gets
        // stored — see `raw`'s module docs for why those bytes are not "content".
        let member = archive.raw_member(name)?;
        let rel = layout::raw_path_for(name);
        claim(&mut claimed, &rel, name)?;
        write_project_file(dir, &rel, &raw::encode(&member))?;
        diagnostics.push(Diagnostic::warn(
            DiagnosticCode::ProjectMemberKeptRaw,
            format!(
                "{name}: kept as its stored block ({} bytes, flags {:#010X}); this workspace cannot \
                 decode it",
                member.block.len(),
                member.flags.0
            ),
        ));
        config.raw.insert(name.to_string(), rel);
    }

    let manifest = dir.join("war3.toml");
    write_project_file(dir, "war3.toml", config.render().as_bytes())?;

    Ok(ExtractReport {
        members: config.member_count(),
        text: config.text.len(),
        binary: config.binary.len(),
        raw: config.raw.len(),
        prefix_bytes: prefix.len(),
        manifest,
        diagnostics,
    })
}

/// The bytes a `[text]` member produces, read back from the file on disk.
///
/// One routine for all three callers — `build`, its self-verification, and
/// `validate` — because they must agree about what a text member reads back as.
/// When they disagree, "build succeeded" and "the reader gets the same bytes"
/// stop meaning the same thing.
///
/// The UTF-8 requirement is why script source cannot always be text: a script
/// whose bytes are not UTF-8 has no text form at all, and `extract` keeps it
/// binary rather than serialising it into something `build` would later refuse.
///
/// The message does not name the member, because every caller already prefixes
/// its own error line with `{name}:`; the path is carried instead, which is the
/// part the member name does not convey.
fn text_member_bytes(dir: &Path, name: &str, rel: &str) -> Result<Vec<u8>> {
    let bytes = read_project_file(dir, rel)?;
    let text = String::from_utf8(bytes).map_err(|e| {
        Error::msg(format!(
            "{rel}: a [text] member must be UTF-8 ({e}); move it to [binary] in war3.toml to \
             carry it verbatim"
        ))
    })?;
    let codec = codecs::codec_for_member(name, text.as_bytes()).ok_or_else(|| {
        Error::msg(
            "listed as text, but this build has no text form for it; move it to [binary] to \
             build it from its current bytes",
        )
    })?;
    (codec.from_text)(name, &text)
}

/// Builds a map back from a source project.
///
/// # Errors
///
/// A missing or unparsable manifest, a manifest path that escapes the project
/// directory, a `[text]` member this build has no text form for, or a
/// stored-block file that is not one.
pub fn build(dir: &Path, out: Option<&Path>) -> Result<BuildReport> {
    let manifest = dir.join("war3.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| Error::msg(format!("{}: {e}", manifest.display())))?;
    let config = Config::parse(&text)?;

    // The pipeline §14 describes starts here. Validating first means a broken
    // project fails with *every* problem at once, before anything is written —
    // rather than partway through an archive that then has to be thrown away.
    let checked = validate(dir)?;
    if !checked.is_ok() {
        return Err(Error::msg(format!(
            "{} problem(s) in the project, so nothing was written:\n  {}",
            checked.errors.len(),
            checked.errors.join("\n  ")
        )));
    }

    // §14's pipeline has a "compile script" step before packing. This workspace
    // has no JASS compiler, so when source scripts are present the honest thing is
    // to stop: packing whatever compiled member happens to be lying there would
    // ship a script that does not match its source, and nothing downstream could
    // tell. Refusing is the same trade the rest of this crate makes.
    let sources = dir.join("scripts").join("src");
    if let Ok(entries) = std::fs::read_dir(&sources) {
        let count = entries.flatten().count();
        if count > 0 {
            return Err(Error::msg(format!(
                "{} holds {count} source script(s) and this build has no JASS compiler: compile \
                 them into the script member yourself, or remove the directory — building now would \
                 pack the compiled script that is already there, which may not match",
                sources.display()
            )));
        }
    }

    let prefix = read_project_file(dir, &config.prefix)?;
    let mut builder = ArchiveBuilder::with_prefix(prefix)?;

    for (name, rel) in &config.text {
        builder.add_stored(name.clone(), text_member_bytes(dir, name, rel)?);
    }

    for (name, rel) in &config.binary {
        builder.add_stored(name.clone(), read_project_file(dir, rel)?);
    }
    for (name, rel) in &config.raw {
        let bytes = read_project_file(dir, rel)?;
        builder.add_raw(raw::decode(name, &bytes)?)?;
    }

    let output = match out {
        Some(path) => path.to_path_buf(),
        None => {
            let rel = config.output.as_ref().ok_or_else(|| {
                Error::msg("war3.toml has no [build] output, so pass --out <file>")
            })?;
            dir.join(rel)
        }
    };
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::msg(format!("{}: {e}", parent.display())))?;
        }
    }
    builder.write(&output)?;

    let mismatched = verify(&output, dir, &config)?;
    let output_bytes = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);

    Ok(BuildReport {
        members: config.member_count(),
        text: config.text.len(),
        binary: config.binary.len(),
        raw: config.raw.len(),
        output,
        output_bytes,
        mismatched,
        diagnostics: Diagnostics::new(),
    })
}

/// Reads the archive back and compares every member with the file it came from.
///
/// This is the check the prototype is judged by, run on every build: an archive
/// is only as good as what a reader gets back out of it.
fn verify(output: &Path, dir: &Path, config: &Config) -> Result<Vec<String>> {
    let rebuilt = Archive::open(output)?;
    let mut mismatched = Vec::new();

    for (name, rel) in &config.binary {
        match (read_project_file(dir, rel), rebuilt.read_file(name)) {
            (Ok(expected), Ok(actual)) if expected == actual => {}
            (Ok(expected), Ok(actual)) => mismatched.push(format!(
                "{name}: {} bytes in the project, {} in the archive",
                expected.len(),
                actual.len()
            )),
            (Ok(_), Err(e)) => mismatched.push(format!("{name}: unreadable in the archive ({e})")),
            (Err(e), _) => mismatched.push(format!("{name}: {e}")),
        }
    }

    // A text member is compared by serialising the text again: the file on disk is
    // the source now, and what matters is the bytes it produces.
    for (name, rel) in &config.text {
        let expected = text_member_bytes(dir, name, rel);
        match (expected, rebuilt.read_file(name)) {
            (Ok(expected), Ok(actual)) if expected == actual => {}
            (Ok(expected), Ok(actual)) => mismatched.push(format!(
                "{name}: {} bytes from the text, {} in the archive",
                expected.len(),
                actual.len()
            )),
            (Ok(_), Err(e)) => mismatched.push(format!("{name}: unreadable in the archive ({e})")),
            (Err(e), _) => mismatched.push(format!("{name}: {e}")),
        }
    }

    for (name, rel) in &config.raw {
        let Ok(bytes) = read_project_file(dir, rel) else {
            mismatched.push(format!("{name}: stored block is missing"));
            continue;
        };
        let expected = raw::decode(name, &bytes)?;
        match rebuilt.raw_member(name) {
            Ok(actual) if actual.block == expected.block && actual.flags == expected.flags => {}
            Ok(actual) => mismatched.push(format!(
                "{name}: stored block differs ({} bytes / flags {:#010X} in the archive)",
                actual.block.len(),
                actual.flags.0
            )),
            Err(e) => mismatched.push(format!("{name}: not in the archive ({e})")),
        }
    }

    Ok(mismatched)
}

/// What a [`validate`] found.
#[derive(Debug)]
pub struct ValidateReport {
    /// Members listed as text.
    pub text: usize,
    /// Members listed as decoded content.
    pub binary: usize,
    /// Members listed as their own stored block.
    pub raw: usize,
    /// Problems that would stop a build or produce a wrong file.
    pub errors: Vec<String>,
    /// Legal, but worth knowing.
    pub warnings: Vec<String>,
}

impl ValidateReport {
    /// Whether the project is buildable.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Checks that a source project is self-consistent, without building it.
///
/// `build` verifies what it wrote, but it can only tell you a project is broken by
/// failing partway through — and a text file a human edited can be broken in ways
/// only that member's own reader notices. This is the fast, read-only pass: the
/// manifest parses, every file it names exists, every text member reads back,
/// every stored block decodes, and no member is claimed by two tables (which would
/// write it twice, with the winner decided by iteration order).
///
/// # Errors
///
/// Only for a manifest that cannot be read at all. Everything else lands in
/// [`ValidateReport::errors`], because a validator that stops at the first problem
/// makes you fix them one run at a time.
pub fn validate(dir: &Path) -> Result<ValidateReport> {
    let manifest = dir.join("war3.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| Error::msg(format!("{}: {e}", manifest.display())))?;
    let config = Config::parse(&text)?;

    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let mut seen: BTreeMap<String, &'static str> = BTreeMap::new();
    for (table, members) in [
        ("text", &config.text),
        ("binary", &config.binary),
        ("raw", &config.raw),
    ] {
        for name in members.keys() {
            if let Some(previous) = seen.insert(name.to_lowercase(), table) {
                errors.push(format!("{name}: listed in both [{previous}] and [{table}]"));
            }
        }
    }

    if let Err(e) = read_project_file(dir, &config.prefix) {
        errors.push(format!("prefix ({}): {e}", config.prefix));
    }

    for (name, rel) in &config.text {
        if let Err(e) = text_member_bytes(dir, name, rel) {
            errors.push(format!("{name}: {e}"));
        }
    }

    for (name, rel) in &config.binary {
        if let Err(e) = read_project_file(dir, rel) {
            errors.push(format!("{name}: {e}"));
        }
    }

    for (name, rel) in &config.raw {
        match read_project_file(dir, rel) {
            Ok(bytes) => {
                if let Err(e) = raw::decode(name, &bytes) {
                    errors.push(format!("{name}: the stored block does not decode: {e}"));
                }
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }

    if config.member_count() == 0 {
        warnings.push("the manifest lists no members at all".to_string());
    }

    Ok(ValidateReport {
        text: config.text.len(),
        binary: config.binary.len(),
        raw: config.raw.len(),
        errors,
        warnings,
    })
}

/// Two members must not want the same file, and the error says which two.
fn claim(claimed: &mut BTreeMap<String, String>, path: &str, member: &str) -> Result<()> {
    if let Some(previous) = claimed.insert(path.to_string(), member.to_string()) {
        return Err(Error::msg(format!(
            "{member}: {path} is already taken by {previous}; the two member names encode to the \
             same file name"
        )));
    }
    Ok(())
}

/// A project-relative path, refusing anything that leaves the project.
fn resolve(dir: &Path, rel: &str) -> Result<PathBuf> {
    let escapes = rel.split(['/', '\\']).any(|segment| segment == "..");
    if escapes || Path::new(rel).is_absolute() || rel.starts_with(['/', '\\']) {
        return Err(Error::msg(format!(
            "war3.toml: `{rel}` points outside the project directory"
        )));
    }
    Ok(dir.join(rel))
}

fn read_project_file(dir: &Path, rel: &str) -> Result<Vec<u8>> {
    let path = resolve(dir, rel)?;
    std::fs::read(&path).map_err(|e| Error::msg(format!("{}: {e}", path.display())))
}

fn write_project_file(dir: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    let path = resolve(dir, rel)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::msg(format!("{}: {e}", parent.display())))?;
    }
    std::fs::write(&path, bytes).map_err(|e| Error::msg(format!("{}: {e}", path.display())))
}

/// The default `[build] output`: a `dist/` directory named after the map.
fn default_output(map: &Path) -> String {
    let stem = map
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "map".to_string());
    format!("dist/{}.w3x", layout::sanitise(&stem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use war3_archive::{BlockFlags, RawMember};

    /// A valid version 25 `war3map.w3i`, written as its **text form** and converted
    /// through the codec — the same path `extract` takes, so the fixture cannot
    /// drift away from the code that is being tested.
    const W3I_TEXT: &str = "\
[info]
version = \"25\"
name = \"TRIGSTR_003\"
author = \"someone\"
description = \"a description\"
camera_bounds = \"-2048 -2048 2048 2048 -2048 -2048 2048 2048\"
playable_width = \"64\"
playable_height = \"64\"
flags = \"0\"
[players]
";

    fn w3i_codec() -> codecs::Codec {
        codecs::codec_for("war3map.w3i").expect("the unit file has a text form")
    }

    fn w3i_bytes() -> Vec<u8> {
        (w3i_codec().from_text)("war3map.w3i", W3I_TEXT).expect("the fixture is a valid text form")
    }

    const W3E: &[u8] = b"terrain bytes";
    /// An imploded block: the `0x08` mask is real, the payload is not.
    const WTS: &[u8] = &[0x08, 0x11, 0x22, 0x33];

    /// A directory that removes itself, so the tests need no dependency.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "war3-project-{tag}-{}-{unique}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A map holding one decodable member, one more, and one we cannot read.
    fn a_map() -> (TempDir, PathBuf) {
        let dir = TempDir::new("extract");
        let map = dir.path().join("sample.w3x");
        let mut builder = ArchiveBuilder::with_prefix(vec![b'H'; 512]).unwrap();
        builder.add_stored("war3map.w3i", w3i_bytes());
        builder.add_stored("war3map.w3e", W3E.to_vec());
        builder
            .add_raw(RawMember {
                name: "war3map.wts".to_string(),
                uncompressed_size: 4096,
                block: WTS.to_vec(),
                flags: BlockFlags(0x8000_0000 | 0x0000_0100 | 0x0000_0200),
                locale: 0,
                platform: 0,
            })
            .unwrap();
        builder.write(&map).unwrap();
        (dir, map)
    }

    #[test]
    fn extract_then_build_keeps_every_member_and_the_prefix() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        let report = extract(&map, &project, false).unwrap();
        assert_eq!(
            (report.members, report.text, report.binary, report.raw),
            (3, 1, 1, 1)
        );
        assert_eq!(report.prefix_bytes, 512);
        // The undecodable member is reported, never dropped quietly.
        assert_eq!(report.diagnostics.warning_count(), 1);
        assert!(report.manifest.exists());

        let out = tmp.path().join("built.w3x");
        let built = build(&project, Some(&out)).unwrap();
        assert_eq!(built.members, 3);
        assert!(built.mismatched.is_empty(), "{:?}", built.mismatched);

        let archive = Archive::open(&out).unwrap();
        assert_eq!(archive.prefix(), vec![b'H'; 512]);
        assert_eq!(archive.read_file("war3map.w3i").unwrap(), w3i_bytes());
        assert_eq!(archive.read_file("war3map.w3e").unwrap(), W3E);
        let wts = archive.raw_member("war3map.wts").unwrap();
        assert_eq!(wts.block, WTS);
        assert_eq!(wts.locale, 0);
    }

    #[test]
    fn the_undecodable_member_keeps_its_flags_and_size() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();
        let out = tmp.path().join("built.w3x");
        build(&project, Some(&out)).unwrap();

        let original = Archive::open(&map)
            .unwrap()
            .raw_member("war3map.wts")
            .unwrap();
        let rebuilt = Archive::open(&out)
            .unwrap()
            .raw_member("war3map.wts")
            .unwrap();
        assert_eq!(rebuilt.flags, original.flags);
        assert_eq!(rebuilt.uncompressed_size, original.uncompressed_size);
    }

    /// Member names come back in whatever case the archive or its listfile
    /// spells them, so these tests look a mapping up without depending on that.
    fn path_of(members: &Members, name: &str) -> Option<String> {
        members
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, path)| path.clone())
    }

    #[test]
    fn the_manifest_lists_each_member_in_exactly_one_table() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();
        let text = std::fs::read_to_string(project.join("war3.toml")).unwrap();
        let config = Config::parse(&text).unwrap();
        assert_eq!(config.text.len(), 1, "the unit file should be text");
        assert_eq!(config.binary.len(), 1, "the terrain file stays binary");
        assert_eq!(config.raw.len(), 1);
        assert_eq!(
            path_of(&config.text, "war3map.w3i").as_deref(),
            Some("info/war3map.w3i")
        );
        assert_eq!(
            path_of(&config.binary, "war3map.w3e").as_deref(),
            Some("terrain/war3map.w3e")
        );
        assert_eq!(
            path_of(&config.raw, "war3map.wts").as_deref(),
            Some("raw/war3map.wts.w3raw")
        );
    }

    #[test]
    fn extracting_twice_writes_the_same_bytes() {
        let (tmp, map) = a_map();
        let (first, second) = (tmp.path().join("a"), tmp.path().join("b"));
        extract(&map, &first, false).unwrap();
        extract(&map, &second, false).unwrap();
        for rel in ["war3.toml", "terrain/war3map.w3e", "raw/war3map.wts.w3raw"] {
            assert_eq!(
                std::fs::read(first.join(rel)).unwrap(),
                std::fs::read(second.join(rel)).unwrap(),
                "{rel} should be reproducible"
            );
        }
    }

    #[test]
    fn a_broken_project_is_refused_before_anything_is_written() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();

        let manifest = project.join("war3.toml");
        let config = Config::parse(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        let terrain = config.binary.values().next().unwrap().clone();
        std::fs::remove_file(project.join(&terrain)).unwrap();
        let out = tmp.path().join("should-not-exist.w3x");

        let err = build(&project, Some(&out)).unwrap_err().to_string();
        assert!(err.contains("problem(s) in the project"), "{err}");
        assert!(err.contains("nothing was written"), "{err}");
        assert!(
            !out.exists(),
            "a refused build must not leave a half-written archive behind"
        );
    }

    #[test]
    fn a_fresh_project_validates_clean() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();

        let report = validate(&project).unwrap();
        assert!(report.is_ok(), "{:?}", report.errors);
        assert_eq!((report.text, report.binary, report.raw), (1, 1, 1));
    }

    #[test]
    fn every_problem_is_reported_not_just_the_first() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();

        // Break two members in two different ways, so a validator that stops at
        // the first problem would report only one of them.
        let manifest = project.join("war3.toml");
        let mut config = Config::parse(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        let w3i_rel = config.text.values().next().unwrap().clone();
        std::fs::write(project.join(&w3i_rel), "[info]\nversion = \"25\"\n").unwrap();

        let w3e_key = config.binary.keys().next().unwrap().clone();
        let w3e_rel = config.binary.remove(&w3e_key).unwrap();
        std::fs::remove_file(project.join(&w3e_rel)).unwrap();
        config.binary.insert(w3e_key, w3e_rel);
        std::fs::write(&manifest, config.render()).unwrap();

        let report = validate(&project).unwrap();
        assert_eq!(report.errors.len(), 2, "{:?}", report.errors);
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("the file is not a complete text form")),
            "the broken w3i text must be reported: {:?}",
            report.errors
        );
        assert!(
            report.errors.iter().any(|e| e.contains("war3map.w3e")),
            "the missing binary member must be reported too: {:?}",
            report.errors
        );
    }

    #[test]
    fn a_member_in_two_tables_is_an_error_before_it_is_written_twice() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();

        let manifest = project.join("war3.toml");
        let mut config = Config::parse(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        let (key, path) = config
            .binary
            .iter()
            .next()
            .map(|(k, v)| (k.clone(), v.clone()))
            .unwrap();
        config.text.insert(key, path);
        std::fs::write(&manifest, config.render()).unwrap();

        let report = validate(&project).unwrap();
        assert!(
            report.errors.iter().any(|e| e.contains("listed in both")),
            "{:?}",
            report.errors
        );
    }

    #[test]
    fn source_scripts_are_refused_rather_than_packed_stale() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();

        // A source script, as an author would add one.
        let sources = project.join("scripts").join("src");
        std::fs::create_dir_all(&sources).unwrap();
        std::fs::write(sources.join("war3map.j"), "// a script\n").unwrap();

        let out = tmp.path().join("should-not-exist.w3x");
        let err = build(&project, Some(&out)).unwrap_err().to_string();
        assert!(err.contains("no JASS compiler"), "{err}");
        assert!(
            !out.exists(),
            "a refused build must not leave an archive with a stale script in it"
        );

        // An empty directory is not a source script and does not stop a build.
        std::fs::remove_file(sources.join("war3map.j")).unwrap();
        assert!(build(&project, None).is_ok());
    }

    #[test]
    fn a_text_form_is_written_and_an_edit_in_it_reaches_the_archive() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();

        let text = std::fs::read_to_string(project.join("info/war3map.w3i")).unwrap();
        assert!(text.contains("[info]"), "{text}");
        assert!(text.contains("name = \"TRIGSTR_003\""), "{text}");

        // The half that matters: a human edit has to reach the bytes.
        let edited = text.replace("TRIGSTR_003", "W3WRIGHT B1B");
        std::fs::write(project.join("info/war3map.w3i"), &edited).unwrap();

        let out = tmp.path().join("edited.w3x");
        let built = build(&project, Some(&out)).unwrap();
        assert!(built.mismatched.is_empty(), "{:?}", built.mismatched);

        let bytes = Archive::open(&out)
            .unwrap()
            .read_file("war3map.w3i")
            .unwrap();
        assert_eq!(
            bytes,
            (w3i_codec().from_text)("war3map.w3i", &edited).unwrap()
        );
        assert_eq!(
            war3_map::w3i::MapInfo::parse(&bytes).unwrap().name,
            "W3WRIGHT B1B"
        );
    }

    #[test]
    fn a_text_member_with_no_text_form_is_refused_rather_than_written_wrong() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();
        let manifest = project.join("war3.toml");
        let mut config = Config::parse(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        let key = config
            .binary
            .keys()
            .find(|k| k.eq_ignore_ascii_case("war3map.w3e"))
            .cloned()
            .expect("the terrain file is in the manifest");
        let path = config.binary.remove(&key).unwrap();
        config.text.insert(key, path);
        std::fs::write(&manifest, config.render()).unwrap();

        let err = build(&project, None).unwrap_err().to_string();
        assert!(err.contains("no text form for it"), "{err}");
        assert!(err.to_lowercase().contains("war3map.w3e"), "{err}");
    }

    #[test]
    fn a_manifest_path_that_escapes_the_project_is_refused() {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();
        let manifest = project.join("war3.toml");
        let mut config = Config::parse(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        config
            .binary
            .insert("war3map.w3i".to_string(), "../outside.bin".to_string());
        std::fs::write(&manifest, config.render()).unwrap();

        let err = build(&project, None).unwrap_err().to_string();
        assert!(err.contains("outside the project"), "{err}");
    }

    #[test]
    fn a_corrupt_stored_block_is_reported() {
        // Long enough that the magic is what fails, not the length.
        let err = corrupt_block(&[b'x'; 32]);
        assert!(err.contains("W3RAWMEM"), "{err}");
    }

    #[test]
    fn a_truncated_stored_block_is_reported() {
        let err = corrupt_block(b"short");
        assert!(err.contains("at least"), "{err}");
    }

    /// Extracts a project, damages the stored block, and returns the build error.
    fn corrupt_block(bytes: &[u8]) -> String {
        let (tmp, map) = a_map();
        let project = tmp.path().join("project");
        extract(&map, &project, false).unwrap();
        std::fs::write(project.join("raw/war3map.wts.w3raw"), bytes).unwrap();
        build(&project, None).unwrap_err().to_string()
    }

    #[test]
    fn the_default_output_is_derived_from_the_map_name() {
        assert_eq!(
            default_output(Path::new("Maps/(4)LostTemple.w3m")),
            "dist/(4)LostTemple.w3x"
        );
    }
}

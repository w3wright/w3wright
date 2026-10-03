//! Where a member lives inside a source project.
//!
//! The mapping is a rule rather than data so that a project is readable at a
//! glance — `terrain/war3map.w3e` needs no explanation. The manifest still
//! records the resulting path, and **the manifest is the authority**: `build`
//! never re-derives a path, so a file may be renamed by hand as long as the
//! manifest is updated with it.
//!
//! Paths are lower-cased even when an archive spells a member `WAR3MAP.W3E`.
//! MPQ names are case-insensitive, so this costs nothing and buys a project that
//! does not depend on how a particular listfile happened to be written. The
//! member's own spelling stays the manifest *key*, and that is what a build
//! writes back into the archive.
//!
//! # Sanitising
//!
//! An archive member name is free-form: a map may hold `war3mapImported\hero.blp`
//! or a name containing `:` or `?`, which no Windows file system accepts. Such
//! characters are percent-encoded (`%3A`) rather than dropped, and because the
//! manifest maps the *original* name to the encoded path, nothing is lost.

/// Directory for a member name, and the file name inside it.
///
/// Returns a project-relative path with `/` separators, which the manifest and
/// every supported platform accept.
#[must_use]
pub fn path_for(member: &str) -> String {
    let lower = member.to_ascii_lowercase();
    let (dir, name) = split_known(&lower).unwrap_or(("assets", member));
    // `scripts\war3map.j` names its own directory, so strip it instead of
    // nesting it as `scripts/scripts/...`.
    let name = match name
        .strip_prefix(dir)
        .and_then(|rest| rest.strip_prefix(['\\', '/']))
    {
        Some(rest) => rest,
        None => name,
    };

    let mut out = String::with_capacity(dir.len() + name.len() + 1);
    out.push_str(dir);
    out.push('/');
    let mut first = true;
    for segment in name.split(['\\', '/']).filter(|s| !s.is_empty()) {
        if !first {
            out.push('/');
        }
        first = false;
        out.push_str(&sanitise(segment));
    }
    out
}

/// Percent-encodes what a file name cannot hold, and reports whether it did.
///
/// The second value is `true` when the name could not be used verbatim, which is
/// worth a diagnostic: the file on disk no longer matches the member name.
#[must_use]
pub fn sanitise_reported(segment: &str) -> (String, bool) {
    let mut out = String::with_capacity(segment.len());
    let mut changed = false;
    let stem_is_reserved = is_reserved(segment);
    for (index, c) in segment.char_indices() {
        let illegal = matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || (c as u32) < 0x20;
        // A trailing dot or space is silently dropped by Windows.
        let trailing = index + c.len_utf8() == segment.len() && (c == '.' || c == ' ');
        // `CON`, `NUL`, `COM1` … are device names, not file names.
        let reserved = index == 0 && stem_is_reserved;
        if illegal || trailing || reserved {
            out.push_str(&format!("%{:02X}", c as u32));
            changed = true;
        } else {
            out.push(c);
        }
    }
    (out, changed)
}

/// [`sanitise_reported`] without the report.
#[must_use]
pub fn sanitise(segment: &str) -> String {
    sanitise_reported(segment).0
}

/// The top-level directory a member lands in.
///
/// Exposed because a codec sometimes has to decide *what a member is* rather than
/// where it goes — script source is identified by the fact that it lands in
/// `scripts/`, so that the two spellings the game uses cannot be handled by two
/// different rules. `'static` because it comes from the mapping itself, not from
/// the member name, and returns `assets` for anything the mapping does not name.
#[must_use]
pub fn member_dir(member: &str) -> &'static str {
    split_known(&member.to_ascii_lowercase()).map_or("assets", |(dir, _)| dir)
}
/// Whether a name is a Windows device name, which cannot be a file name.
fn is_reserved(segment: &str) -> bool {
    let stem = segment
        .split('.')
        .next()
        .unwrap_or(segment)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}

/// The directory for the members whose names the project layout names.
fn split_known(lower: &str) -> Option<(&'static str, &str)> {
    let dir = match lower {
        "war3map.w3i" | "war3map.wts" | "war3map.imp" => "info",
        "war3map.w3e" | "war3map.shd" | "war3map.wpm" | "war3map.mmp" => "terrain",
        "war3map.w3u" | "war3map.w3t" | "war3map.w3a" | "war3map.w3b" | "war3map.w3d"
        | "war3map.w3h" | "war3map.w3q" => "objects",
        "war3map.doo" => "doodads",
        "war3mapunits.doo" => "units",
        "war3map.j" | "war3map.lua" => "scripts",
        _ if lower.starts_with("scripts\\") || lower.starts_with("scripts/") => "scripts",
        _ => return None,
    };
    Some((dir, lower))
}

/// [`path_for`] plus whether the file name had to be percent-encoded.
///
/// The second value is worth a diagnostic: the file on disk is then *not* named
/// like the member, and only the manifest knows the correspondence.
#[must_use]
pub fn path_for_reported(member: &str) -> (String, bool) {
    let path = path_for(member);
    let changed = member
        .split(['\\', '/'])
        .filter(|s| !s.is_empty())
        .any(|segment| sanitise(segment) != segment);
    (path, changed)
}

/// Where a member's *stored block* goes when it cannot be decoded: `raw/` plus
/// the encoded member name and a `.w3raw` suffix.
#[must_use]
pub fn raw_path_for(member: &str) -> String {
    let mut out = String::from("raw/");
    let mut first = true;
    for segment in member.split(['\\', '/']).filter(|s| !s.is_empty()) {
        if !first {
            out.push('/');
        }
        first = false;
        out.push_str(&sanitise(&segment.to_ascii_lowercase()));
    }
    out.push_str(".w3raw");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_members_land_in_their_directory() {
        for (member, expect) in [
            ("war3map.w3i", "info/war3map.w3i"),
            ("war3map.wts", "info/war3map.wts"),
            ("war3map.w3e", "terrain/war3map.w3e"),
            ("war3map.shd", "terrain/war3map.shd"),
            ("war3map.w3u", "objects/war3map.w3u"),
            ("war3map.doo", "doodads/war3map.doo"),
            ("war3mapUnits.doo", "units/war3mapunits.doo"),
            ("war3map.j", "scripts/war3map.j"),
            ("scripts\\war3map.j", "scripts/war3map.j"),
        ] {
            assert_eq!(path_for(member), expect, "{member}");
        }
    }

    #[test]
    fn paths_are_lowercased_whatever_the_archive_spells() {
        // Real listfiles carry names like `WAR3MAP.W3I`. The path does not inherit
        // that spelling — the manifest key does, and that is what a build writes.
        assert_eq!(path_for("WAR3MAP.W3I"), "info/war3map.w3i");
        assert_eq!(path_for("War3MapUnits.doo"), "units/war3mapunits.doo");
        assert_eq!(path_for("war3map.w3i"), "info/war3map.w3i");
    }

    #[test]
    fn anything_unknown_becomes_an_asset_and_keeps_its_structure() {
        assert_eq!(path_for("war3mapMap.blp"), "assets/war3mapMap.blp");
        assert_eq!(
            path_for("war3mapImported\\hero.blp"),
            "assets/war3mapImported/hero.blp"
        );
        assert_eq!(path_for("(listfile)"), "assets/(listfile)");
        assert_eq!(path_for("(attributes)"), "assets/(attributes)");
    }

    #[test]
    fn illegal_characters_are_encoded_not_dropped() {
        let (safe, changed) = sanitise_reported("a:b?c*d");
        assert_eq!(safe, "a%3Ab%3Fc%2Ad");
        assert!(changed);
        // Encoding is reversible in practice: the manifest keeps the name.
        assert!(!safe.contains([':', '?', '*']));
    }

    #[test]
    fn a_trailing_dot_or_space_is_encoded() {
        assert_eq!(sanitise_reported("name.").0, "name%2E");
        assert_eq!(sanitise_reported("name ").0, "name%20");
        assert!(!sanitise_reported("name.txt").1);
    }

    #[test]
    fn device_names_are_encoded() {
        assert_eq!(sanitise_reported("CON").0, "%43ON");
        assert_eq!(sanitise_reported("com1.txt").0, "%63om1.txt");
        // A name that merely starts with those letters is fine.
        assert_eq!(sanitise_reported("console.txt").0, "console.txt");
    }

    #[test]
    fn a_stored_block_keeps_the_project_shape() {
        assert_eq!(raw_path_for("war3map.wts"), "raw/war3map.wts.w3raw");
        assert_eq!(raw_path_for("WAR3MAP.WTS"), "raw/war3map.wts.w3raw");
        assert_eq!(
            raw_path_for("war3mapImported\\hero.blp"),
            "raw/war3mapimported/hero.blp.w3raw"
        );
    }

    #[test]
    fn the_directory_is_what_a_codec_can_recognise_a_member_by() {
        // Script source is found by directory, not by a list of names, so both
        // spellings the game uses resolve the same way.
        for member in [
            "war3map.j",
            "WAR3MAP.J",
            "scripts\\war3map.j",
            "war3map.lua",
        ] {
            assert_eq!(member_dir(member), "scripts", "{member}");
        }
        assert_eq!(member_dir("war3map.w3i"), "info");
        assert_eq!(member_dir("war3map.w3e"), "terrain");
        assert_eq!(member_dir("war3mapUnits.doo"), "units");
        assert_eq!(member_dir("war3mapMap.blp"), "assets");
    }

    #[test]
    fn ordinary_names_are_left_alone() {
        assert_eq!(sanitise("war3map.w3e"), "war3map.w3e");
        assert!(!sanitise_reported("hero_01.blp").1);
        // Parentheses, spaces and non-ASCII are all legal on disk.
        assert!(!sanitise_reported("中文 名字(1).blp").1);
    }
}

//! The map composition model.
//!
//! Holds what can be parsed today: metadata from `war3map.w3i`, `war3map.wts` and
//! `war3map.imp`, plus terrain from `war3map.w3e`. Units, doodads, objects and
//! scripts are added as their parsers land; declaring them now as empty vectors
//! would make "not parsed yet" indistinguishable from "parsed and empty".
//!
//! # `MapSource`
//!
//! The model does not own an archive. It reads member files through
//! [`MapSource`], so the desktop path can back it with an MPQ archive, tests with
//! an in-memory map, and the browser with user-selected file handles.
//!
//! This is a different abstraction from the asset source in the core crate: that
//! one reads **game** data, which is never bundled, while this one reads the
//! **user's map**, which travels with the project.

use std::collections::BTreeMap;
use std::path::Path;

use war3_core::diag::Diagnostics;
use war3_core::{Error, ParseError, Result};

use crate::imports::ImportList;
use crate::w3i::MapInfo;
use crate::wts::StringTable;

/// A provider of map member files, matched case-insensitively.
pub trait MapSource: std::fmt::Debug {
    /// Returns a member's bytes.
    fn get(&self, name: &str) -> Option<Vec<u8>>;

    /// Whether a member exists. Defaults to a lookup.
    fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

impl MapSource for war3_archive::Archive {
    fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.read_file(name).ok()
    }
}

/// An in-memory map source, for tests.
#[derive(Debug, Default, Clone)]
pub struct MemoryMapSource {
    files: BTreeMap<String, Vec<u8>>,
}

impl MemoryMapSource {
    /// Empty source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a member, keyed case-insensitively.
    pub fn insert(&mut self, name: impl AsRef<str>, bytes: impl Into<Vec<u8>>) {
        self.files.insert(normalise(name.as_ref()), bytes.into());
    }

    /// Builds a source in one expression.
    #[must_use]
    pub fn with(mut self, name: impl AsRef<str>, bytes: impl Into<Vec<u8>>) -> Self {
        self.insert(name, bytes);
        self
    }

    /// How many members were inserted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the source is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl MapSource for MemoryMapSource {
    fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.files.get(&normalise(name)).cloned()
    }
}

/// Normalises a member name to a case-insensitive key.
#[must_use]
pub fn normalise(name: &str) -> String {
    name.chars()
        .map(|c| if c == '/' { '\\' } else { c })
        .collect::<String>()
        .to_lowercase()
}

/// One parsed map.
///
/// Terrain is optional rather than defaulting to an empty terrain: "this map has
/// no `.w3e`" and "the terrain parsed as zero tiles" are different facts.
#[derive(Debug, Clone)]
pub struct Map {
    /// Map information. Always present.
    pub metadata: MapInfo,
    /// The string table, possibly empty.
    pub strings: StringTable,
    /// The import list.
    pub imports: ImportList,
    /// Terrain, absent when the map has no `.w3e`.
    pub terrain: Option<war3_terrain::Terrain>,
    /// Member list with sizes, for `map list`.
    pub files: Vec<MapFileEntry>,
    /// All diagnostics, including those from sub-parsers.
    pub diagnostics: Diagnostics,
}

/// One entry of the member list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapFileEntry {
    /// Member name.
    pub name: String,
    /// Size in the archive.
    pub size: u32,
}

impl Map {
    /// Assembles a map from any [`MapSource`].
    ///
    /// # What is fatal
    ///
    /// Only a missing or unparseable `war3map.w3i` is an error: without it there
    /// is no map. Missing `war3map.wts`, `war3map.imp` or `war3map.w3e` are all
    /// legitimate states and produce diagnostics instead:
    ///
    /// - an empty string table is normal for maps with no localised text;
    /// - no imports is normal;
    /// - no terrain means this is not a playable map, but it is still a valid
    ///   archive.
    ///
    /// Read as much as possible and say clearly what could not be read.
    pub fn from_source(source: &dyn MapSource) -> Result<Self> {
        let mut diagnostics = Diagnostics::new();

        // ---- war3map.w3i, required ----
        let w3i_bytes = source
            .get("war3map.w3i")
            .ok_or_else(|| Error::from(ParseError::NotFound("war3map.w3i".to_string())))?;
        let metadata = MapInfo::parse(&w3i_bytes)?;
        diagnostics.merge(&metadata.diagnostics);

        // ---- war3map.wts, optional ----
        let strings = match source.get("war3map.wts") {
            Some(bytes) => {
                let table = StringTable::parse(&bytes);
                diagnostics.merge(table.diagnostics());
                table
            }
            None => StringTable::new(),
        };

        // ---- war3map.imp, optional ----
        let imports = match source.get("war3map.imp") {
            Some(bytes) => match war3_archive::archive::parse_imports(&bytes) {
                Ok(entries) => {
                    let list = ImportList::from_entries(
                        1,
                        entries.into_iter().map(|e| (e.path, e.flags)).collect(),
                    );
                    diagnostics.merge(&list.diagnostics);
                    list
                }
                Err(e) => {
                    diagnostics.push(war3_core::Diagnostic::warn(
                        war3_core::DiagnosticCode::AssetFallbackUsed,
                        format!("war3map.imp failed to parse and was ignored: {e}"),
                    ));
                    ImportList::default()
                }
            },
            None => ImportList::default(),
        };

        // ---- war3map.w3e, optional ----
        let terrain = match source.get("war3map.w3e") {
            Some(bytes) => match war3_terrain::Terrain::parse(&bytes) {
                Ok(mut t) => {
                    diagnostics.merge(&t.diagnostics);
                    cross_check_terrain_size(&metadata, &mut t, &mut diagnostics);
                    Some(t)
                }
                Err(e) => {
                    diagnostics.push(war3_core::Diagnostic::error(
                        war3_core::DiagnosticCode::W3eRecordSizeMismatch,
                        format!("war3map.w3e failed to parse; terrain skipped: {e}"),
                    ));
                    None
                }
            },
            None => None,
        };

        let files = collect_file_list(source);

        // ---- resolve TRIGSTR_ references in the metadata ----
        let mut metadata = metadata;
        resolve_metadata_strings(&mut metadata, &strings, &mut diagnostics);

        Ok(Self {
            metadata,
            strings,
            imports,
            terrain,
            files,
            diagnostics,
        })
    }

    /// Opens a map from a file, through the MPQ reader.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let archive = war3_archive::Archive::open(path)?;
        let mut map = Self::from_source(&archive)?;
        // Carry over the archive's own diagnostics, such as a header that is not
        // at offset 0.
        map.diagnostics.merge(archive.diagnostics());
        Ok(map)
    }

    /// How many members were found.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Whether terrain was parsed.
    #[must_use]
    pub const fn has_terrain(&self) -> bool {
        self.terrain.is_some()
    }
}

/// Cross-checks the terrain size against the **whole** map size in the map info.
///
/// Two off-by-one traps meet here, and getting either wrong produces a warning
/// on every real ladder map:
///
/// 1. `.w3e` counts tile **points**, which is tiles plus one.
/// 2. The size to compare against is the *total* map, not the playable area.
///    `(4)LostTemple.w3m` is playable 124x124 with unplayable edges 16/20/18/18,
///    so 16 + 124 + 20 = 160 tiles across, i.e. 161 tile points — which is what
///    its `.w3e` holds. Comparing the playable 124 against 161 reports a
///    mismatch on a file that is perfectly consistent.
fn cross_check_terrain_size(
    metadata: &MapInfo,
    terrain: &mut war3_terrain::Terrain,
    diagnostics: &mut Diagnostics,
) {
    let expected_points_x = metadata.width() + 1;
    let expected_points_y = metadata.height() + 1;
    if terrain.width as i32 != expected_points_x || terrain.height as i32 != expected_points_y {
        diagnostics.push(war3_core::Diagnostic::warn(
            war3_core::DiagnosticCode::W3eRecordSizeMismatch,
            format!(
                "size mismatch: .w3i describes a map of {}x{} tiles (so {}x{} tile points) but \
                 .w3e has {}x{} tile points; the editor may have resized the map, or a \
                 third-party tool damaged one of the two",
                metadata.width(),
                metadata.height(),
                expected_points_x,
                expected_points_y,
                terrain.width,
                terrain.height
            ),
        ));
    }
}

/// Resolves every metadata string field that could be a `TRIGSTR_nnn` reference.
fn resolve_metadata_strings(
    metadata: &mut MapInfo,
    strings: &StringTable,
    diagnostics: &mut Diagnostics,
) {
    metadata.name = strings.resolve(&metadata.name, diagnostics);
    metadata.author = strings.resolve(&metadata.author, diagnostics);
    metadata.description = strings.resolve(&metadata.description, diagnostics);
    if let Some(v) = metadata.recommended_players.clone() {
        metadata.recommended_players = Some(strings.resolve(&v, diagnostics));
    }
    for slot in [
        &mut metadata.loading_screen_text,
        &mut metadata.loading_screen_title,
        &mut metadata.loading_screen_subtitle,
        &mut metadata.prologue_screen_text,
        &mut metadata.prologue_screen_title,
        &mut metadata.prologue_screen_subtitle,
    ] {
        if let Some(v) = slot.clone() {
            *slot = Some(strings.resolve(&v, diagnostics));
        }
    }
    for player in &mut metadata.players {
        player.name = strings.resolve(&player.name, diagnostics);
    }
    for force in &mut metadata.forces {
        force.name = strings.resolve(&force.name, diagnostics);
    }
    for table in &mut metadata.random_units {
        table.name = strings.resolve(&table.name, diagnostics);
    }
    for table in &mut metadata.random_items {
        table.name = strings.resolve(&table.name, diagnostics);
    }
}

fn collect_file_list(source: &dyn MapSource) -> Vec<MapFileEntry> {
    // `MapSource` can only read by name, not enumerate; enumeration is specific
    // to the archive layer. Probing the known-names list is the most this layer
    // can do, and the CLI reads the archive directly for a full listing.
    war3_archive::archive::KNOWN_MEMBER_NAMES
        .iter()
        .filter_map(|name| {
            source.get(name).map(|bytes| MapFileEntry {
                name: (*name).to_string(),
                size: bytes.len() as u32,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_w3i() -> Vec<u8> {
        minimal_w3i_sized([0, 0, 0, 0], 32, 32)
    }

    /// A `.w3i` with the given unplayable edges and playable area.
    fn minimal_w3i_sized(
        unplayable: [i32; 4],
        playable_width: i32,
        playable_height: i32,
    ) -> Vec<u8> {
        let mut b = Vec::new();
        let i32s = |b: &mut Vec<u8>, v: i32| b.extend_from_slice(&v.to_le_bytes());
        let strs = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        };
        i32s(&mut b, 25);
        i32s(&mut b, 1);
        i32s(&mut b, 6059);
        strs(&mut b, "TRIGSTR_001");
        strs(&mut b, "Author");
        strs(&mut b, "Desc");
        strs(&mut b, "");
        for _ in 0..8 {
            b.extend_from_slice(&0f32.to_le_bytes());
        }
        for v in unplayable {
            i32s(&mut b, v); // unplayable A B C D
        }
        i32s(&mut b, playable_width);
        i32s(&mut b, playable_height);
        i32s(&mut b, 0);
        b.push(b'L');
        i32s(&mut b, -1);
        strs(&mut b, "");
        strs(&mut b, "");
        strs(&mut b, "");
        strs(&mut b, "");
        i32s(&mut b, 1);
        strs(&mut b, "");
        strs(&mut b, "");
        strs(&mut b, "");
        strs(&mut b, "");
        i32s(&mut b, 0);
        b.extend_from_slice(&0f32.to_le_bytes());
        b.extend_from_slice(&0f32.to_le_bytes());
        b.extend_from_slice(&0f32.to_le_bytes());
        b.extend_from_slice(&[0, 0, 0, 255]);
        b.extend_from_slice(b"\0\0\0\0");
        strs(&mut b, "");
        b.push(b'L');
        b.extend_from_slice(&[255, 255, 255, 255]);
        b
    }

    fn minimal_w3e(width: u32, height: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"W3E!");
        b.extend_from_slice(&11i32.to_le_bytes());
        b.push(b'L');
        b.extend_from_slice(&0i32.to_le_bytes()); // usesCustomTileset
                                                  // Both texture lists declare zero entries and write none: the header
                                                  // declaration and the data have to agree or the header length is wrong.
        b.extend_from_slice(&0i32.to_le_bytes()); // a = 0
        b.extend_from_slice(&0i32.to_le_bytes()); // b = 0
        b.extend_from_slice(&width.to_le_bytes());
        b.extend_from_slice(&height.to_le_bytes());
        b.extend_from_slice(&0f32.to_le_bytes());
        b.extend_from_slice(&0f32.to_le_bytes());
        assert_eq!(b.len(), 37, "the fixed header is 37 bytes");
        for _ in 0..(width * height) {
            b.extend_from_slice(&0x2000i16.to_le_bytes());
            b.extend_from_slice(&0u16.to_le_bytes());
            b.push(0);
            b.push(0);
            b.push(0x22);
        }
        assert_eq!(b.len(), 37 + 7 * (width as usize) * (height as usize));
        b
    }

    #[test]
    fn assembles_a_minimal_map() {
        let source = MemoryMapSource::new()
            .with("war3map.w3i", minimal_w3i())
            .with(
                "war3map.wts",
                b"STRING 1\r\n{\r\nReal Map Name\r\n}\r\n".to_vec(),
            )
            .with("war3map.w3e", minimal_w3e(33, 33));

        let map = Map::from_source(&source).unwrap();
        assert_eq!(map.metadata.name, "Real Map Name");
        assert!(map.has_terrain());
        let terrain = map.terrain.as_ref().unwrap();
        assert_eq!(terrain.width, 33);
        assert_eq!(terrain.height, 33);
        // A 32x32 playable area means 33x33 tile points, so no size diagnostic.
        assert!(!map
            .diagnostics
            .items()
            .iter()
            .any(|d| d.message.contains("size mismatch")));
    }

    #[test]
    fn terrain_size_is_compared_against_the_whole_map_not_the_playable_area() {
        // `(4)LostTemple.w3m`'s geometry: playable 32x32 with unplayable edges
        // 16/20/18/18, so 16 + 32 + 20 = 68 tiles across and down, i.e. 69x69
        // tile points. Comparing the playable 32 against 69 warns on a map that
        // is perfectly consistent.
        let source = MemoryMapSource::new()
            .with("war3map.w3i", minimal_w3i_sized([16, 20, 18, 18], 32, 32))
            .with("war3map.w3e", minimal_w3e(69, 69));
        let map = Map::from_source(&source).unwrap();
        assert_eq!(map.metadata.width(), 68);
        assert_eq!(map.metadata.height(), 68);
        assert!(
            !map.diagnostics
                .items()
                .iter()
                .any(|d| d.message.contains("size mismatch")),
            "{:?}",
            map.diagnostics.items()
        );
    }

    #[test]
    fn a_genuine_terrain_size_mismatch_is_still_reported() {
        let source = MemoryMapSource::new()
            .with("war3map.w3i", minimal_w3i())
            .with("war3map.w3e", minimal_w3e(64, 64));
        let map = Map::from_source(&source).unwrap();
        assert!(map
            .diagnostics
            .items()
            .iter()
            .any(|d| d.message.contains("size mismatch")));
    }

    #[test]
    fn missing_w3i_is_a_hard_error() {
        let source = MemoryMapSource::new().with("war3map.w3e", minimal_w3e(2, 2));
        let err = Map::from_source(&source).unwrap_err();
        assert!(err.to_string().contains("war3map.w3i"), "{err}");
    }

    #[test]
    fn missing_wts_degrades_without_failing() {
        let source = MemoryMapSource::new().with("war3map.w3i", minimal_w3i());
        let map = Map::from_source(&source).unwrap();
        assert_eq!(map.metadata.name, "TRIGSTR_001");
        assert!(map
            .diagnostics
            .items()
            .iter()
            .any(|d| d.code == war3_core::DiagnosticCode::WtsMissingKey));
    }

    #[test]
    fn corrupt_terrain_is_diagnosed_but_the_map_still_loads() {
        let source = MemoryMapSource::new()
            .with("war3map.w3i", minimal_w3i())
            .with("war3map.w3e", b"W3E!garbage".to_vec());
        let map = Map::from_source(&source).unwrap();
        assert!(!map.has_terrain());
        assert!(map
            .diagnostics
            .items()
            .iter()
            .any(|d| d.message.contains("war3map.w3e failed to parse")));
    }

    #[test]
    fn size_mismatch_between_w3i_and_w3e_is_reported() {
        // w3i says 32x32 tiles, so 33x33 tile points are expected; w3e has 64x64.
        let source = MemoryMapSource::new()
            .with("war3map.w3i", minimal_w3i())
            .with("war3map.w3e", minimal_w3e(64, 64));
        let map = Map::from_source(&source).unwrap();
        assert!(map
            .diagnostics
            .items()
            .iter()
            .any(|d| d.message.contains("size mismatch")));
    }

    #[test]
    fn map_source_normalises_case_and_separators() {
        let source = MemoryMapSource::new().with("War3Map.W3I", minimal_w3i());
        assert!(source.get("war3map.w3i").is_some());
        assert!(source.get("WAR3MAP.W3I").is_some());
    }
}

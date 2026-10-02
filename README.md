# w3wright

**English** | [中文](README_CN.md)
A Warcraft III map development platform, written in Rust.

This repository is the implementation of the design captured in `docs/`. That
design is authoritative; this README only says where the code is, how to run it,
and how far along it is.

> **Note**: `docs/` is not in the tree. The design document is referenced as
> authoritative but has never been committed, which is a gap worth closing — the
> conventions below are currently the only place some of it is written down.

> Target game version: **1.27**
> CLI binary: **`war3`**, crate prefix: **`war3-*`**

---

## Status

Phase 1 covers the W3X parser: everything needed to open a `.w3x` and read the
metadata-class, terrain-class and object-class files inside it, from real maps and
real Blizzard archives. The archive write side exists as well; what is still
missing is the source-project direction.

| Crate | State | Contents |
| --- | --- | --- |
| `war3-core` | done | `FourCC`, `Vec3`, error and diagnostic machinery, injectable asset source |
| `war3-archive` | done | Blizzard archive reading **and writing**: header discovery, table decryption, hash lookup, enumeration ladder, sector decryption, hand-written inflate, `ArchiveBuilder` |
| `war3-map` | done | `.w3i` (versions 0–33), `.wts`, `.imp`, `war3map.doo`, `war3mapUnits.doo`, the map composition model |
| `war3-terrain` | done | `.w3e` in both the v11 and v12 layouts, plus a SYLK reader |
| `war3-meta` | done | object field metadata read from the game's `*MetaData.slk`, `TriggerData.txt` trigger definitions, the editor-data type registry |
| `war3-object` | done | object data (`.w3u` and friends): inheritance, field typing, `TRIGSTR_` resolution, and writing back — **137 of 137 members over 7 kinds byte-identical** on a real corpus (`cargo run --example objects_roundtrip -p war3-object -- <maps>`) |
| `war3-project` | partial | source project: `war3 map extract` / `war3 validate` / `war3 build`. `war3map.w3i`, **the seven object files, `war3map.doo`, `war3mapUnits.doo` and the script** are text (edit a unit's field, a doodad's position or the map description and the game sees it); every other member is decoded binary or a verbatim block, and a member is only textified once its text form is proved to reproduce it byte for byte |
| `war3-cli` | done | the `war3` binary |
| `.w3x` round trip | done for member content | `war3 map rebuild` rewrites an archive member by member; `extract` / `build` round-trip every member's content byte for byte |
| `war3-wtg` | not started | trigger data (`.wtg` / `.wct`); prototype scope is a lossless round trip only |
| `war3-script` | not started | script front end (**read-only**): JASS / vJASS / Lua lexing, parsing, AST, symbol analysis. It does not execute, **does not emit source**, and does not infer types — scope in ADR-0024, boundaries in ADR-0025 |

**353 tests pass (`cargo test --workspace`) and `cargo clippy --all-targets` is clean.** The dependency
set is empty apart from the crates in this workspace, including the zlib
decompressor, so the core builds for WASM and with a plain Rust toolchain.

### What it reads today

| Target | Result |
| --- | --- |
| `D:\Warcraft3\war3.mpq` | hash table decrypts (22084 of 32768 slots empty); **10684 members** in the block table |
| `D:\Warcraft3\War3xlocal.mpq` | 1133 members |
| `(4)LostTemple.w3m` | 16 members by name; `.w3i` v18, `.w3e` v11 (161x161 tile points), `war3map.j` 72697 bytes of JASS, **5317 doodads**; its `war3mapUnits.doo` is **v7/sub9 and still unparsed** — reported with an offset, never guessed |
| `ydwe-sample-1.19.w3x` | `.w3i` v25, `.w3e` v11, `.w3u`, **125 placed units**, and a non-ASCII map name (`YDWE的UI演示`) via the string table |
| `war3 map rebuild` | `(4)LostTemple.w3m` → **16 of 16 members byte-identical** (238.5 KB from a 245.2 KB source); both rebuild modes (`--stored` too) load in the game client |
| `.w3i` re-serialisation | **188 of 190 maps parse → serialise byte for byte** (`cargo run --example w3i_roundtrip -p war3-map -- <maps>`); the two that differ have an imploded `war3map.w3i` |
| placement re-serialisation | `war3map.doo` **188 of 188 maps byte-identical** (735,618 doodads) and `war3mapUnits.doo` **175 of 175** (26,826 units): `cargo run --release --example doodads_roundtrip -p war3-map -- <maps>` |
| source-project round trip | the whole `extract → build` path over every map on this machine, with the member counts and every refusal printed: `cargo run --release --example project_regression -p war3-project -- <maps>` |
| script textification | `war3map.j` lands in `[text]` whenever it is representable as UTF-8 (moved verbatim, never parsed); over 190 maps textified members went **594 → 762** while member content stayed byte-identical on **172 of 172** maps, 0 failures. A script that is not UTF-8 stays in `[binary]` and is diagnosed |
| `war3 meta check D:\Warcraft3` | 7 field-metadata tables (267 unit fields, 747 ability fields, …) and 164 trigger types / 1389 actions |

Extraction is byte-identical to an independent implementation of the format for
`war3map.w3i`, `war3map.w3e` and `war3map.j`.

**Not yet supported: PKWare implode** (`0x08` sectors), which is what a map's
`war3map.wts` and Blizzard's own `(listfile)` use. Huffman, bzip2 and ADPCM are
reported as unsupported too. See
[`examples/lost-temple/README.md`](examples/lost-temple/README.md).
An imploded member is not lost on a rebuild: it is copied across verbatim, and
`TRIGSTR_` references that need it stay unresolved and are diagnosed.
Measured on the local 193-map corpus: **5 maps cannot be enumerated at all
because their `(listfile)` is imploded**, and 13 more because it is absent or
reduced (see Q20 in the `docs/` design set).


---

## Quick start

```bash
cargo build --release

# Map information, the main Phase 1 verb.
./target/release/war3 map info "<map>"

# Archive structure. Run this first when a map will not open.
./target/release/war3 map archive "<map>"

# Terrain statistics, placed doodads, placed units.
./target/release/war3 map terrain "<map>"
./target/release/war3 map doodads "<map>"
./target/release/war3 map units "<map>"

# Object data. Field ids become readable names when a game directory is given;
# without one they are printed as FourCCs.
./target/release/war3 map objects "<map>" --game-dir "D:\Warcraft3"

# Member listing, and extracting a single member.
./target/release/war3 map list "<map>"
./target/release/war3 map file "<map>" war3map.w3i > w3i.bin

# ⚠️ Use --out whenever the bytes are a *baseline* (round-trip comparison, parsing,
#    compiling outside this workspace). stdout is a text channel: measured on a
#    4,059,324-byte script it produced 4,184,598 bytes instead (+3.09%).
./target/release/war3 map file "<map>" WAR3MAP.J --out scripts/war3map.j

# Rewrite the archive with this workspace's writer, and verify the result.
./target/release/war3 map rebuild "<map>" out.w3x

# Source project: a map becomes a directory, and comes back.
./target/release/war3 map extract "<map>" my-project/
./target/release/war3 validate my-project/          # read-only: every problem at once, exit 2 if any
./target/release/war3 build my-project/ --out rebuilt.w3x

# Check whether metadata and trigger definitions can be found locally.
./target/release/war3 meta check "D:\Warcraft3"
```

`--verbose` also prints info-level diagnostics; by default only warnings and
above appear.

**Exit codes**: `0` success, `1` success with diagnostics, `2` usage error or
unreadable input.

---

## Layout

```text
w3wright/
├── Cargo.toml                 # virtual manifest, members = ["crates/*"]
├── crates/
│   ├── war3-core/             # FourCC / Vec3 / errors / diagnostics / AssetSource
│   ├── war3-archive/          # Blizzard archive reading and writing + a hand-written inflate
│   ├── war3-map/              # .w3i / .wts / .imp / .doo / war3mapUnits.doo and the map model
│   ├── war3-terrain/          # .w3e v11 and v12, plus SYLK
│   ├── war3-meta/             # object field metadata and trigger definitions
│   ├── war3-object/           # object data (.w3u and friends)
│   ├── war3-project/          # source project: extract / build
│   ├── war3-cli/              # umbrella crate providing the war3 binary
│   └── (war3-script/ arrives in Phase 3)
├── examples/                  # test map samples (binaries are gitignored)
└── README.md, README_CN.md
```

### Dependency direction

```text
war3-cli  <- umbrella crate, the only one producing a binary
  ├── war3-map     -> war3-archive, war3-terrain, war3-core
  ├── war3-project -> war3-archive, war3-core
  ├── war3-object  -> war3-meta, war3-core
  ├── war3-meta    -> war3-terrain, war3-core
  ├── war3-archive -> war3-core
  └── war3-terrain -> war3-core
```

`war3-cli` depends on the others; none of the others depends on `war3-cli`.

⚠️ **`war3-script` (Phase 3) has an extra hard rule: it may depend on `war3-core` and
nothing else.** It does not depend on any map-format crate and never opens an archive
itself — reading game assets (`common.j` / `blizzard.j`) is injected by the caller as an
`AssetSource`. That rule is what keeps a future split down to moving a directory
(ADR-0018); **do not split it before the rule is broken.**

---

## Conventions worth knowing

These are not style preferences. Each one corresponds to a failure that is
silent — the parser reports success and produces wrong data.

1. **Never drop anything quietly.** Fields that cannot be interpreted, version
   fields that disagree with the file, and metadata that is missing all produce a
   diagnostic rather than being skipped.
2. **Verify the geometry before trusting a version field.** A `.w3e` file's
   record size is derived from the file length and the tile count, then compared
   against the version field. Trusting the field alone corrupts v12 files.
3. **Read bit fields as unsigned, then mask.** The water word is a `u16` carrying
   a 14-bit level; reading it as `i16` and masking afterwards hits sign extension.
4. **Preserve bytes whose meaning is unknown.** Reserved bits survive a round trip
   verbatim rather than being normalised away.
5. **Every offset in the format is relative to the archive header.** That covers
   the table positions in the header *and* a block entry's member position, while
   the `HM3W` prefix means the header is usually at 512 rather than 0. Reading a
   member without adding the header offset yields data shifted by 512 bytes, with
   no error.
6. **Never trust encryption and decryption to validate each other.** They are an
   exact inverse pair, so a writer and a reader that share a mistake round-trip
   perfectly. Pin pinned ciphertext against a real archive instead; this is why
   `war3-archive`'s tests carry fixed vectors.
7. **No dependencies unless there is a reason.** The core, terrain and metadata
   crates have none; the MPQ crate implements inflate itself so that the
   workspace stays pure Rust and reaches WASM.

---

## Workspace layout of test data

`examples/lost-temple/` holds map samples, and `examples/fresh/` is reserved for
maps known to be unmodified. **Map binaries are gitignored**: official maps
belong to Blizzard, sample maps shipped inside other projects carry their own
licences, and community maps need the author's permission before redistribution.

One sample is generated locally instead:
`cargo run --example make_synthetic -p war3-archive` writes a structurally valid MPQ
archive — encrypted tables, four storage layouts, sectors that are genuinely
deflated and sectors stored raw, and deliberately no `(listfile)`. See
[`examples/lost-temple/README.md`](examples/lost-temple/README.md).

---

## On the misdiagnosis this README used to contain

Earlier revisions of this file and of `examples/lost-temple/README.md` claimed
that every real file on this machine was protected or repackaged, that a map
protector or an undocumented key derivation was at fault, and that **"a clean
sample is needed to continue"**. That was wrong. The files were ordinary; the
reader had four defects, each of which failed silently:

1. the two tables were keyed with ASCII constants (`HASH`, `BLK#`) instead of
   `HashString("(hash table)", MPQ_HASH_FILE_KEY)` and the same for
   `(block table)`;
2. the cipher's key update folded in the plaintext instead of walking its own
   sequence;
3. member offsets were read as file-relative rather than archive-relative;
4. a sector's `0xFF` first byte was treated as a "compressed" marker; the format
   has no such byte, and decides between a stored and a compressed sector by
   comparing the stored size against the data the sector must hold.

The old evidence for "these files are unusual" was that the hash table decrypted
to zero empty slots out of 32768, where a valid table is mostly empty. That
measurement was correct and the inference was not: it is exactly what a wrong key
produces. The decisive control sample was already in the repository and could not
help, because the generator shared the reader's cipher and its keys — which the
old README even warned about, without noticing that it applied to the very check
being used. Real archives were the only thing that could settle it, and they now
serve as the reference: see [`examples/lost-temple/README.md`](examples/lost-temple/README.md)
for the measured evidence and for the pinned vectors in
`war3_archive::crypto`'s tests that keep this from recurring.


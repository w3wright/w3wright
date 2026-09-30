# w3wright

**English** | [中文](README_CN.md)
A Warcraft III map development platform, written in Rust.

This repository is the implementation of the design captured in `docs/`. That
design is authoritative; this README only says where the code is, how to run it,
and how far along it is.

> Target game version: **1.27**
> CLI binary: **`war3`**, crate prefix: **`war3-*`**

---

## Status

Phase 1 covers the W3X parser: everything needed to open a `.w3x` and read the
metadata-class and terrain-class files inside it.

| Crate | State | Contents |
| --- | --- | --- |
| `war3-core` | done | `FourCC`, `Vec3`, error and diagnostic machinery, injectable asset source |
| `war3-archive` | done | Blizzard archive reading: header discovery, table decryption, hash lookup, enumeration ladder, sector decryption, hand-written inflate |
| `war3-map` | done | `.w3i` (versions 0–33), `.wts`, `.imp`, map composition model |
| `war3-terrain` | done | `.w3e` in both the v11 and v12 layouts, plus a SYLK reader |
| `war3-meta` | done | SYLK metadata tables and `TriggerData.txt` trigger definitions |
| `war3-cli` | done | the `war3` binary |
| `war3-object` | not started | object data (`.w3u` and friends) |
| writing `.w3x` | not started | the build direction |

**192 unit tests pass and `cargo clippy --all-targets` is clean.** The dependency
set is empty apart from the crates in this workspace, including the zlib
decompressor, so the core builds for WASM and with a plain Rust toolchain.

---

## Quick start

```bash
cargo build --release

# Map information, the main Phase 1 verb.
./target/release/war3 map info "<map>"

# Archive structure. Run this first when a map will not open.
./target/release/war3 map archive "<map>"

# Terrain statistics.
./target/release/war3 map terrain "<map>"

# Member listing, and extracting a single member.
./target/release/war3 map list "<map>"
./target/release/war3 map file "<map>" war3map.w3i > w3i.bin

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
│   ├── war3-archive/          # Blizzard archive reading + a hand-written inflate
│   ├── war3-map/              # .w3i / .wts / .imp and the map composition model
│   ├── war3-terrain/          # .w3e v11 and v12, plus SYLK
│   ├── war3-meta/             # object field metadata and trigger definitions
│   └── war3-cli/              # umbrella crate providing the war3 binary
├── examples/                  # test map samples (binaries are gitignored)
└── README.md, README_CN.md
```

### Dependency direction

```text
war3-cli  <- umbrella crate, the only one producing a binary
  ├── war3-map  -> war3-archive, war3-terrain, war3-core
  ├── war3-meta -> war3-terrain, war3-core
  ├── war3-archive  -> war3-core
  └── war3-terrain  -> war3-core
```

`war3-cli` depends on the others; none of the others depends on `war3-cli`.

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
5. **Keep the two coordinate systems apart.** MPQ table offsets are relative to
   the header, while a member's offset is relative to the file. Mixing them
   yields data shifted by 512 bytes, with no error.
6. **No dependencies unless there is a reason.** The core, terrain and metadata
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
archive — encrypted tables, three storage layouts, and deliberately no
`(listfile)` — which is what isolates "the reader is wrong" from "this particular
file is unusual". See [`examples/lost-temple/README.md`](examples/lost-temple/README.md).

---

## Known issue: table decryption on the local installation

Reading a synthetic archive end to end works: header discovery, table
decryption, hash lookup, name enumeration, sector decryption and zlib
decompression all produce content identical to what was written. The crypt table
and the filename hash also match an independent implementation value for value.

For the real files on this machine, however — the maps under `D:\Warcraft3\` and
the game's own `war3.mpq`, `War3x.mpq` and `War3xLocal.mpq` — the hash and block
tables decrypt into random bytes. The decisive measurement: `war3.mpq` has a
32768-entry hash table in which the number of unused slots is **zero**, whereas a
valid archive is mostly unused slots (the synthetic archive measures 61 of 64).

So those files' tables are not stored the way the format documentation describes.
Possible causes are a map protector, a repackaging tool, or a key derivation step
that is not documented. **A clean sample is needed to continue**, and
[`examples/fresh/README.md`](examples/fresh/README.md) describes how to produce one
and what to look for.

This does not block the rest of Phase 1: each format parser has unit tests over
byte streams it constructs itself. What is missing is the end-to-end run against a
real map.

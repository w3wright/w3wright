# Test map samples

> The `.w3x` / `.w3m` binaries in this directory are **not committed**. See
> "Licensing and provenance" below. Only this README is tracked.

Samples are organised by what they cover rather than by count.

## What is here

| File | Source | Covers | Licence |
| --- | --- | --- | --- |
| `(4)LostTemple.w3m` | the local installation's `D:\Warcraft3\Maps\` | a classic TFT melee map; **MPQ header at offset 512**; `.w3e`, `.w3i`, `.w3u`, `.w3d` | Blizzard's own map; **local verification only, do not redistribute** |
| `ydwe-sample-1.19.w3x` | the YDWE repository's example maps | non-ASCII paths and object data extended by YDWE | the YDWE repository is **GPL-3.0**; **local verification only** |
| `synthetic.w3x` | **generated locally** (see below) | a **structurally valid** MPQ: encrypted tables, multi-block zlib, single-block zlib, stored, and no `(listfile)` | generated here, no third-party rights |

## Why `synthetic.w3x` exists

This is not a nice-to-have. It is the only way to tell "the reader is wrong" apart
from "this file is unusual".

The MPQ read path has several linked stages: header discovery, table decryption,
hash lookup, name enumeration, sector decryption, decompression. Debugging
against a real map that will not open gives no clue which stage is at fault. An
archive with a known-correct answer does.

```bash
cargo run --example make_synthetic -p war3-mpq -- examples/lost-temple/synthetic.w3x
```

The generator writes the archive with the same crypto primitives the reader uses,
so it exercises:

- tables encrypted with the `HASH` and `BLK#` keys, covering table decryption;
- 4096-byte sectors with their per-sector compression flag;
- three members covering the multi-block, single-block and stored layouts;
- **no `(listfile)`**, forcing the reader onto its known-names fallback;
- the header at offset 512 behind an `HM3W` prefix.

Because generator and reader share those primitives, the generator **cannot**
catch a mistake in the crypto itself. It guards the assembly: the two offset
systems, sector lengths, the enumeration ladder, branch selection. The crypto is
guarded by three other means:

1. `crypto::tests::crypt_table_first_entries_are_stable` pins the table's opening
   values, checked against an independent implementation of the documented rule;
2. hash property tests, for case and separator insensitivity;
3. running against real archives, below.

## Using the real samples

```bash
cargo run -p war3-cli -- map info "examples/lost-temple/(4)LostTemple.w3m"
cargo run -p war3-cli -- map archive "examples/lost-temple/(4)LostTemple.w3m"
cargo run -p war3-cli -- map terrain "examples/lost-temple/(4)LostTemple.w3m"
```

## Licensing and provenance

| Source | Handling |
| --- | --- |
| maps you built yourself | committed, used as golden files |
| community maps | need the author's permission to redistribute; by default used for local checks only |
| maps bundled with YDWE | the repository is GPL-3.0, so **local verification only** |
| Blizzard's own maps | **local verification only**, never redistributed |

Every `.w3x` and `.w3m` file here is therefore excluded by `.gitignore`. To get a
sample, either place your own, or generate the synthetic archive.

## Known issue: the real maps cannot be read

**Every map under `D:\Warcraft3\` — including `(4)LostTemple.w3m` and
`ydwe-sample-1.19.w3x` — yields no member files.**

### Ruled out

| Stage | State | Evidence |
| --- | --- | --- |
| header discovery at offset 512 | correct | header fields are self-consistent: `hash_pos + hash_size*16 == block_pos`, and `block_pos + block_size*16 == archive_size` |
| crypt table generation | correct | matches an independent implementation value for value |
| `hash_string` | correct | matches an independent implementation value for value (`WAR3MAP.W3I` gives `TABLE_OFFSET=987145CE`, `HASH_A=33E887B7`) |
| table decryption | correct | `encrypt` composed with `decrypt` is the identity |
| hash lookup and the enumeration ladder | correct | the synthetic archive resolves all three names to the right block indices |
| sector decryption and zlib | correct | the synthetic archive reads all three storage layouts back byte-identically |

### What fails

**The block tables of the real maps and of the game's own archives decrypt into
random bytes**: member offsets come out as values such as `0x18B2B860` and
`0x63EB8C10`, far beyond the file length.

The decisive measurement: `war3.mpq` has a 32768-entry hash table, and a valid one
is mostly unused slots marked `0xFFFFFFFF`. Measured unused count: **0 of 4096**
for all 256 candidate low bytes of the key. The synthetic archive measures 61 of
64 under the same count.

So the data at that offset is not a hash table encrypted in the documented way.

Keys tried and rejected: `BLK#`, `HASH`, `(block)`, `(hash)`, the empty key,
`(block table)`, `(hash table)`, and a sweep over the low byte. The independent
Python implementation `mpyq` fails on the same files: `Encryption is not supported
yet` for `war3.mpq` and `Invalid file header` for the maps.

### Conclusion

The implementation has been validated stage by stage against a conforming archive,
so this is a property of **these files**: a map protector, a repackaging tool, or
an undocumented key derivation step.

### Next step: a clean sample

Produce a map with the local `World Editor.exe` — new map, default size, save —
and place it in [`../fresh/`](../fresh/README.md). The commands to run and the
criteria to judge them by are there.

```bash
cargo run --example dump_mpq -p war3-mpq -- "<map or archive>"
```

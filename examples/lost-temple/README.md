# Test map samples

> The `.w3x` / `.w3m` binaries in this directory are **not committed**. See
> "Licensing and provenance" below. Only this README is tracked, and every
> command here needs a sample you placed yourself or generated locally.

Samples are organised by what they cover rather than by count.

## What is here

| File | Source | Covers | Licence |
| --- | --- | --- | --- |
| `(4)LostTemple.w3m` | the local installation's `D:\Warcraft3\Maps\` | a classic TFT melee map; **MPQ header at offset 512**; `.w3e` v11, `.w3i` v18, `.w3e`, `.w3u`, `.w3d` | Blizzard's own map; **local verification only, do not redistribute** |
| `ydwe-sample-1.19.w3x` | the YDWE repository's example maps | non-ASCII names and object data extended by YDWE; `.w3i` v25 | the YDWE repository is **GPL-3.0**; **local verification only** |
| `synthetic.w3x` | **generated locally** (see below) | a **structurally valid** MPQ: encrypted tables, four storage layouts, sectors that are genuinely deflated or stored, and no `(listfile)` | generated here, no third-party rights |

## Why `synthetic.w3x` exists

This is not a nice-to-have. It is the only way to tell "the reader is wrong" apart
from "this file is unusual".

The MPQ read path has several linked stages: header discovery, table decryption,
hash lookup, name enumeration, sector decryption, decompression. Debugging
against a real map that will not open gives no clue which stage is at fault. An
archive with a known-correct answer does.

```bash
cargo run --example make_synthetic -p war3-archive -- examples/lost-temple/synthetic.w3x
```

It writes four members, one per storage layout:

| Member | Layout |
| --- | --- |
| `war3map.w3i` | one deflated sector, no sector table (`SINGLE_UNIT`) |
| `war3map.w3e` | a sector offset table plus deflated sectors (`MULTI_BLOCK`) |
| `war3map.j` | stored, single unit |
| `war3map.wts` | stored, multi-block, **no sector table** |

The sectors really are compressed, with the `0x02` mask byte and a stored size
below the data they hold — the generator carries a small DEFLATE encoder for
exactly that reason, because a stored-block stream is always *larger* than its
input and would leave the reader's compressed branch untested. It also puts the
header at offset 512 behind an `HM3W` prefix, so member offsets have to be
resolved against the archive header rather than the file.

## What `synthetic.w3x` cannot catch

The generator encrypts the tables with the same cipher the reader decrypts with,
and the two are an exact inverse pair. A mistake **shared by both** therefore
round-trips perfectly and never shows up.

That is not a hypothetical: this workspace once read every real Blizzard archive
as noise while its own generated archive read back byte for byte. The cipher and
the table keys were both wrong, and the generator reproduced both mistakes. The
guards against that class of failure are:

1. the pinned ciphertext and table-key hashes in `war3_archive::crypto`'s tests;
2. the in-memory archive built by `war3_archive::archive`'s tests, which pins
   the header offset, the table keys, the cipher and both sector branches in
   every `cargo test`;
3. running against real archives, below.

## Using the real samples

```bash
cargo run -p war3-cli -- map info "examples/lost-temple/(4)LostTemple.w3m"
cargo run -p war3-cli -- map archive "examples/lost-temple/(4)LostTemple.w3m"
cargo run -p war3-cli -- map terrain "examples/lost-temple/(4)LostTemple.w3m"
cargo run -p war3-cli -- map list "examples/lost-temple/(4)LostTemple.w3m"
```

The game's own archives work too — keep in mind they belong to Blizzard and are
read in place, never copied here:

```bash
cargo run -p war3-cli -- map archive "D:\Warcraft3\war3.mpq"
```

## What the real samples report

Measured on this machine, with the current reader:

| File | Result |
| --- | --- |
| `war3.mpq` | hash table decrypts with **22084 of 32768 slots empty**; the block table lists **10684 members in use**; `(listfile)` and `(attributes)` are present but PKWare-imploded |
| `War3xlocal.mpq` | 1133 block entries in use |
| `(4)LostTemple.w3m` | 16 members enumerated by name; `.w3i` v18, `.w3e` v11 (161x161 tile points), `war3map.j` 72697 bytes of JASS |
| `ydwe-sample-1.19.w3x` | `.w3i` v25, `.w3e` v11, `.w3u` object data, map name `YDWE的UI演示` resolved through the string table |

Extraction is byte-identical to an independent implementation of the format for
`war3map.w3i` (451 bytes), `war3map.w3e` (181524 bytes) and `war3map.j`
(72697 bytes).

## The four bugs this directory found

Reading these files exposed four defects in `war3-archive`, all of which failed
silently — every one of them produced plausible-looking output rather than an
error:

1. **The table keys were ASCII constants** `HASH` / `BLK#` (and byte-swapped on
   top of that). The format keys both tables with
   `HashString("(hash table)", MPQ_HASH_FILE_KEY)` and the same for
   `(block table)`.
2. **The cipher's key update folded in the plaintext.** The key walks its own
   sequence; anything else makes real archives decrypt into noise.
3. **Member offsets were treated as file-relative.** They are relative to the
   archive header, like the table offsets, so every member of a map came back
   shifted by the 512-byte `HM3W` prefix.
4. **Sector compression was detected by a `0xFF` byte** that the format does not
   use. A sector's first byte is a compression *mask* (`0x02` deflate, `0x08`
   PKWare implode, ...), and whether it is compressed at all is decided by
   comparing its stored size against the data it must hold.

A fifth, smaller one: an uncompressed multi-block member has **no** sector
offset table, and the reader used to parse one anyway.

`examples/fresh/README.md` describes the clean control sample that the earlier
misdiagnosis asked for. It is no longer a blocker; it is still worth keeping for
new parser work.

## Known gap: PKWare implode

Sectors with the `0x08` mask byte are imploded with the PKWare Data Compression
Library, and `war3-archive` does not implement "explode" yet. It reports this
plainly:

```text
WAR3MAP.WTS   ?  (decompression failed: unsupported compression mask 0x00000008)
```

That costs two things today: a map's `war3map.wts` (its string table, so
`TRIGSTR_nnn` references stay unresolved), and Blizzard's own `(listfile)` and
`(attributes)`, which is why `war3.mpq` enumerates only the names it can guess
rather than all 10684 members. Huffman (`0x01`), bzip2 (`0x10`) and ADPCM
(`0x40`, `0x80`) are likewise reported as unsupported; none of them occur in the
map samples, but bzip2 does occur in some community archives.

## Licensing and provenance

| Source | Handling |
| --- | --- |
| maps you built yourself | committed, used as golden files |
| community maps | need the author's permission to redistribute; by default used for local checks only |
| maps bundled with YDWE | the repository is GPL-3.0, so **local verification only** |
| Blizzard's own maps and archives | **local verification only**, never redistributed |

Every `.w3x` and `.w3m` file here is therefore excluded by `.gitignore`. To get a
sample, either place your own, or generate the synthetic archive.

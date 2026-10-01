# Clean samples

> This directory is for maps that are **known to be unmodified**. Binaries are
> excluded by `.gitignore`.

## Why a separate directory

The samples in [`../lost-temple/`](../lost-temple/README.md) have muddled
provenance: a repackaged installation, an example map from another project, and
no record of what touched them. When a new parser disagrees with one of them,
there is no way to tell whether the parser or the file is the odd one out.

So this directory holds exactly one kind of thing: **a map that the local
`World Editor.exe` created and saved itself**, with nothing else having opened
it. That is the control sample.

## How to produce one

1. Open `D:\Warcraft3\World Editor.exe`.
2. Create a new map; the default 64x64 size is fine. Simpler is better.
3. Save it straight into this directory, with an ASCII file name such as
   `fresh-64x64.w3x`.
4. Do not open it with any third-party tool afterwards.

If the World Editor will not start, a fallback: copy a map from
`Maps\FrozenThrone\` that **no third-party tool has ever opened** — for example
`(2)EchoIsles.w3x`. The stock ladder maps are clean too.

## What to run

```bash
# 1. Archive structure. The tables decrypting is immediately visible: a valid
#    hash table is mostly empty slots.
cargo run --example dump_mpq -p war3-archive -- "examples/fresh/<map>"

# 2. The Phase 1 verbs.
cargo run -p war3-cli -- map info "examples/fresh/<map>"
cargo run -p war3-cli -- map terrain "examples/fresh/<map>"
cargo run -p war3-cli -- map list "examples/fresh/<map>"
cargo run -p war3-cli -- map file "examples/fresh/<map>" war3map.w3i > w3i.bin
```

## How to read the result

Archive-level failures are no longer the open question they once were — the
reader is verified against real Blizzard archives and extracts byte-identically
to an independent implementation. What remains is parser coverage, so the useful
signal is now in the per-format output:

| Observation | Conclusion |
| --- | --- |
| tables decrypt, every member reads, no diagnostics | the control agrees with the reader |
| a member reads but a parser rejects it | the fault is in that parser; the error names the offset |
| `(listfile)` is reported as `unsupported compression mask 0x08` | expected: PKWare implode is not implemented yet |
| the tables do not decrypt at all | the reader is at fault; go back to `crypt_table`, `decrypt` and the key constants in `war3_archive::crypto` |

A map saved with its `(listfile)` suppressed is still useful: the reader falls
back to its known-names list and `war3map.imp`, which is a path worth exercising.

# Clean samples

> This directory is for maps that are **known to be unmodified**. Binaries are
> excluded by `.gitignore`.

## Why a separate directory

The samples in [`../lost-temple/`](../lost-temple/README.md) cannot be read, and
their provenance is muddled: a repackaged installation, an example map from
another project, and no record of what touched them.

To decide whether the reader or the file is at fault, there has to be a control
sample that is known to be clean. So this directory holds exactly one kind of
thing: **a map that the local `World Editor.exe` created and saved itself.**

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
# 1. Archive structure: whether the tables decrypt is immediately visible.
cargo run --example dump_mpq -p war3-mpq -- "examples/fresh/<map>"

# 2. Map parsing, the Phase 1 verb.
cargo run -p war3-cli -- map info "examples/fresh/<map>"
cargo run -p war3-cli -- map terrain "examples/fresh/<map>"
cargo run -p war3-cli -- map list "examples/fresh/<map>"
```

## How to read the result

| Observation | Conclusion |
| --- | --- |
| `named_files > 0` and every block `pos` is inside the file | the reader is fine; continue verifying `.w3i` and `.w3e` |
| `named_files` is still 0 | the reader is at fault; go back to `crypt_table` and `decrypt` and recover the keystream from known plaintext |
| `named_files > 0` but `map info` fails | the fault is in the `.w3i` parser; the error names the offset |

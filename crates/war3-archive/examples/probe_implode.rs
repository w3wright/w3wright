//! One-off probe: what does an imploded member actually look like on disk?
//!
//! Run: `cargo run --release --example probe_implode -p war3-archive -- <map> <member>`
//!
//! Kept in the tree rather than thrown away because "read the bytes before
//! trusting the decoder" is the step that turns a wrong guess into a fact, and the
//! next person to touch [`war3_archive::codec::pkware`] will want it.

use war3_archive::Archive;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let map = args
        .next()
        .unwrap_or_else(|| "D:\\Warcraft3\\Maps\\(4)LostTemple.w3m".to_string());
    let member = args.next().unwrap_or_else(|| "WAR3MAP.WTS".to_string());

    let archive = Archive::open(&map)?;
    let raw = archive.raw_member(&member)?;

    println!("member    {member}");
    println!("flags     {:#010X}", raw.flags.0);
    println!("locale    {}", raw.locale);
    println!("platform  {}", raw.platform);
    println!("block     {} bytes", raw.block.len());

    let head = raw
        .block
        .iter()
        .take(32)
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>();
    println!("first 32  {}", head.join(" "));
    let ascii: String = raw
        .block
        .iter()
        .take(32)
        .map(|&b| {
            if (32..127).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect();
    println!("as ascii  {ascii}");

    // The MPQ sector's leading byte is the compression mask; the bytes after it
    // are what the method's own decoder sees. Printing the split is the point of
    // this probe, because the two formats both start with a byte that looks like a
    // header and confusing them is easy.
    if let Some((mask, body)) = raw.block.split_first() {
        println!("mask byte {mask:#04X}");
        let body_head = body
            .iter()
            .take(24)
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>();
        println!("body      {}", body_head.join(" "));
        println!(
            "body[0] (implode compression type) = {}",
            body.first().copied().unwrap_or(255)
        );
        println!(
            "body[1] (implode dict size bits)   = {}",
            body.get(1).copied().unwrap_or(255)
        );

        let declared = raw.uncompressed_size as usize;
        println!("declared  {declared} bytes");
        match war3_archive::codec::decompress(body, u32::from(*mask), declared) {
            Ok(out) => {
                println!("decoded   {} bytes", out.len());
                let text: String = out
                    .iter()
                    .take(160)
                    .map(|&b| {
                        if (32..127).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                println!("text      {text}");
            }
            Err(e) => println!("decode    FAILED: {e}"),
        }
    }

    // Every plausible offset at which a PKWare header could begin: a compression
    // type of 0 or 1 followed by a dictionary size of 4, 5 or 6. If the first
    // attempt above failed, this says where the stream actually starts instead of
    // leaving the next reader to guess a second time.
    println!("\ncandidate PKWare headers in the block:");
    let mut found = 0;
    for i in 0..raw.block.len().saturating_sub(4) {
        let ctype = raw.block[i];
        let dict = raw.block[i + 1];
        if (ctype == 0 || ctype == 1) && (4..=6).contains(&dict) {
            println!("  offset {i:3}: type={ctype} dict_bits={dict}");
            found += 1;
            if found >= 6 {
                break;
            }
        }
    }
    if found == 0 {
        println!("  none — this member is not PKWare implode");
    } else {
        // Try the decoder directly at each candidate, which separates "the decoder
        // is wrong" from "the caller handed it the wrong bytes". The header is
        // two bytes, so the decoder wants the slice starting at the candidate.
        println!("\ndecoding from each candidate:");
        for i in 0..raw.block.len().saturating_sub(4) {
            let ctype = raw.block[i];
            let dict = raw.block[i + 1];
            if (ctype == 0 || ctype == 1) && (4..=6).contains(&dict) {
                let declared = raw.uncompressed_size as usize;
                match war3_archive::codec::pkware::explode(&raw.block[i..], declared) {
                    Ok(out) => {
                        let text: String = out
                            .iter()
                            .take(96)
                            .map(|&b| {
                                if (32..127).contains(&b) {
                                    b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect();
                        println!("  offset {i:3}: OK {} bytes  {text}", out.len());
                    }
                    Err(e) => println!("  offset {i:3}: {e}"),
                }
            }
        }
    }

    // Optional: hand the decoder's exact input to a file so a reference
    // implementation can be run on the same bytes. Comparing the two separates "the
    // bit model is wrong" from "the caller extracted the wrong bytes", which
    // reading the reference source did not settle.
    //
    // ⚠️ What the decoder is handed is **one sector**, not the whole block. A
    // compressed member may be split into sectors and then begins with a sector
    // offset table — one more entry than there are sectors — so the block's own
    // first byte is an offset, not a compression mask. Dumping the block from byte
    // zero makes a reference implementation read `00 00` as its header and stop,
    // which looks exactly like a reference implementation that disagrees with you.
    if let Ok(path) = std::env::var("W3_PKWARE_DUMP") {
        let masked = raw.flags.0 & 0x0000_0200 != 0;
        let single = raw.flags.0 & 0x0100_0000 != 0;
        let data: &[u8] = if masked && !single {
            let start = u32::from_le_bytes([raw.block[0], raw.block[1], raw.block[2], raw.block[3]])
                as usize;
            let end = u32::from_le_bytes([raw.block[4], raw.block[5], raw.block[6], raw.block[7]])
                as usize;
            println!("\nsector table: first sector {start}..{end}");
            &raw.block[start..end.min(raw.block.len())]
        } else {
            &raw.block
        };

        if let Some((mask, body)) = data.split_first() {
            std::fs::write(&path, body)?;
            println!(
                "dumped {} bytes (sector 0 implode stream, mask {mask:#04X} stripped) to {path}",
                body.len()
            );

            // The decoder's own output, for a byte-for-byte comparison against the
            // reference. This is the step that turns "the two disagree" into "they
            // first disagree at offset N, where the reference has X and we have Y".
            let out_path = format!("{path}.rust");
            match war3_archive::codec::pkware::explode_detailed(
                body,
                raw.uncompressed_size as usize,
            ) {
                Ok(outcome) => {
                    println!(
                        "rust     {} of {} bytes, error={:?}, end_marker={}",
                        outcome.out.len(),
                        outcome.expected,
                        outcome.error,
                        outcome.saw_end_marker
                    );
                    if outcome.is_complete() {
                        std::fs::write(&out_path, &outcome.out)?;
                        println!("rust     wrote {out_path}");
                    } else {
                        // The prefix is the evidence: a wrong bit model fails on the
                        // first symbol, while a local bug right for N bytes and then
                        // wrong looks nothing like it.
                        let show = outcome.out.len().min(80);
                        let text: String = outcome.out[..show]
                            .iter()
                            .map(|&b| {
                                if (32..127).contains(&b) {
                                    b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect();
                        println!("rust     prefix: {text}");
                    }
                }
                Err(e) => println!("rust     header rejected: {e}"),
            }
        }
    }

    Ok(())
}

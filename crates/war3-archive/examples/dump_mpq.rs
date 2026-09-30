//! Prints an archive's structure and reads a couple of members.
//!
//! This is the first thing to run when a map will not open: it shows whether the
//! tables decrypted into something plausible (member offsets inside the file,
//! some hash slots unused) before any format parser is involved.
//!
//! ```text
//! cargo run --example dump_mpq -p war3-archive -- <archive>
//! ```

use war3_archive::archive::{self, Archive};

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_mpq <file>");
    let archive = Archive::open(&path).expect("failed to open archive");
    let info = archive.info();
    let h = info.header;

    println!("file_offset        = {}", h.file_offset);
    println!("header_size        = {}", h.header_size);
    println!(
        "archive_size       = {} (0x{:X})",
        h.archive_size, h.archive_size
    );
    println!("format_version     = {}", h.format_version);
    println!("sector_size_shift  = {}", h.sector_size_shift);
    println!("sector_size        = {}", h.sector_size());
    println!("hash_table_pos     = 0x{:X}", h.hash_table_pos);
    println!("block_table_pos    = 0x{:X}", h.block_table_pos);
    println!("hash_table_size    = {}", h.hash_table_size);
    println!("block_table_size   = {}", h.block_table_size);
    println!("used_blocks        = {}", info.used_blocks);
    println!("named_files        = {}", archive.file_count());

    println!("\n--- block table ---");
    for i in 0..h.block_table_size as usize {
        if let Some(b) = archive.block_entry(i) {
            println!(
                "[{i:2}] pos=0x{:08X} comp={:8} uncomp={:8} method-bits=0x{:03X} method={:<34} flags={}",
                b.file_pos,
                b.compressed_size,
                b.uncompressed_size,
                b.compression_method_bits(),
                b.compression_name(),
                b.flags
            );
        }
    }

    println!("\n--- enumerated names ---");
    for name in archive.file_names() {
        println!("  {name}");
    }

    println!("\n--- diagnostics ---");
    for d in archive.diagnostics().items() {
        println!("  {d}");
    }

    for probe in ["war3map.w3i", "war3map.w3e"] {
        println!("\n--- {probe}, first 32 bytes ---");
        match archive.read_file(probe) {
            Ok(bytes) => {
                println!("  len = {}", bytes.len());
                println!("  {:02X?}", &bytes[..bytes.len().min(32)]);
            }
            Err(e) => println!("  failed: {e}"),
        }
    }

    let _ = archive::MPQ_MAGIC_LEN;
}

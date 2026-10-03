//! Verifies ADR-0028's acceptance criterion 2: a map patched **in place** is still readable by the
//! game.
//!
//! Criterion 1 and 3 already have tests in `war3-archive` (zero diff outside the block table, an
//! untouched member reads back identical). Criterion 2 is the one that cannot be a test: it needs the
//! game. This program therefore does the patching half and prints what a human should then look for.
//!
//! What it changes, and why this field:
//!
//! `hC06` is a custom unit on `(6)BlizzardTD.w3x` — a builder costing 10 gold, based on `hpea`. Its
//! name comes from `TRIGSTR_117`, which the map's `war3map.wts` resolves to `守卫 (基本建造者)`.
//! Replacing that value with **literal text** (`W3WRIGHT PATCH TEST`) makes the change visible in the
//! game with no other edit: the builder's name is shown in the build menu.
//!
//! ⚠️ It patches the **bytes of `war3map.w3u`** and hands them to `ArchivePatcher::replace`, which is
//! the real save path. It does not go through `extract → edit → rebuild`, which is the path
//! ADR-0021 already verified — conflating the two is exactly the mistake the docs warn about.
//!
//! # Usage
//!
//! ```text
//! cargo run -p war3-cli --example patch_for_game -- <in.w3x> <out.w3x>
//! ```
fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("usage: <in.w3x> <out.w3x>");
    let output = args.next().expect("usage: <in.w3x> <out.w3x>");

    // ---- read the map's own object file ----
    let archive = war3_archive::Archive::open(&input).expect("the map opens");
    let bytes = archive
        .read_file("war3map.w3u")
        .expect("the map has war3map.w3u");

    let mut file = war3_object::ObjectFile::parse(war3_object::ObjectKind::Unit, &bytes)
        .expect("war3map.w3u parses");

    // ---- change exactly one field of exactly one object ----
    let mut changed = 0;
    for object in &mut file.table.custom {
        if object.id.to_string().eq_ignore_ascii_case("hC06") {
            for modification in &mut object.modifications {
                if modification.field.to_string().eq_ignore_ascii_case("unam") {
                    let before = modification.value.to_string();
                    // Literal text, not a TRIGSTR reference: the game then shows this string with no
                    // string-table hop, so a wrong resolution cannot be mistaken for a failed patch.
                    modification.value =
                        war3_object::FieldValue::String("W3WRIGHT PATCH TEST".into());
                    println!("hC06 unam: {before} -> W3WRIGHT PATCH TEST");
                    changed += 1;
                }
            }
        }
    }
    assert_eq!(changed, 1, "expected to change exactly one field");

    // ---- write it through the real save path ----
    let patched_bytes = file.to_bytes();
    let mut patcher =
        war3_archive::ArchivePatcher::new(std::fs::read(&input).expect("the map reads"))
            .expect("the archive opens for patching");
    patcher
        .replace("war3map.w3u", patched_bytes)
        .expect("war3map.w3u is writable");
    std::fs::write(&output, patcher.to_bytes()).expect("the patched map writes");

    println!("patched {} member(s) -> {output}", patcher.patched_count());

    // ---- read it back, because "the game can read it" starts with "we can" ----
    let check = war3_archive::Archive::open(&output).expect("the patched map opens");
    let read_back = check
        .read_file("war3map.w3u")
        .expect("war3map.w3u is still there");
    let reread = war3_object::ObjectFile::parse(war3_object::ObjectKind::Unit, &read_back)
        .expect("war3map.w3u still parses");
    let name = reread
        .table
        .custom
        .iter()
        .find(|o| o.id.to_string().eq_ignore_ascii_case("hC06"))
        .and_then(|o| {
            o.modifications
                .iter()
                .find(|m| m.field.to_string().eq_ignore_ascii_case("unam"))
        })
        .map(|m| m.value.to_string());
    println!("read back: hC06 unam = {name:?}");
    assert_eq!(
        name.as_deref(),
        Some("W3WRIGHT PATCH TEST"),
        "the patch must survive a round trip through the archive"
    );

    // The other members must be untouched: a rebuild that also rewrote them would pass the check
    // above while having changed more than asked.
    let original = war3_archive::Archive::open(&input).expect("the original opens");
    let mut differing = Vec::new();
    for member in original.file_names() {
        if member.eq_ignore_ascii_case("war3map.w3u") {
            continue;
        }
        let a = original.read_file(member);
        let b = check.read_file(member);
        if a.ok() != b.ok() {
            differing.push(format!("{member}: one reads and the other does not"));
        }
    }
    assert!(
        differing.is_empty(),
        "members changed besides war3map.w3u: {differing:?}"
    );
    println!("every other member still reads, so only war3map.w3u changed");
    println!();
    println!("NOW CHECK IN THE GAME: build the builder (10 gold) and read its name.");
    println!("It should say W3WRIGHT PATCH TEST. If the map fails to load at all, that is the");
    println!("answer to ADR-0028 criterion 2 and the in-place path needs rethinking.");
}

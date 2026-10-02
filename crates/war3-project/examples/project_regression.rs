//! Runs the whole source-project round trip over a directory of maps.
//!
//! ```text
//! cargo run --release --example project_regression -p war3-project -- "D:\Warcraft3\Maps"
//! ```
//!
//! For every map it finds: extract it to a temporary project, build it back, and
//! check that every member came back identical. This is the measurement the
//! prototype is judged by (see `docs/02-Core与数据格式设计.md` §16), and it is the
//! one number that cannot be faked by a unit test: it is every real map on this
//! machine, at once.
//!
//! # What the totals mean
//!
//! - `round trip ok` — `build` reported no mismatch for that map.
//! - `refused` — `extract` declined, always with a reason. A refusal is the
//!   designed outcome for a map whose blocks cannot be named; it is reported, not
//!   hidden, and it never produces a wrong file.
//! - `members textified` / `binary` / `raw` — where the members of the maps that
//!   *did* work ended up. `textified` is the interesting one: it only counts
//!   members whose text form reproduced them byte for byte.
//!
//! Real maps are not in the repository (they carry their own licences), so this
//! cannot be a test; it is a command you run against your own map folder.

use std::path::{Path, PathBuf};

use war3_project::{build, extract};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let roots: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if roots.is_empty() {
        eprintln!("usage: project_regression <map-or-directory>...");
        std::process::exit(2);
    }

    let mut maps = Vec::new();
    for root in &roots {
        collect(root, &mut maps);
    }
    maps.sort();

    let (mut rebuilt, mut refused) = (0usize, 0usize);
    let (mut textified, mut binary, mut raw) = (0usize, 0usize, 0usize);
    // Kept apart on purpose: a refusal is a designed outcome with a reason, not a
    // failure, and lumping the two together would make the exit code lie.
    let mut refusals = Vec::new();
    let mut problems = Vec::new();

    for (index, map) in maps.iter().enumerate() {
        let work = TempDir::new("project-regression")?;
        let project = work.path().join("project");
        let out = work.path().join("rebuilt.w3x");

        let report = match extract(map, &project, false) {
            Ok(report) => report,
            Err(e) => {
                refused += 1;
                refusals.push(format!("REFUSED  {}: {e}", file_name(map)));
                continue;
            }
        };
        textified += report.text;
        binary += report.binary;
        raw += report.raw;

        match build(&project, Some(&out)) {
            Ok(built) if built.mismatched.is_empty() => rebuilt += 1,
            Ok(built) => problems.push(format!(
                "MISMATCH {}: {}",
                file_name(map),
                built.mismatched.join("; ")
            )),
            Err(e) => problems.push(format!("FAILED   {}: {e}", file_name(map))),
        }

        if index % 25 == 24 {
            println!("  ... {}/{}, {rebuilt} rebuilt", index + 1, maps.len());
        }
    }

    println!("maps walked        : {}", maps.len());
    println!("round trip ok      : {rebuilt}");
    println!("refused            : {refused}  (designed: a map is refused, never half-written)");
    println!("failed             : {}", problems.len());
    println!("members textified  : {textified}");
    println!("members binary     : {binary}");
    println!("members raw        : {raw}");
    for line in refusals.iter().chain(&problems) {
        println!("  {line}");
    }

    if !problems.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                collect(&entry.path(), out);
            }
        }
        return;
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if matches!(extension.as_deref(), Some("w3x" | "w3m")) {
        out.push(path.to_path_buf());
    }
}

/// A directory that removes itself, so a run leaves no scratch behind.
///
/// The workspace has no external dependencies, so this is twenty lines instead of
/// `tempfile` — and a released example is the wrong place to add one.
#[derive(Debug)]
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> std::io::Result<Self> {
        for attempt in 0..1000u32 {
            let path = std::env::temp_dir()
                .join(format!("w3wright-{label}-{}-{attempt}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "no free temporary directory name",
        ))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

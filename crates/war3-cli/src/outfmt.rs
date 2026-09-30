//! Output formatting helpers.
//!
//! Two rules shape the output: unknown values print as `?` rather than `0`, since
//! a zero reads as real data; and fields whose meaning is undetermined are still
//! printed, labelled as such.

use war3_core::{Diagnostics, Severity};

/// One indentation level.
pub const INDENT: &str = "  ";

/// Prints an aligned `label: value` line.
pub fn field(label: &str, value: impl std::fmt::Display, width: usize) {
    println!("{INDENT}{label:<width$} {value}");
}

/// Renders an optional value, printing `?` for `None`.
pub fn opt<T: std::fmt::Display>(value: Option<T>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "?".to_string(),
    }
}

/// Prints a section heading.
pub fn section(title: &str) {
    println!();
    println!("{title}");
    println!("{}", "-".repeat(title.chars().count().max(8)));
}

/// Renders a boolean as `yes` or `no`.
pub fn yesno(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}

/// Prints diagnostics.
///
/// Returns whether anything at `Warning` or above was present, so the caller can
/// pick an exit code.
pub fn diagnostics(diags: &Diagnostics, verbose: bool) -> bool {
    let items: Vec<_> = if verbose {
        diags.items().iter().collect()
    } else {
        diags.at_least(Severity::Warning).collect()
    };

    if items.is_empty() {
        return false;
    }

    println!();
    println!("diagnostics ({}):", items.len());
    for d in &items {
        println!("{INDENT}[{}] {} - {}", d.severity, d.code, d.message);
    }
    if !verbose && diags.len() > items.len() {
        println!(
            "{INDENT}({} info-level entries hidden; pass --verbose to show them)",
            diags.len() - items.len()
        );
    }
    diags.has_problems()
}

/// Renders a byte count with a readable unit.
pub fn bytes(n: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Renders a four-part version as `A.B.C.D`.
pub fn version(a: [i32; 4]) -> String {
    format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use war3_core::{Diagnostic, DiagnosticCode};

    #[test]
    fn bytes_picks_a_readable_unit() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(2048), "2.0 KB");
        assert_eq!(bytes(1024 * 1024 * 3 / 2), "1.5 MB");
    }

    #[test]
    fn version_joins_with_dots() {
        assert_eq!(version([1, 31, 1, 12173]), "1.31.1.12173");
    }

    #[test]
    fn opt_prints_question_mark_for_none() {
        assert_eq!(opt(None::<i32>), "?");
        assert_eq!(opt(Some(5)), "5");
    }

    #[test]
    fn yesno_is_stable() {
        assert_eq!(yesno(true), "yes");
        assert_eq!(yesno(false), "no");
    }

    #[test]
    fn info_only_diagnostics_are_not_a_problem() {
        let mut d = Diagnostics::new();
        d.push(Diagnostic::info(DiagnosticCode::MpqHeaderOffset, "at 512"));
        assert!(!diagnostics(&d, false));
    }

    #[test]
    fn warning_diagnostics_are_a_problem() {
        let mut d = Diagnostics::new();
        d.push(Diagnostic::warn(DiagnosticCode::W3eRecordSizeMismatch, "11 vs 12"));
        assert!(diagnostics(&d, false));
    }
}

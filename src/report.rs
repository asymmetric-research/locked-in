use crate::{Severity, Violation};
use colored::Colorize;
use std::path::Path;

pub fn print_violations(path: &Path, violations: &[Violation]) {
    let has_errors = violations
        .iter()
        .any(|violation| violation.severity == Severity::Error);
    let heading = if has_errors {
        format!("✗ {}", path.display()).red()
    } else {
        format!("! {}", path.display()).yellow()
    };

    println!("{heading}");
    for violation in violations {
        let label = match violation.severity {
            Severity::Error => "Error",
            Severity::Warning => "Warning",
        };
        let location = if violation.line_num == 0 {
            label.to_string()
        } else {
            format!("{label} line {}", violation.line_num)
        };
        println!(
            "  {} {}",
            format!("{location}:").yellow(),
            violation.message
        );
        println!("  {} {}", ">".blue(), violation.line_content);
    }
    println!();
}

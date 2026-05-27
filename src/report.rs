use crate::Violation;
use colored::Colorize;
use std::path::Path;

pub fn print_violations(path: &Path, violations: &[Violation]) {
    println!("{}", format!("✗ {}", path.display()).red());
    for violation in violations {
        println!(
            "  {} {}",
            format!("Line {}:", violation.line_num).yellow(),
            violation.message
        );
        println!("  {} {}", ">".blue(), violation.line_content);
    }
    println!();
}

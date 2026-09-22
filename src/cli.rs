use crate::scanner::{LintOptions, lint_files_with_options};
use clap::Parser;
use colored::Colorize;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    about = "Lints package-manager commands for lockfile and version-pin safety.",
    version
)]
struct Cli {
    /// Repository path to scan.
    #[arg(value_name = "PATH")]
    root: Option<PathBuf>,

    /// Do not validate npm lockfile integrity metadata.
    #[arg(long)]
    skip_npm_integrity_check: bool,
}

pub fn run<I, S>(args: I) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = error.exit_code();
            let _ = error.print();
            return exit_code;
        }
    };
    let root = cli.root.unwrap_or_else(|| PathBuf::from("."));
    let options = LintOptions {
        check_npm_integrity: !cli.skip_npm_integrity_check,
    };

    println!("{}", "Checking for package manager violations...\n".blue());

    let result = lint_files_with_options(&root, options);

    println!();
    println!("{}", "═══════════════════════════════════════".blue());

    if result.violations_found == 0 {
        println!("{}", "✓ No violations found!".green());
        if result.warnings_found > 0 {
            println!(
                "{}",
                format!("Warnings: {}", result.warnings_found).yellow()
            );
        }
        println!(
            "{}",
            format!("Files checked: {}", result.files_checked).blue()
        );
        0
    } else {
        println!(
            "{}",
            format!(
                "✗ Found {} violation(s) and {} warning(s) in {} files",
                result.violations_found, result.warnings_found, result.files_checked
            )
            .red()
        );
        println!(
            "{}",
            "Tip: use lockfiles/version pins plus dependency cooldowns (minimum release age) for defense in depth.".blue()
        );
        println!();
        1
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, run};
    use clap::Parser;

    #[test]
    fn help_exits_without_scanning() {
        assert_eq!(run(["locked-in", "--help"]), 0);
    }

    #[test]
    fn version_exits_without_scanning() {
        assert_eq!(run(["locked-in", "--version"]), 0);
    }

    #[test]
    fn unknown_option_returns_usage_error() {
        assert_eq!(run(["locked-in", "--json"]), 2);
    }

    #[test]
    fn skip_npm_integrity_check_is_parsed() {
        let cli = Cli::try_parse_from(["locked-in", "--skip-npm-integrity-check"]).unwrap();

        assert!(cli.skip_npm_integrity_check);
    }
}

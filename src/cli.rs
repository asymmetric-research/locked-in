use crate::lint_files;
use colored::Colorize;
use std::path::PathBuf;

const USAGE: &str = "Usage: locked-in [PATH]\n\nLints package-manager commands for lockfile and version-pin safety.\n\nArguments:\n  PATH        Repository path to scan (default: current directory)\n\nOptions:\n  -h, --help     Show this help text\n  -V, --version  Show version\n";

pub fn run<I, S>(args: I) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    let _program = args.next();
    let mut root: Option<PathBuf> = None;

    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return 0;
            }
            "-V" | "--version" => {
                println!("locked-in {}", env!("CARGO_PKG_VERSION"));
                return 0;
            }
            _ if arg.starts_with('-') => {
                eprintln!("Unknown option: {arg}\n\n{USAGE}");
                return 2;
            }
            _ if root.is_some() => {
                eprintln!("Unexpected extra argument: {arg}\n\n{USAGE}");
                return 2;
            }
            _ => root = Some(PathBuf::from(arg)),
        }
    }

    let root = root.unwrap_or_else(|| PathBuf::from("."));

    println!("{}", "Checking for package manager violations...\n".blue());

    let result = lint_files(&root);

    println!();
    println!("{}", "═══════════════════════════════════════".blue());

    if result.violations_found == 0 {
        println!("{}", "✓ No violations found!".green());
        println!(
            "{}",
            format!("Files checked: {}", result.files_checked).blue()
        );
        0
    } else {
        println!(
            "{}",
            format!(
                "✗ Found {} violation(s) in {} files",
                result.violations_found, result.files_checked
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
    use super::run;

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
}

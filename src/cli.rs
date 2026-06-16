use crate::lint_files;
use crate::report::{render_json, render_text};
use crate::{EXIT_FINDINGS, EXIT_SUCCESS, EXIT_USAGE, LintResult, output};
use colored::Colorize;
use std::io::{self, Write};
use std::path::PathBuf;

const USAGE: &str = "Usage: locked-in [OPTIONS] [PATH]\n\nLints package-manager commands for lockfile and version-pin safety.\n\nArguments:\n  PATH        Repository path to scan (default: current directory)\n\nOptions:\n  -q, --quiet            Suppress warning lines from output; errors and summary still print\n                         (e.g. locked-in --quiet .)\n      --max-warnings <N>  Exit non-zero if warning count exceeds N; default: unlimited\n                         (e.g. locked-in --max-warnings 0 .)\n      --fail-on <level>   Exit non-zero threshold: 'error' (default) or 'warning';\n                         '--fail-on warning' is equivalent to '--max-warnings 0'\n                         (e.g. locked-in --fail-on warning .)\n      --format <fmt>     Output format: 'text' (default) or 'json'\n                         (e.g. locked-in --format json .)\n      --no-summary       Skip the trailing summary block (text format only)\n                         (e.g. locked-in --no-summary .)\n  -h, --help             Show this help text\n  -V, --version          Show version\n\nFindings are written to stdout; progress and the summary are written to stderr.";

const DIVIDER: &str = "═══════════════════════════════════════";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailLevel {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

struct Options {
    root: PathBuf,
    quiet: bool,
    no_summary: bool,
    max_warnings: Option<usize>,
    fail_on: FailLevel,
    format: OutputFormat,
}

/// Resolve the effective warning threshold from `--max-warnings` and `--fail-on`.
/// `--fail-on warning` is equivalent to `--max-warnings 0`; when both are present the
/// strictest (lowest) threshold wins.
fn effective_max_warnings(max_warnings: Option<usize>, fail_on: FailLevel) -> Option<usize> {
    let fail_on_threshold = match fail_on {
        FailLevel::Warning => Some(0),
        FailLevel::Error => None,
    };
    [max_warnings, fail_on_threshold].into_iter().flatten().min()
}

/// Compute the process exit code. Errors always fail; warnings fail only when they exceed
/// the effective threshold.
fn exit_code(errors: usize, warnings: usize, max_warnings: Option<usize>) -> i32 {
    if errors > 0 || max_warnings.is_some_and(|max| warnings > max) {
        EXIT_FINDINGS
    } else {
        EXIT_SUCCESS
    }
}

/// Pull the value for a value-taking flag, supporting both `--flag value` and `--flag=value`.
/// Returns `Err(EXIT_USAGE)` (after printing) when the value is missing.
fn take_value(
    arg: &str,
    flag: &str,
    args: &mut impl Iterator<Item = String>,
) -> Result<String, i32> {
    if let Some(inline) = arg.strip_prefix(&format!("{flag}=")) {
        return Ok(inline.to_string());
    }
    let Some(value) = args.next() else {
        output::err_line(format!("Missing value for {flag}\n\n{USAGE}"));
        return Err(EXIT_USAGE);
    };
    Ok(value)
}

/// Parse CLI arguments into `Options`. `Err(code)` means "stop and exit with this code" —
/// `EXIT_SUCCESS` for `--help`/`--version` (already printed), `EXIT_USAGE` for bad input.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Options, i32> {
    let mut root: Option<PathBuf> = None;
    let mut quiet = false;
    let mut no_summary = false;
    let mut max_warnings: Option<usize> = None;
    let mut fail_on = FailLevel::Error;
    let mut format = OutputFormat::Text;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                output::out_line(USAGE);
                return Err(EXIT_SUCCESS);
            }
            "-V" | "--version" => {
                output::out_line(format!("locked-in {}", env!("CARGO_PKG_VERSION")));
                return Err(EXIT_SUCCESS);
            }
            "-q" | "--quiet" => quiet = true,
            "--no-summary" => no_summary = true,
            _ if arg == "--max-warnings" || arg.starts_with("--max-warnings=") => {
                let value = take_value(&arg, "--max-warnings", &mut args)?;
                let Ok(parsed) = value.parse::<usize>() else {
                    output::err_line(format!("Invalid value for --max-warnings: {value}\n\n{USAGE}"));
                    return Err(EXIT_USAGE);
                };
                max_warnings = Some(parsed);
            }
            _ if arg == "--fail-on" || arg.starts_with("--fail-on=") => {
                let value = take_value(&arg, "--fail-on", &mut args)?;
                fail_on = match value.as_str() {
                    "error" => FailLevel::Error,
                    "warning" => FailLevel::Warning,
                    _ => {
                        output::err_line(format!(
                            "Invalid value for --fail-on: {value} (expected 'error' or 'warning')\n\n{USAGE}"
                        ));
                        return Err(EXIT_USAGE);
                    }
                };
            }
            _ if arg == "--format" || arg.starts_with("--format=") => {
                let value = take_value(&arg, "--format", &mut args)?;
                format = match value.as_str() {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    _ => {
                        output::err_line(format!(
                            "Invalid value for --format: {value} (expected 'text' or 'json')\n\n{USAGE}"
                        ));
                        return Err(EXIT_USAGE);
                    }
                };
            }
            _ if arg.starts_with('-') => {
                output::err_line(format!("Unknown option: {arg}\n\n{USAGE}"));
                return Err(EXIT_USAGE);
            }
            _ if root.is_some() => {
                output::err_line(format!("Unexpected extra argument: {arg}\n\n{USAGE}"));
                return Err(EXIT_USAGE);
            }
            _ => root = Some(PathBuf::from(arg)),
        }
    }

    Ok(Options {
        root: root.unwrap_or_else(|| PathBuf::from(".")),
        quiet,
        no_summary,
        max_warnings,
        fail_on,
        format,
    })
}

pub fn run<I, S>(args: I) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    let _program = args.next();

    let options = match parse_args(args) {
        Ok(options) => options,
        Err(code) => return code,
    };

    // Progress chrome goes to stderr so stdout carries only findings.
    if options.format == OutputFormat::Text {
        output::err_line("Checking for package manager violations...\n".blue());
    }

    let result = lint_files(&options.root);
    let max = effective_max_warnings(options.max_warnings, options.fail_on);
    let code = exit_code(result.violations_found, result.warnings_found, max);

    // Buffer findings so large output is a few syscalls, not one per line.
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    match options.format {
        OutputFormat::Json => {
            output::guard(render_json(&mut out, &result, options.quiet).and_then(|()| out.flush()));
        }
        OutputFormat::Text => {
            output::guard(
                render_text(&mut out, &result.results, options.quiet).and_then(|()| out.flush()),
            );
            if !options.no_summary {
                print_summary(&result, max);
            }
        }
    }

    code
}

fn print_summary(result: &LintResult, max_warnings: Option<usize>) {
    output::err_line("");
    output::err_line(DIVIDER.blue());

    if exit_code(result.violations_found, result.warnings_found, max_warnings) == EXIT_SUCCESS {
        output::err_line("✓ No violations found!".green());
        if result.warnings_found > 0 {
            output::err_line(format!("Warnings: {}", result.warnings_found).yellow());
        }
        output::err_line(format!("Files checked: {}", result.files_checked).blue());
        return;
    }

    let suffix = if result.violations_found == 0 {
        format!(" (warnings exceed --max-warnings {})", max_warnings.unwrap_or(0))
    } else {
        String::new()
    };
    output::err_line(
        format!(
            "✗ Found {} violation(s) and {} warning(s) in {} files{suffix}",
            result.violations_found, result.warnings_found, result.files_checked
        )
        .red(),
    );
    output::err_line(
        "Tip: use lockfiles/version pins plus dependency cooldowns (minimum release age) for defense in depth.".blue(),
    );
    output::err_line("");
}

#[cfg(test)]
mod tests {
    use super::{FailLevel, effective_max_warnings, exit_code, run};

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
    fn invalid_flag_values_return_usage_error() {
        assert_eq!(run(["locked-in", "--max-warnings", "nope"]), 2);
        assert_eq!(run(["locked-in", "--max-warnings"]), 2);
        assert_eq!(run(["locked-in", "--fail-on", "critical"]), 2);
        assert_eq!(run(["locked-in", "--fail-on"]), 2);
        assert_eq!(run(["locked-in", "--format", "yaml"]), 2);
        assert_eq!(run(["locked-in", "--format"]), 2);
    }

    #[test]
    fn errors_always_fail_regardless_of_threshold() {
        assert_eq!(exit_code(1, 0, None), 1);
        assert_eq!(exit_code(3, 5, Some(10)), 1);
    }

    #[test]
    fn warnings_fail_only_above_threshold() {
        assert_eq!(exit_code(0, 0, None), 0);
        assert_eq!(exit_code(0, 5, None), 0); // unlimited: warnings never fail
        assert_eq!(exit_code(0, 0, Some(0)), 0); // zero warnings, threshold 0
        assert_eq!(exit_code(0, 1, Some(0)), 1); // any warning fails at 0
        assert_eq!(exit_code(0, 3, Some(3)), 0); // equal to threshold is allowed
        assert_eq!(exit_code(0, 4, Some(3)), 1); // exceeds threshold
    }

    #[test]
    fn fail_on_warning_is_equivalent_to_max_warnings_zero() {
        assert_eq!(effective_max_warnings(None, FailLevel::Warning), Some(0));
        assert_eq!(effective_max_warnings(None, FailLevel::Error), None);
        assert_eq!(effective_max_warnings(Some(5), FailLevel::Error), Some(5));
    }

    #[test]
    fn strictest_threshold_wins_when_both_set() {
        // --fail-on warning (0) is stricter than --max-warnings 5
        assert_eq!(effective_max_warnings(Some(5), FailLevel::Warning), Some(0));
        // --max-warnings 0 with --fail-on error stays 0
        assert_eq!(effective_max_warnings(Some(0), FailLevel::Error), Some(0));
    }
}

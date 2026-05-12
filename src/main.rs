use colored::Colorize;
use ignore::WalkBuilder;
use rayon::prelude::*;
use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{LazyLock, Mutex};

static PACKAGE_MANAGER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(pnpm|yarn|bun)\b").unwrap());
static NPM_CI_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bnpm\s+ci\b").unwrap());
static NPM_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bnpm\s+(install|i)(\s|$)").unwrap());
static VERSION_PIN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@[0-9]+\.[0-9]+").unwrap());
static BARE_NPM_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bnpm\s+(install|i)(\s+)?($|&&|;|\||#)").unwrap());
static PNPM_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bpnpm\s+install\b").unwrap());
static PNPM_ADD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bpnpm\s+add\s").unwrap());
static YARN_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\byarn(\s+install)?(\s+)?($|&&|;|\||#)").unwrap());
static YARN_FROZEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"--(frozen-lockfile|immutable)").unwrap());
static YARN_ADD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\byarn\s+(global\s+)?add\s").unwrap());
static BUN_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bbun\s+install\b").unwrap());
static BUN_ADD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bbun\s+add\s").unwrap());

#[derive(Debug)]
struct Violation {
    line_num: usize,
    message: String,
    line_content: String,
}

struct LintResult {
    violations_found: usize,
    files_checked: usize,
}

struct FileLintResult {
    path: PathBuf,
    violations: Vec<Violation>,
}

struct LintContext {
    bun_frozen_lockfile: bool,
    is_markdown: bool,
}

fn main() {
    let root = env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    println!(
        "{}",
        "Checking for JS package manager violations...\n".blue()
    );

    let result = lint_files(&root);

    println!();
    println!("{}", "═══════════════════════════════════════".blue());

    if result.violations_found == 0 {
        println!("{}", "✓ No violations found!".green());
        println!(
            "{}",
            format!("Files checked: {}", result.files_checked).blue()
        );
        process::exit(0);
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
        process::exit(1);
    }
}

fn lint_files(root: &Path) -> LintResult {
    let files_to_check: Vec<PathBuf> = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .filter_entry(|e| !is_excluded(e.path()))
        .build()
        .filter_map(Result::ok)
        .map(ignore::DirEntry::into_path)
        .filter(|path| path.is_file() && should_check_file(path))
        .collect();

    let bun_context_cache: Mutex<HashMap<PathBuf, bool>> = Mutex::new(HashMap::new());

    let mut checked_results: Vec<FileLintResult> = files_to_check
        .par_iter()
        .filter_map(|path| lint_file(root, path, &bun_context_cache))
        .collect();

    checked_results.sort_by(|a, b| a.path.cmp(&b.path));

    let mut violations_found: usize = 0;

    for result in &checked_results {
        if !result.violations.is_empty() {
            print_violations(&result.path, &result.violations);
            violations_found = violations_found.saturating_add(result.violations.len());
        }
    }

    LintResult {
        violations_found,
        files_checked: checked_results.len(),
    }
}

fn lint_file(
    root: &Path,
    path: &Path,
    bun_context_cache: &Mutex<HashMap<PathBuf, bool>>,
) -> Option<FileLintResult> {
    let source = fs::read_to_string(path).ok()?;
    let context = LintContext {
        bun_frozen_lockfile: bun_frozen_lockfile_enabled(root, path, bun_context_cache),
        is_markdown: has_extension(path, "md"),
    };

    Some(FileLintResult {
        path: path.to_path_buf(),
        violations: check_file(&source, &context),
    })
}

fn is_excluded(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str();
        name == OsStr::new("node_modules")
            || name == OsStr::new(".git")
            || name == OsStr::new("target")
            || name == OsStr::new("dist")
            || name == OsStr::new("build")
            || name == OsStr::new("coverage")
            || name == OsStr::new("vendor")
            || name == OsStr::new(".next")
            || name == OsStr::new(".nuxt")
            || name == OsStr::new(".turbo")
            || name == OsStr::new(".cache")
    }) || path.file_name() == Some(OsStr::new("lint-package-install.sh"))
}

fn should_check_file(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let path_str = path.to_string_lossy();

    if is_makefile(file_name) {
        return true;
    }

    // Check Dockerfiles
    if file_name.starts_with("Dockerfile") || file_name.ends_with(".dockerfile") {
        return true;
    }

    // Check markdown files
    if has_extension(path, "md") {
        return true;
    }

    // Check shell scripts and common shell-specific files
    if is_shell_file(path) {
        return true;
    }

    // Check GitHub workflow files
    let is_yaml = has_extension(path, "yml") || has_extension(path, "yaml");

    if is_yaml && path_str.contains(".github/workflows") {
        return true;
    }

    false
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

fn is_shell_file(path: &Path) -> bool {
    ["sh", "bash", "zsh", "fish", "ksh", "csh"]
        .iter()
        .any(|extension| has_extension(path, extension))
}

fn is_makefile(file_name: &str) -> bool {
    file_name == "Makefile"
        || file_name == "makefile"
        || file_name == "GNUmakefile"
        || has_extension(Path::new(file_name), "mk")
}

fn bun_frozen_lockfile_enabled(
    root: &Path,
    path: &Path,
    cache: &Mutex<HashMap<PathBuf, bool>>,
) -> bool {
    for ancestor in path.ancestors().filter(|ancestor| ancestor.is_dir()) {
        if !ancestor.starts_with(root) {
            continue;
        }

        let bunfig_path = ancestor.join("bunfig.toml");
        if !bunfig_path.is_file() {
            continue;
        }

        if let Some(enabled) = cache
            .lock()
            .expect("bun cache mutex poisoned")
            .get(&bunfig_path)
        {
            return *enabled;
        }

        let enabled = fs::read_to_string(&bunfig_path)
            .is_ok_and(|content| bunfig_has_frozen_lockfile(&content));
        cache
            .lock()
            .expect("bun cache mutex poisoned")
            .insert(bunfig_path, enabled);
        return enabled;
    }

    false
}

fn bunfig_has_frozen_lockfile(content: &str) -> bool {
    let mut in_install_section = false;

    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            in_install_section = line == "[install]";
            continue;
        }

        if !in_install_section {
            continue;
        }

        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "frozenLockfile"
        {
            return value.trim() == "true";
        }
    }

    false
}

fn check_file(content: &str, lint_context: &LintContext) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut in_code_block = false;
    let mut lint_code_block = false;

    for (line_num, line) in content.lines().enumerate() {
        let line_num = line_num.saturating_add(1); // 1-indexed

        // Track code blocks in markdown
        if lint_context.is_markdown && line.trim().starts_with("```") {
            if in_code_block {
                lint_code_block = false;
            } else {
                lint_code_block = should_lint_markdown_code_block(line);
            }
            in_code_block = !in_code_block;
            continue;
        }

        // Skip non-shell markdown code blocks
        if in_code_block && !lint_code_block {
            continue;
        }

        // Skip comments and placeholders
        if is_comment_or_placeholder(line) {
            continue;
        }

        // Check all package managers
        violations.extend(check_npm(line, line_num));
        violations.extend(check_pnpm(line, line_num));
        violations.extend(check_yarn(line, line_num));
        violations.extend(check_bun(line, line_num, lint_context.bun_frozen_lockfile));
    }

    violations
}

fn is_comment_or_placeholder(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('#')
        || trimmed.contains("<package>")
        || trimmed.contains("<version>")
        || trimmed.starts_with('`')  // Skip inline code
        || trimmed.starts_with('>')  // Skip quoted text/blockquotes
        || trimmed.starts_with('-') // Skip markdown list items that are examples
}

fn should_lint_markdown_code_block(fence_line: &str) -> bool {
    let info = fence_line.trim().trim_start_matches("```").trim();
    if info.is_empty() {
        return false;
    }

    let language = info.split_whitespace().next().unwrap_or("");
    matches!(
        language,
        "bash" | "sh" | "shell" | "zsh" | "console" | "terminal"
    )
}

fn check_npm(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Skip if it's pnpm, yarn, or bun (not npm)
    if PACKAGE_MANAGER_RE.is_match(line) {
        return violations;
    }

    // NPM CI is always allowed
    if NPM_CI_RE.is_match(line) {
        return violations;
    }

    // Check for npm install or npm i
    if NPM_INSTALL_RE.is_match(line) {
        // Check if it has a version pin
        if VERSION_PIN_RE.is_match(line) {
            return violations; // Has version pin, allowed
        }

        // Check if it's bare 'npm install' (should use npm ci)
        if BARE_NPM_INSTALL_RE.is_match(line) {
            violations.push(Violation {
                line_num,
                message: "Use 'npm ci' instead of 'npm install' for lockfile-based installations"
                    .to_string(),
                line_content: line.trim().to_string(),
            });
        } else {
            violations.push(Violation {
                line_num,
                message:
                    "npm package installation without version pin (use 'npm i package@version')"
                        .to_string(),
                line_content: line.trim().to_string(),
            });
        }
    }

    violations
}

fn check_pnpm(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Check for pnpm install without --frozen-lockfile
    if PNPM_INSTALL_RE.is_match(line) && !line.contains("--frozen-lockfile") {
        violations.push(Violation {
            line_num,
            message: "Use 'pnpm install --frozen-lockfile' to respect lockfile".to_string(),
            line_content: line.trim().to_string(),
        });
    }

    // Check for pnpm add without version
    if PNPM_ADD_RE.is_match(line) && !VERSION_PIN_RE.is_match(line) {
        violations.push(Violation {
            line_num,
            message:
                "pnpm package installation without version pin (use 'pnpm add package@version')"
                    .to_string(),
            line_content: line.trim().to_string(),
        });
    }

    violations
}

fn check_yarn(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Check for yarn install or bare yarn without --frozen-lockfile or --immutable
    if YARN_INSTALL_RE.is_match(line) && !YARN_FROZEN_RE.is_match(line) {
        violations.push(Violation {
            line_num,
            message: "Use 'yarn install --frozen-lockfile' to respect lockfile".to_string(),
            line_content: line.trim().to_string(),
        });
    }

    // Check for yarn add without version
    if YARN_ADD_RE.is_match(line) && !VERSION_PIN_RE.is_match(line) {
        violations.push(Violation {
            line_num,
            message:
                "yarn package installation without version pin (use 'yarn add package@version')"
                    .to_string(),
            line_content: line.trim().to_string(),
        });
    }

    violations
}

fn check_bun(line: &str, line_num: usize, bun_frozen_lockfile: bool) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Bun only freezes installs when `--frozen-lockfile` is passed or a repo-local
    // bunfig.toml enables `[install].frozenLockfile = true`.
    // Docs: https://bun.com/docs/runtime/bunfig#install-frozenlockfile
    if BUN_INSTALL_RE.is_match(line) && !line.contains("--frozen-lockfile") && !bun_frozen_lockfile
    {
        violations.push(Violation {
                line_num,
                message: "Use 'bun install --frozen-lockfile' unless repo-local bunfig.toml sets '[install].frozenLockfile = true' (https://bun.com/docs/runtime/bunfig#install-frozenlockfile)".to_string(),
                line_content: line.trim().to_string(),
            });
    }

    // Check for bun add without version
    if BUN_ADD_RE.is_match(line) && !VERSION_PIN_RE.is_match(line) {
        violations.push(Violation {
            line_num,
            message: "bun package installation without version pin (use 'bun add package@version')"
                .to_string(),
            line_content: line.trim().to_string(),
        });
    }

    violations
}

fn print_violations(path: &Path, violations: &[Violation]) {
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

#[cfg(test)]
#[allow(clippy::needless_raw_string_hashes, clippy::similar_names)]
mod tests {
    use super::*;

    #[test]
    fn test_npm_ci_allowed() {
        let violations = check_npm("npm ci", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_npm_install_bare_violation() {
        let violations = check_npm("npm install", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("npm ci"));
    }

    #[test]
    fn test_npm_install_with_version_allowed() {
        let violations = check_npm("npm i eslint@8.50.0", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_npm_install_without_version_violation() {
        let violations = check_npm("npm i eslint", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_pnpm_install_frozen_allowed() {
        let violations = check_pnpm("pnpm install --frozen-lockfile", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_pnpm_install_violation() {
        let violations = check_pnpm("pnpm install", 1);
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn test_yarn_frozen_allowed() {
        let violations = check_yarn("yarn install --frozen-lockfile", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_bun_add_with_version_allowed() {
        let violations = check_bun("bun add react@18.2.0", 1, false);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_bun_frozen_lockfile_enabled_stays_within_root() {
        let root = Path::new("/tmp/project");
        let file = Path::new("/tmp/project/docs/README.md");
        let cache = Mutex::new(HashMap::from([(PathBuf::from("/tmp/bunfig.toml"), true)]));

        assert!(!bun_frozen_lockfile_enabled(root, file, &cache));
    }

    #[test]
    fn test_shell_like_extensions_checked() {
        for file_name in ["install.sh", "install.bash", "install.zsh", "install.fish"] {
            assert!(should_check_file(Path::new(file_name)));
        }
    }

    #[test]
    fn test_makefiles_checked() {
        for file_name in ["Makefile", "makefile", "GNUmakefile", "rules.mk"] {
            assert!(should_check_file(Path::new(file_name)));
        }
    }

    #[test]
    fn test_github_workflows_not_excluded_as_git_dir() {
        assert!(!is_excluded(Path::new(".github/workflows/ci.yml")));
        assert!(should_check_file(Path::new(".github/workflows/ci.yml")));
    }

    #[test]
    fn test_bun_install_without_bunfig_violation() {
        let violations = check_bun("bun install", 1, false);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("bunfig.toml"));
        assert!(
            violations[0]
                .message
                .contains("bun.com/docs/runtime/bunfig#install-frozenlockfile")
        );
    }

    #[test]
    fn test_bun_install_allowed_with_bunfig_policy() {
        let violations = check_bun("bun install", 1, true);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_comment_skipped() {
        assert!(is_comment_or_placeholder("# npm install"));
        assert!(is_comment_or_placeholder("npm install <package>"));
    }

    #[test]
    fn test_markdown_bash_block_bun_install_violation() {
        let content = r#"
# Example Project

## Setup

```bash
bun install
```
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: true,
        };

        let violations = check_file(content, &context);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].line_content.contains("bun install"));
    }

    #[test]
    fn test_markdown_bash_block_respects_bunfig_policy() {
        let content = r#"
## Prerequisites

```bash
bun install
```
"#;
        let context = LintContext {
            bun_frozen_lockfile: true,
            is_markdown: true,
        };

        let violations = check_file(content, &context);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_markdown_unlabeled_code_block_not_linted() {
        let content = r#"
## Example Output

```
bun install
Done in 1.2s
```
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: true,
        };

        let violations = check_file(content, &context);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_markdown_text_examples_outside_code_blocks_not_linted() {
        let content = r#"
## Rules

- ✅ `bun install --frozen-lockfile`
- ❌ `bun install`
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: true,
        };

        let violations = check_file(content, &context);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_markdown_console_block_bun_install_violation() {
        let content = r#"
## Quickstart

```console
$ bun install
```
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: true,
        };

        let violations = check_file(content, &context);
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn test_markdown_code_block_comments_skipped() {
        let content = r#"
## Setup

```bash
# bun install
bun add react@18.2.0
```
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: true,
        };

        let violations = check_file(content, &context);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_markdown_output_example_not_linted() {
        let content = r#"
## Example

```
✗ ./README.md
  Line 11: Use 'bun install --frozen-lockfile'
  > bun install
```
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: true,
        };

        let violations = check_file(content, &context);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_shell_script_backticks_not_treated_as_markdown_fence() {
        let content = r#"
if [ "$x" = "```" ]; then
  bun install
fi
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
        };

        let violations = check_file(content, &context);
        assert_eq!(violations.len(), 1);
    }

    // ===== Scoped Package Tests =====
    // Reference: https://docs.npmjs.com/cli/v10/commands/npm-install
    // Scoped packages use @scope/package format where @ is part of the scope, not the version

    #[test]
    fn test_npm_scoped_package_without_version_violation() {
        // Should flag @types/node without version
        let violations = check_npm("npm i @types/node", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_npm_scoped_package_with_version_allowed() {
        // Should allow @types/node@18.0.0
        let violations = check_npm("npm i @types/node@18.0.0", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_npm_scoped_org_package_without_version_violation() {
        // Should flag @myorg/privatepackage without version
        let violations = check_npm("npm install @myorg/privatepackage", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_npm_scoped_org_package_with_version_allowed() {
        // Should allow @myorg/privatepackage@1.5.0
        let violations = check_npm("npm install @myorg/privatepackage@1.5.0", 1);
        assert_eq!(violations.len(), 0);
    }

    // ===== Full Semver Version Tests =====
    // Reference: https://docs.npmjs.com/cli/v10/commands/npm-install
    // npm supports full semver: major.minor.patch (e.g., 1.2.3)

    #[test]
    fn test_npm_full_semver_allowed() {
        // Should allow package@1.2.3
        let violations = check_npm("npm i eslint@8.50.0", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_npm_short_semver_allowed() {
        // Should allow package@1.2 (current regex matches this)
        let violations = check_npm("npm i package@1.2", 1);
        assert_eq!(violations.len(), 0);
    }

    // ===== npm ci Tests =====
    // Reference: https://docs.npmjs.com/cli/v10/commands/npm-ci
    // npm ci can only install entire projects; individual dependencies cannot be added

    #[test]
    fn test_npm_ci_bare_allowed() {
        // npm ci with no arguments is allowed
        let violations = check_npm("npm ci", 1);
        assert_eq!(violations.len(), 0);
    }

    // ===== Dev Dependency Flags Tests =====
    // Reference: https://docs.npmjs.com/cli/v10/commands/npm-install
    // -D, --save-dev flags should still require version pins

    #[test]
    fn test_npm_dev_flag_without_version_violation() {
        // Should flag npm i -D eslint
        let violations = check_npm("npm i -D eslint", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_npm_dev_flag_with_version_allowed() {
        // Should allow npm i -D eslint@8.0.0
        let violations = check_npm("npm i -D eslint@8.50.0", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_npm_save_dev_flag_without_version_violation() {
        // Should flag npm install --save-dev typescript
        let violations = check_npm("npm install --save-dev typescript", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_npm_save_dev_flag_with_version_allowed() {
        // Should allow npm install --save-dev typescript@5.0.0
        let violations = check_npm("npm install --save-dev typescript@5.0.0", 1);
        assert_eq!(violations.len(), 0);
    }

    // ===== pnpm Tests =====
    // Reference: https://pnpm.io/cli/add

    #[test]
    fn test_pnpm_scoped_package_without_version_violation() {
        let violations = check_pnpm("pnpm add @types/react", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_pnpm_scoped_package_with_version_allowed() {
        let violations = check_pnpm("pnpm add @types/react@18.0.0", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_pnpm_dev_flag_without_version_violation() {
        let violations = check_pnpm("pnpm add -D vitest", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_pnpm_dev_flag_with_version_allowed() {
        let violations = check_pnpm("pnpm add -D vitest@1.0.0", 1);
        assert_eq!(violations.len(), 0);
    }

    // ===== yarn Tests =====
    // Reference: https://yarnpkg.com/cli/add

    #[test]
    fn test_yarn_scoped_package_without_version_violation() {
        let violations = check_yarn("yarn add @babel/core", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_yarn_scoped_package_with_version_allowed() {
        let violations = check_yarn("yarn add @babel/core@7.22.0", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_yarn_dev_flag_without_version_violation() {
        let violations = check_yarn("yarn add -D jest", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_yarn_dev_flag_with_version_allowed() {
        let violations = check_yarn("yarn add -D jest@29.0.0", 1);
        assert_eq!(violations.len(), 0);
    }

    // ===== bun Tests =====
    // Reference: https://bun.sh/package-manager

    #[test]
    fn test_bun_scoped_package_without_version_violation() {
        let violations = check_bun("bun add @hono/hono", 1, false);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_bun_scoped_package_with_version_allowed() {
        let violations = check_bun("bun add @hono/hono@4.0.0", 1, false);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_bun_dev_flag_without_version_violation() {
        let violations = check_bun("bun add -D prettier", 1, false);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_bun_dev_flag_with_version_allowed() {
        let violations = check_bun("bun add -D prettier@3.0.0", 1, false);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_bunfig_has_frozen_lockfile_in_install_section() {
        let bunfig = r#"
            [install]
            frozenLockfile = true
        "#;

        assert!(bunfig_has_frozen_lockfile(bunfig));
    }

    #[test]
    fn test_bunfig_false_does_not_enable_frozen_lockfile() {
        let bunfig = r#"
            [install]
            frozenLockfile = false
        "#;

        assert!(!bunfig_has_frozen_lockfile(bunfig));
    }

    #[test]
    fn test_bunfig_ignores_frozen_lockfile_outside_install_section() {
        let bunfig = r#"
            [test]
            frozenLockfile = true
        "#;

        assert!(!bunfig_has_frozen_lockfile(bunfig));
    }

    #[test]
    fn test_should_lint_markdown_code_block_for_shell_languages() {
        assert!(should_lint_markdown_code_block("```bash"));
        assert!(should_lint_markdown_code_block("```sh"));
        assert!(should_lint_markdown_code_block("```console"));
        assert!(!should_lint_markdown_code_block("```"));
        assert!(!should_lint_markdown_code_block("```text"));
    }
}

use colored::Colorize;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use walkdir::WalkDir;

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

fn main() {
    println!("{}", "Checking for JS package manager violations...\n".blue());

    // Collect all .gitignore files in the repository
    let gitignores = collect_gitignores();
    let result = lint_files(&gitignores);

    println!();
    println!("{}", "═══════════════════════════════════════".blue());

    if result.violations_found == 0 {
        println!("{}", "✓ No violations found!".green());
        println!("{}", format!("Files checked: {}", result.files_checked).blue());
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
        println!();
        process::exit(1);
    }
}

fn collect_gitignores() -> Vec<(PathBuf, Gitignore)> {
    let mut gitignores = Vec::new();

    for entry in WalkDir::new(".")
        .into_iter()
        .filter_entry(|e| {
            let path = e.path();
            // Don't traverse into .git directory
            !path.to_string_lossy().contains("/.git/")
        })
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(".gitignore") {
            if let Some(parent) = path.parent() {
                let mut builder = GitignoreBuilder::new(parent);
                if builder.add(path).is_none() {
                    if let Ok(gitignore) = builder.build() {
                        gitignores.push((parent.to_path_buf(), gitignore));
                    }
                }
            }
        }
    }

    gitignores
}

fn is_ignored(path: &Path, gitignores: &[(PathBuf, Gitignore)]) -> bool {
    for (base_path, gitignore) in gitignores {
        // Check if the file is under this gitignore's directory
        if let Ok(relative) = path.strip_prefix(base_path) {
            let matched = gitignore.matched(relative, path.is_dir());
            if matched.is_ignore() {
                return true;
            }
        }
    }
    false
}

fn lint_files(gitignores: &[(PathBuf, Gitignore)]) -> LintResult {
    let mut violations_found: usize = 0;
    let mut files_checked: usize = 0;

    for entry in WalkDir::new(".")
        .into_iter()
        .filter_entry(|e| !is_excluded(e.path()))
        .filter_map(Result::ok)
    {
        let path = entry.path();

        if path.is_file() && should_check_file(path) {
            files_checked = files_checked.saturating_add(1);

            if let Ok(content) = fs::read_to_string(path) {
                let violations = check_file(&content);

                if !violations.is_empty() {
                    // Skip reporting if file is in gitignore
                    if is_ignored(path, gitignores) {
                        continue;
                    }

                    print_violations(path, &violations);
                    violations_found = violations_found.saturating_add(violations.len());
                }
            }
        }
    }

    LintResult {
        violations_found,
        files_checked,
    }
}

fn is_excluded(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains("node_modules")
        || path_str.contains(".git")
        || path_str.ends_with("lint-package-install.sh")
}

fn should_check_file(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let path_str = path.to_string_lossy();

    // Check Dockerfiles
    if file_name.starts_with("Dockerfile") || file_name.ends_with(".dockerfile") {
        return true;
    }

    // Check markdown files
    if Path::new(file_name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    {
        return true;
    }

    // Check shell scripts
    if Path::new(file_name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sh"))
    {
        return true;
    }

    // Check GitHub workflow files
    let is_yaml = Path::new(file_name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml"));
    
    if is_yaml && path_str.contains(".github/workflows") {
        return true;
    }

    false
}

fn check_file(content: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut in_code_block = false;

    for (line_num, line) in content.lines().enumerate() {
        let line_num = line_num.saturating_add(1); // 1-indexed

        // Track code blocks in markdown
        if line.trim().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        // Skip lines inside code blocks
        if in_code_block {
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
        violations.extend(check_bun(line, line_num));
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
        || trimmed.starts_with('-')  // Skip markdown list items that are examples
}

fn check_npm(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Skip if it's pnpm, yarn, or bun (not npm)
    if Regex::new(r"\b(pnpm|yarn|bun)\b").unwrap().is_match(line) {
        return violations;
    }

    // NPM CI is always allowed
    if Regex::new(r"\bnpm\s+ci\b").unwrap().is_match(line) {
        return violations;
    }

    // Check for npm install or npm i
    let npm_install_re = Regex::new(r"\bnpm\s+(install|i)(\s|$)").unwrap();
    if npm_install_re.is_match(line) {
        // Check if it has a version pin
        let version_pin_re = Regex::new(r"@[0-9]+\.[0-9]+").unwrap();
        if version_pin_re.is_match(line) {
            return violations; // Has version pin, allowed
        }

        // Check if it's bare 'npm install' (should use npm ci)
        let bare_install_re = Regex::new(r"\bnpm\s+(install|i)(\s+)?($|&&|;|\||#)").unwrap();
        if bare_install_re.is_match(line) {
            violations.push(Violation {
                line_num,
                message: "Use 'npm ci' instead of 'npm install' for lockfile-based installations"
                    .to_string(),
                line_content: line.trim().to_string(),
            });
        } else {
            violations.push(Violation {
                line_num,
                message: "npm package installation without version pin (use 'npm i package@version')"
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
    let pnpm_install_re = Regex::new(r"\bpnpm\s+install\b").unwrap();
    if pnpm_install_re.is_match(line) {
        let frozen_lockfile_re = Regex::new(r"--frozen-lockfile").unwrap();
        if !frozen_lockfile_re.is_match(line) {
            violations.push(Violation {
                line_num,
                message: "Use 'pnpm install --frozen-lockfile' to respect lockfile".to_string(),
                line_content: line.trim().to_string(),
            });
        }
    }

    // Check for pnpm add without version
    let pnpm_add_re = Regex::new(r"\bpnpm\s+add\s").unwrap();
    if pnpm_add_re.is_match(line) {
        let version_pin_re = Regex::new(r"@[0-9]+\.[0-9]+").unwrap();
        if !version_pin_re.is_match(line) {
            violations.push(Violation {
                line_num,
                message: "pnpm package installation without version pin (use 'pnpm add package@version')"
                    .to_string(),
                line_content: line.trim().to_string(),
            });
        }
    }

    violations
}

fn check_yarn(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Check for yarn install or bare yarn without --frozen-lockfile or --immutable
    let yarn_install_re = Regex::new(r"\byarn(\s+install)?(\s+)?($|&&|;|\||#)").unwrap();
    if yarn_install_re.is_match(line) {
        let frozen_re = Regex::new(r"--(frozen-lockfile|immutable)").unwrap();
        if !frozen_re.is_match(line) {
            violations.push(Violation {
                line_num,
                message: "Use 'yarn install --frozen-lockfile' to respect lockfile".to_string(),
                line_content: line.trim().to_string(),
            });
        }
    }

    // Check for yarn add without version
    let yarn_add_re = Regex::new(r"\byarn\s+(global\s+)?add\s").unwrap();
    if yarn_add_re.is_match(line) {
        let version_pin_re = Regex::new(r"@[0-9]+\.[0-9]+").unwrap();
        if !version_pin_re.is_match(line) {
            violations.push(Violation {
                line_num,
                message: "yarn package installation without version pin (use 'yarn add package@version')"
                    .to_string(),
                line_content: line.trim().to_string(),
            });
        }
    }

    violations
}

fn check_bun(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Check for bun install without --frozen-lockfile
    let bun_install_re = Regex::new(r"\bbun\s+install\b").unwrap();
    if bun_install_re.is_match(line) {
        let frozen_lockfile_re = Regex::new(r"--frozen-lockfile").unwrap();
        if !frozen_lockfile_re.is_match(line) {
            violations.push(Violation {
                line_num,
                message: "Use 'bun install --frozen-lockfile' to respect lockfile".to_string(),
                line_content: line.trim().to_string(),
            });
        }
    }

    // Check for bun add without version
    let bun_add_re = Regex::new(r"\bbun\s+add\s").unwrap();
    if bun_add_re.is_match(line) {
        let version_pin_re = Regex::new(r"@[0-9]+\.[0-9]+").unwrap();
        if !version_pin_re.is_match(line) {
            violations.push(Violation {
                line_num,
                message: "bun package installation without version pin (use 'bun add package@version')"
                    .to_string(),
                line_content: line.trim().to_string(),
            });
        }
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
        let violations = check_bun("bun add react@18.2.0", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_comment_skipped() {
        assert!(is_comment_or_placeholder("# npm install"));
        assert!(is_comment_or_placeholder("npm install <package>"));
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
        let violations = check_bun("bun add @hono/hono", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_bun_scoped_package_with_version_allowed() {
        let violations = check_bun("bun add @hono/hono@4.0.0", 1);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_bun_dev_flag_without_version_violation() {
        let violations = check_bun("bun add -D prettier", 1);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_bun_dev_flag_with_version_allowed() {
        let violations = check_bun("bun add -D prettier@3.0.0", 1);
        assert_eq!(violations.len(), 0);
    }
}

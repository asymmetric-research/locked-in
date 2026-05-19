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
    rule_id: Option<String>,
}

struct CommentStyle {
    prefix: &'static str,
    suffix: &'static str,
}

enum IgnoreDirective {
    All,
    Specific(String),
}

static IGNORE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"locked-in:\s*ignore(?:\s*\[([^\]]*)\])?").unwrap());

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
    is_package_json: bool,
}

struct SubmodulePruner {
    root: PathBuf,
    gitmodules_paths: Vec<PathBuf>,
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
    let submodule_pruner = SubmodulePruner::new(root);
    let files_to_check: Vec<PathBuf> = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .filter_entry(move |e| {
            let path = e.path();
            !is_excluded(path) && (!path.is_dir() || !submodule_pruner.is_submodule_dir(path))
        })
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
    let comment_style = comment_style_for_file(path)?;
    let context = LintContext {
        bun_frozen_lockfile: bun_frozen_lockfile_enabled(root, path, bun_context_cache),
        is_markdown: has_extension(path, "md"),
        is_package_json: is_package_json(path),
    };

    Some(FileLintResult {
        path: path.to_path_buf(),
        violations: check_file(&source, &context, &comment_style),
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

impl SubmodulePruner {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            gitmodules_paths: parse_gitmodules_paths(root),
        }
    }

    fn is_submodule_dir(&self, path: &Path) -> bool {
        self.is_declared_submodule_path(path)
            || (path != self.root && has_submodule_gitdir_file(path))
    }

    fn is_declared_submodule_path(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };

        self.gitmodules_paths
            .iter()
            .any(|submodule_path| relative == submodule_path)
    }
}

fn parse_gitmodules_paths(root: &Path) -> Vec<PathBuf> {
    let Ok(content) = fs::read_to_string(root.join(".gitmodules")) else {
        return Vec::new();
    };

    parse_gitmodules_paths_from_content(&content)
}

fn parse_gitmodules_paths_from_content(content: &str) -> Vec<PathBuf> {
    content
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .filter_map(|(key, value)| {
            if key.trim() == "path" {
                Some(PathBuf::from(value.trim().trim_matches('"')))
            } else {
                None
            }
        })
        .collect()
}

fn has_submodule_gitdir_file(path: &Path) -> bool {
    fs::read_to_string(path.join(".git"))
        .ok()
        .and_then(|content| parse_gitdir_target_from_content(&content))
        .is_some_and(|gitdir| gitdir_target_is_submodule(&gitdir))
}

fn parse_gitdir_target_from_content(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        line.trim_start()
            .strip_prefix("gitdir:")
            .map(|gitdir| gitdir.trim().to_string())
    })
}

fn gitdir_target_is_submodule(gitdir: &str) -> bool {
    let normalized = gitdir.replace('\\', "/");
    normalized.contains("/.git/modules/")
        || normalized.starts_with(".git/modules/")
        || normalized.starts_with("../.git/modules/")
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

    // Check package.json (npm/pnpm/yarn/bun scripts run here)
    if is_package_json(path) {
        return true;
    }

    // Check GitHub workflow files
    let is_yaml = has_extension(path, "yml") || has_extension(path, "yaml");

    if is_yaml && path_str.contains(".github/workflows") {
        return true;
    }

    false
}

fn is_package_json(path: &Path) -> bool {
    path.file_name() == Some(OsStr::new("package.json"))
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

fn comment_style_for_file(path: &Path) -> Option<CommentStyle> {
    if has_extension(path, "md") {
        return Some(CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        });
    }

    if is_shell_file(path) || is_makefile(path.file_name().and_then(|n| n.to_str()).unwrap_or(""))
    {
        return Some(CommentStyle {
            prefix: "#",
            suffix: "",
        });
    }

    if has_extension(path, "yml") || has_extension(path, "yaml") {
        return Some(CommentStyle {
            prefix: "#",
            suffix: "",
        });
    }

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if file_name.starts_with("Dockerfile") || file_name.ends_with(".dockerfile") {
        return Some(CommentStyle {
            prefix: "#",
            suffix: "",
        });
    }

    None
}

fn is_ignore_directive(line: &str, style: &CommentStyle) -> Option<IgnoreDirective> {
    let trimmed = line.trim();

    if !trimmed.starts_with(style.prefix) {
        return None;
    }

    if !style.suffix.is_empty() && !trimmed.ends_with(style.suffix) {
        return None;
    }

    let inner = trimmed
        .strip_prefix(style.prefix)
        .and_then(|rest| {
            if style.suffix.is_empty() {
                Some(rest.trim())
            } else if let Some(stripped) = rest.strip_suffix(style.suffix) {
                Some(stripped.trim())
            } else {
                None
            }
        });

    inner.and_then(|content| {
        IGNORE_RE.find(content).map(|m| {
            let caps = IGNORE_RE.captures(m.as_str());
            caps.and_then(|c| c.get(1)).map_or(IgnoreDirective::All, |rule_match| {
                let rule = rule_match.as_str().trim();
                if rule.is_empty() {
                    IgnoreDirective::All
                } else {
                    IgnoreDirective::Specific(rule.to_string())
                }
            })
        })
    })
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

fn check_file(content: &str, lint_context: &LintContext, comment_style: &CommentStyle) -> Vec<Violation> {
    if lint_context.is_package_json {
        return check_package_json(content, lint_context);
    }

    let mut violations = Vec::new();
    let mut in_code_block = false;
    let mut lint_code_block = false;
    let mut skip_next: Option<IgnoreDirective> = None;

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

        // Check if this line is an ignore directive
        if let Some(directive) = is_ignore_directive(line, comment_style) {
            skip_next = Some(directive);
            continue;
        }

        // Check if the ignore directive covers this line
        let skip_rule: Option<String> = match skip_next.take() {
            None => None,
            Some(IgnoreDirective::All) => continue,
            Some(IgnoreDirective::Specific(rule)) => Some(rule),
        };

        // Skip comments and placeholders
        if is_comment_or_placeholder(line) {
            continue;
        }

        // Check all package managers
        let mut line_violations: Vec<Violation> = Vec::new();
        line_violations.extend(check_npm(line, line_num));
        line_violations.extend(check_pnpm(line, line_num));
        line_violations.extend(check_yarn(line, line_num));
        line_violations.extend(check_bun(line, line_num, lint_context.bun_frozen_lockfile));

        if let Some(rule) = &skip_rule {
            line_violations.retain(|v| v.rule_id.as_deref() != Some(rule.as_str()));
        }

        violations.extend(line_violations);
    }

    violations
}

fn check_package_json(content: &str, lint_context: &LintContext) -> Vec<Violation> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(scripts) = value.get("scripts").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut violations = Vec::new();
    for (name, script_value) in scripts {
        let Some(script) = script_value.as_str() else {
            continue;
        };
        let line_num = find_script_line(content, name).unwrap_or(1);

        violations.extend(check_npm(script, line_num));
        violations.extend(check_pnpm(script, line_num));
        violations.extend(check_yarn(script, line_num));
        violations.extend(check_bun(script, line_num, lint_context.bun_frozen_lockfile));
    }

    violations.sort_by_key(|v| v.line_num);
    violations
}

fn find_script_line(content: &str, script_name: &str) -> Option<usize> {
    let needle = format!("\"{script_name}\"");
    let mut in_scripts = false;
    for (idx, line) in content.lines().enumerate() {
        if !in_scripts {
            if line.contains("\"scripts\"") {
                in_scripts = true;
            }
            continue;
        }
        if line.contains(&needle) {
            return Some(idx.saturating_add(1));
        }
    }
    None
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
                rule_id: Some("npm-install-bare".to_string()),
            });
        } else {
            violations.push(Violation {
                line_num,
                message:
                    "npm package installation without version pin (use 'npm i package@version')"
                        .to_string(),
                line_content: line.trim().to_string(),
                rule_id: Some("npm-version-pin".to_string()),
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
            rule_id: Some("pnpm-frozen-lockfile".to_string()),
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
            rule_id: Some("pnpm-version-pin".to_string()),
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
            rule_id: Some("yarn-frozen-lockfile".to_string()),
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
            rule_id: Some("yarn-version-pin".to_string()),
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
                rule_id: Some("bun-frozen-lockfile".to_string()),
            });
    }

    // Check for bun add without version
    if BUN_ADD_RE.is_match(line) && !VERSION_PIN_RE.is_match(line) {
        violations.push(Violation {
            line_num,
            message: "bun package installation without version pin (use 'bun add package@version')"
                .to_string(),
            line_content: line.trim().to_string(),
            rule_id: Some("bun-version-pin".to_string()),
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
    fn test_parse_gitmodules_paths() {
        let content = r#"
[submodule "vendor/foo"]
    path = vendor/foo
    url = https://example.com/foo.git
[submodule "deps/bar"]
    path = "deps/bar"
    url = https://example.com/bar.git
"#;

        assert_eq!(
            parse_gitmodules_paths_from_content(content),
            vec![PathBuf::from("vendor/foo"), PathBuf::from("deps/bar")]
        );
    }

    #[test]
    fn test_declared_submodule_path_detected() {
        let pruner = SubmodulePruner {
            root: PathBuf::from("/repo"),
            gitmodules_paths: vec![PathBuf::from("deps/foo")],
        };

        assert!(pruner.is_declared_submodule_path(Path::new("/repo/deps/foo")));
        assert!(!pruner.is_declared_submodule_path(Path::new("/repo/deps/foo/src")));
        assert!(!pruner.is_declared_submodule_path(Path::new("/repo/deps/bar")));
    }

    #[test]
    fn test_gitdir_target_submodule_detection() {
        assert!(gitdir_target_is_submodule("../.git/modules/deps/foo"));
        assert!(gitdir_target_is_submodule("/repo/.git/modules/deps/foo"));
        assert!(!gitdir_target_is_submodule("/repo/.git/worktrees/feature"));
        assert!(!gitdir_target_is_submodule("/repo/.git"));
    }

    #[test]
    fn test_parse_gitdir_target() {
        assert_eq!(
            parse_gitdir_target_from_content("gitdir: ../.git/modules/deps/foo\n"),
            Some("../.git/modules/deps/foo".to_string())
        );
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
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].line_content.contains("bun install"));
        assert_eq!(
            violations[0].rule_id.as_deref(),
            Some("bun-frozen-lockfile")
        );
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
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        let violations = check_file(content, &context, &style);
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
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        let violations = check_file(content, &context, &style);
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
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        let violations = check_file(content, &context, &style);
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
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        let violations = check_file(content, &context, &style);
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
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        let violations = check_file(content, &context, &style);
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
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        let violations = check_file(content, &context, &style);
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
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
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

    // ===== package.json Tests =====

    #[test]
    fn test_package_json_detected() {
        assert!(is_package_json(Path::new("package.json")));
        assert!(is_package_json(Path::new("/repo/package.json")));
        assert!(!is_package_json(Path::new("package-lock.json")));
        assert!(!is_package_json(Path::new("packages.json")));
        assert!(should_check_file(Path::new("package.json")));
    }

    fn package_json_context() -> LintContext {
        LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: true,
        }
    }

    fn shell_comment_style() -> CommentStyle {
        CommentStyle {
            prefix: "#",
            suffix: "",
        }
    }

    #[test]
    fn test_package_json_npm_install_violation() {
        let content = r#"{
  "name": "demo",
  "scripts": {
    "setup": "npm install"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("npm ci"));
        assert_eq!(violations[0].line_num, 4);
    }

    #[test]
    fn test_package_json_npm_ci_allowed() {
        let content = r#"{
  "scripts": {
    "ci": "npm ci"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_package_json_yarn_install_violation() {
        let content = r#"{
  "scripts": {
    "bootstrap": "yarn install"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("frozen-lockfile"));
    }

    #[test]
    fn test_package_json_yarn_frozen_allowed() {
        let content = r#"{
  "scripts": {
    "bootstrap": "yarn install --frozen-lockfile"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_package_json_pnpm_install_violation() {
        let content = r#"{
  "scripts": {
    "bootstrap": "pnpm install"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("frozen-lockfile"));
    }

    #[test]
    fn test_package_json_bun_install_violation() {
        let content = r#"{
  "scripts": {
    "bootstrap": "bun install"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("bunfig.toml"));
    }

    #[test]
    fn test_package_json_bun_install_allowed_with_bunfig_policy() {
        let content = r#"{
  "scripts": {
    "bootstrap": "bun install"
  }
}"#;
        let context = LintContext {
            bun_frozen_lockfile: true,
            is_markdown: false,
            is_package_json: true,
        };
        let violations = check_file(content, &context, &shell_comment_style());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_package_json_chained_command_violation() {
        let content = r#"{
  "scripts": {
    "build": "npm install && tsc"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("npm ci"));
    }

    #[test]
    fn test_package_json_add_without_version_violation() {
        let content = r#"{
  "scripts": {
    "add-dep": "yarn add react"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("version pin"));
    }

    #[test]
    fn test_package_json_add_with_version_allowed() {
        let content = r#"{
  "scripts": {
    "add-dep": "yarn add react@18.2.0"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 0);
    }

    // ===== Ignore Directive Tests =====

    #[test]
    fn test_ignore_directive_skips_next_line_shell() {
        let content = "# locked-in: ignore\nbun install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_ignore_directive_with_specific_rule() {
        let content = "# locked-in: ignore[bun-frozen-lockfile]\nbun install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_ignore_specific_rule_only_skips_that_rule() {
        // bun add should still be caught, only bun install is ignored
        let content = "# locked-in: ignore[bun-frozen-lockfile]\nbun install\nbun add react\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule_id.as_deref(),
            Some("bun-version-pin")
        );
    }

    #[test]
    fn test_ignore_directive_only_applies_to_next_line() {
        let content = "# locked-in: ignore\nbun install\nbun install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn test_ignore_directive_with_whitespace_variations() {
        let content = "  # locked-in: ignore\nbun install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_package_json_ignores_non_script_fields() {
        // Random commands appearing in description/keywords should not be linted.
        let content = r#"{
  "name": "demo",
  "description": "Run npm install to set up",
  "keywords": ["yarn install"],
  "scripts": {
    "ci": "npm ci"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_ignore_directive_with_whitespace_in_brackets() {
        let content = "# locked-in: ignore[ bun-frozen-lockfile ]\nbun install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_package_json_no_scripts_field() {
        let content = r#"{
  "name": "demo",
  "version": "1.0.0"
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_ignore_directive_markdown_html_comment() {
        let content = r#"<!-- locked-in: ignore -->
```bash
bun install
```
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: true,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_package_json_invalid_json_ignored() {
        let content = "{ not valid json";
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_ignore_directive_markdown_specific_rule() {
        let content = r#"<!-- locked-in: ignore[bun-frozen-lockfile] -->
```bash
bun install
bun add react
```
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: true,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule_id.as_deref(),
            Some("bun-version-pin")
        );
    }

    #[test]
    fn test_ignore_directive_yarn_respect_lockfile() {
        let content = "# locked-in: ignore[yarn-frozen-lockfile]\nyarn install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_package_json_multiple_violations() {
        let content = r#"{
  "scripts": {
    "a": "npm install",
    "b": "yarn install",
    "c": "pnpm install"
  }
}"#;
        let violations = check_file(content, &package_json_context(), &shell_comment_style());
        assert_eq!(violations.len(), 3);
    }

    #[test]
    fn test_ignore_directive_npm_install_bare() {
        let content = "# locked-in: ignore[npm-install-bare]\nnpm install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_ignore_directive_npm_install_version_pin() {
        let content =
            "# locked-in: ignore[npm-version-pin]\nnpm i eslint\nnpm install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        // Only npm-version-pin is ignored on the first line; bare npm install still fires on line 3
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule_id.as_deref(),
            Some("npm-install-bare")
        );
    }

    #[test]
    fn test_ignore_directive_yaml() {
        let content = r#"
- name: Install
  # locked-in: ignore
  run: bun install
"#;
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_ignore_directive_not_a_comment_is_not_parsed() {
        // A non-comment line containing "locked-in: ignore" should not be treated as a directive
        let content = "echo locked-in: ignore\nbun install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn test_ignore_directive_pnpm_frozen_lockfile() {
        let content =
            "# locked-in: ignore[pnpm-frozen-lockfile]\npnpm install\npnpm add react\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule_id.as_deref(),
            Some("pnpm-version-pin")
        );
    }

    #[test]
    fn test_ignore_directive_pnpm_version_pin() {
        let content =
            "# locked-in: ignore[pnpm-version-pin]\npnpm add react\npnpm install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule_id.as_deref(),
            Some("pnpm-frozen-lockfile")
        );
    }

    #[test]
    fn test_ignore_directive_yarn_version_pin() {
        let content =
            "# locked-in: ignore[yarn-version-pin]\nyarn add react\nyarn install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule_id.as_deref(),
            Some("yarn-frozen-lockfile")
        );
    }

    #[test]
    fn test_ignore_directive_bun_version_pin() {
        let content =
            "# locked-in: ignore[bun-version-pin]\nbun add react\nbun install\n";
        let context = LintContext {
            bun_frozen_lockfile: false,
            is_markdown: false,
            is_package_json: false,
        };
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        let violations = check_file(content, &context, &style);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule_id.as_deref(),
            Some("bun-frozen-lockfile")
        );
    }

    #[test]
    fn test_is_ignore_directive_shell() {
        let style = CommentStyle {
            prefix: "#",
            suffix: "",
        };

        assert!(matches!(
            is_ignore_directive("# locked-in: ignore", &style),
            Some(IgnoreDirective::All)
        ));
        assert!(matches!(
            is_ignore_directive("# locked-in: ignore[yarn-frozen-lockfile]", &style),
            Some(IgnoreDirective::Specific(ref s)) if s == "yarn-frozen-lockfile"
        ));
        assert!(matches!(
            is_ignore_directive("  # locked-in: ignore", &style),
            Some(IgnoreDirective::All)
        ));
        assert!(is_ignore_directive("# not an ignore", &style).is_none());
        assert!(is_ignore_directive("bun install", &style).is_none());
    }

    #[test]
    fn test_is_ignore_directive_markdown() {
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };

        assert!(matches!(
            is_ignore_directive("<!-- locked-in: ignore -->", &style),
            Some(IgnoreDirective::All)
        ));
        assert!(matches!(
            is_ignore_directive("<!-- locked-in: ignore[bun-frozen-lockfile] -->", &style),
            Some(IgnoreDirective::Specific(ref s)) if s == "bun-frozen-lockfile"
        ));
        assert!(matches!(
            is_ignore_directive("  <!-- locked-in: ignore -->  ", &style),
            Some(IgnoreDirective::All)
        ));
        assert!(is_ignore_directive("<!-- regular comment -->", &style).is_none());
    }

    #[test]
    fn test_comment_style_for_file() {
        assert!(comment_style_for_file(Path::new("script.sh")).is_some());
        assert!(comment_style_for_file(Path::new("README.md")).is_some());
        assert!(comment_style_for_file(Path::new(".github/workflows/ci.yml")).is_some());
        assert!(comment_style_for_file(Path::new("Dockerfile")).is_some());
        assert!(comment_style_for_file(Path::new("Makefile")).is_some());

        let sh_style = comment_style_for_file(Path::new("script.sh")).unwrap();
        assert_eq!(sh_style.prefix, "#");
        assert_eq!(sh_style.suffix, "");

        let md_style = comment_style_for_file(Path::new("README.md")).unwrap();
        assert_eq!(md_style.prefix, "<!--");
        assert_eq!(md_style.suffix, "-->");
    }
}

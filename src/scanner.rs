use crate::bounded_io::{MAX_CONFIG_FILE_SIZE, MAX_SOURCE_FILE_SIZE, read_bounded_utf8};
use crate::context::bun::bun_frozen_lockfile_enabled;
use crate::context::git::{GitIndexStatus, SubmodulePruner, tracked_paths};
use crate::context::js_workspace::{declares_member, workspace_patterns};
use crate::context::lockfiles::{
    Ecosystem, expected_lockfile_paths, expected_lockfiles_for_manifest, javascript_lockfiles,
};
use crate::diagnostic::{FileLintResult, LintResult, Severity, Violation};
use crate::file_types::{
    comment_style_for_file, has_extension, is_excluded, is_package_json, should_check_file,
};
use crate::parsers::line::check_file;
use crate::report::print_violations;
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_FILES_TO_SCAN: usize = 100_000;
const MAX_CONCURRENT_FILE_SCANS: usize = 2;
const MIN_STRIPPED_NPM_PACKAGE_RECORDS: usize = 10;

#[derive(Clone, Copy)]
pub struct LintOptions {
    pub check_npm_integrity: bool,
}

impl Default for LintOptions {
    fn default() -> Self {
        Self {
            check_npm_integrity: true,
        }
    }
}

pub struct LintContext {
    pub bun_frozen_lockfile: bool,
    pub is_markdown: bool,
    pub is_package_json: bool,
}

#[must_use]
pub fn lint_files(root: &Path) -> LintResult {
    lint_files_with_options(root, LintOptions::default())
}

#[must_use]
pub fn lint_files_with_options(root: &Path, options: LintOptions) -> LintResult {
    let submodule_pruner = SubmodulePruner::new(root);
    let mut files_to_check: Vec<PathBuf> = WalkBuilder::new(root)
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
        // Symlink targets are intentionally not scanner inputs. Bounded I/O
        // still rejects them for callers that explicitly attempt to read one.
        .filter(|path| !path.is_symlink() && path.is_file() && should_check_file(path))
        .take(MAX_FILES_TO_SCAN.saturating_add(1))
        .collect();
    let file_limit_exceeded = files_to_check.len() > MAX_FILES_TO_SCAN;
    files_to_check.truncate(MAX_FILES_TO_SCAN);

    let bun_context_cache: Mutex<HashMap<PathBuf, bool>> = Mutex::new(HashMap::new());

    let mut checked_results = Vec::new();
    for files in files_to_check.chunks(MAX_CONCURRENT_FILE_SCANS) {
        let chunk_results: Vec<FileLintResult> = files
            .par_iter()
            .filter_map(|path| lint_file(root, path, &bun_context_cache))
            .collect();
        checked_results.extend(chunk_results);
    }

    if file_limit_exceeded {
        checked_results.push(FileLintResult {
            path: root.to_path_buf(),
            violations: vec![Violation::error(
                0,
                format!("Repository exceeds the {MAX_FILES_TO_SCAN}-file scan limit"),
                root.to_string_lossy(),
                "scan-file-limit",
            )],
        });
    }

    checked_results.extend(check_tracked_lockfiles(root, options));

    checked_results.sort_by(|a, b| a.path.cmp(&b.path));

    let mut violations_found: usize = 0;
    let mut warnings_found: usize = 0;

    for result in &checked_results {
        if !result.violations.is_empty() {
            print_violations(&result.path, &result.violations);
            violations_found = violations_found.saturating_add(
                result
                    .violations
                    .iter()
                    .filter(|violation| violation.severity == Severity::Error)
                    .count(),
            );
            warnings_found = warnings_found.saturating_add(
                result
                    .violations
                    .iter()
                    .filter(|violation| violation.severity == Severity::Warning)
                    .count(),
            );
        }
    }

    LintResult {
        violations_found,
        warnings_found,
        files_checked: checked_results.len(),
    }
}

fn lint_file(
    root: &Path,
    path: &Path,
    bun_context_cache: &Mutex<HashMap<PathBuf, bool>>,
) -> Option<FileLintResult> {
    let comment_style = comment_style_for_file(path)?;
    let max_file_size = if is_package_json(path) {
        MAX_CONFIG_FILE_SIZE
    } else {
        MAX_SOURCE_FILE_SIZE
    };
    let source = match read_bounded_utf8(path, max_file_size, true) {
        Ok(source) => source,
        Err(error) => {
            return Some(FileLintResult {
                path: path.to_path_buf(),
                violations: vec![Violation::error(
                    0,
                    format!("Input file {error}; refusing to skip security checks"),
                    path.to_string_lossy(),
                    "scan-input-unavailable",
                )],
            });
        }
    };
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

fn check_tracked_lockfiles(root: &Path, options: LintOptions) -> Vec<FileLintResult> {
    let tracked = match tracked_paths(root) {
        Ok(paths) => paths,
        Err(status) => {
            return vec![FileLintResult {
                path: root.join(".git"),
                violations: vec![git_metadata_warning(status)],
            }];
        }
    };

    let tracked_set: HashSet<PathBuf> = tracked.iter().cloned().collect();
    let mut results = Vec::new();

    for manifest in tracked {
        let Some(expectation) = expected_lockfiles_for_manifest(&manifest) else {
            continue;
        };
        let Some(expected_paths) = expected_lockfile_paths(&manifest) else {
            continue;
        };
        if !manifest_needs_lockfile(root, &manifest, expectation.ecosystem) {
            continue;
        }

        let acceptable_paths = acceptable_lockfile_paths(root, &manifest, &expected_paths);

        if acceptable_paths
            .iter()
            .any(|path| tracked_set.contains(path))
        {
            continue;
        }

        let lockfiles = acceptable_paths
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join(", ");
        results.push(FileLintResult {
            path: root.join(&manifest),
            violations: vec![Violation::warning(
                format!("Tracked manifest is missing a tracked lockfile: {lockfiles}"),
                manifest.to_string_lossy(),
                "missing-tracked-lockfile",
            )],
        });
    }

    if options.check_npm_integrity {
        results.extend(check_npm_lockfile_integrity(root, &tracked_set));
    }

    results
}

fn check_npm_lockfile_integrity(root: &Path, tracked: &HashSet<PathBuf>) -> Vec<FileLintResult> {
    tracked
        .iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name == "package-lock.json" || name == "npm-shrinkwrap.json")
        })
        .filter_map(|path| {
            let lockfile = root.join(path);
            let content = read_bounded_utf8(&lockfile, MAX_CONFIG_FILE_SIZE, true).ok()?;
            let stats = npm_lockfile_integrity_stats(&content)?;

            let message = if stats.fetchable_without_integrity > 0 {
                format!(
                    "npm lockfile has {} fetched package artifact(s) without integrity hashes; regenerate it after removing node_modules",
                    stats.fetchable_without_integrity
                )
            } else if stats.candidates >= MIN_STRIPPED_NPM_PACKAGE_RECORDS
                && stats.without_source_metadata == stats.candidates
            {
                format!(
                    "npm lockfile lacks source metadata: all {} likely registry package records lack both resolved URLs and integrity hashes; regenerate it after removing node_modules",
                    stats.candidates
                )
            } else {
                return None;
            };

            Some(FileLintResult {
                path: lockfile,
                violations: vec![Violation::error(
                    0,
                    message,
                    path.to_string_lossy(),
                    "npm-lockfile-missing-integrity",
                )],
            })
        })
        .collect()
}

#[derive(Default)]
struct NpmIntegrityStats {
    candidates: usize,
    fetchable_without_integrity: usize,
    without_source_metadata: usize,
}

fn npm_lockfile_integrity_stats(content: &str) -> Option<NpmIntegrityStats> {
    let lockfile: Value = serde_json::from_str(content).ok()?;
    let object = lockfile.as_object()?;

    if let Some(packages) = object.get("packages").and_then(Value::as_object) {
        let mut stats = NpmIntegrityStats::default();
        for (path, package) in packages {
            if path.is_empty() || !path.split('/').any(|component| component == "node_modules") {
                continue;
            }
            add_package_integrity_stat(package, &mut stats);
        }
        return Some(stats);
    }

    let mut stats = NpmIntegrityStats::default();
    collect_v1_package_integrity_stats(object.get("dependencies")?, &mut stats);
    Some(stats)
}

fn collect_v1_package_integrity_stats(packages: &Value, stats: &mut NpmIntegrityStats) {
    let Some(packages) = packages.as_object() else {
        return;
    };

    for package in packages.values() {
        add_package_integrity_stat(package, stats);
        if let Some(dependencies) = package.get("dependencies") {
            collect_v1_package_integrity_stats(dependencies, stats);
        }
    }
}

fn add_package_integrity_stat(package: &Value, stats: &mut NpmIntegrityStats) {
    if !package.is_object() {
        return;
    }

    if package.get("link").and_then(Value::as_bool) == Some(true)
        || package.get("inBundle").and_then(Value::as_bool) == Some(true)
        || package.get("bundled").and_then(Value::as_bool) == Some(true)
    {
        return;
    }

    let has_integrity = package
        .get("integrity")
        .and_then(Value::as_str)
        .is_some_and(|integrity| !integrity.is_empty());
    let resolved = package
        .get("resolved")
        .and_then(Value::as_str)
        .filter(|resolved| !resolved.is_empty());

    if resolved.is_some_and(is_non_artifact_source) {
        return;
    }

    if resolved.is_some_and(is_fetchable_artifact) {
        stats.candidates = stats.candidates.saturating_add(1);
        if !has_integrity {
            stats.fetchable_without_integrity = stats.fetchable_without_integrity.saturating_add(1);
        }
    } else if has_integrity {
        stats.candidates = stats.candidates.saturating_add(1);
    } else if resolved.is_none() && package_version_looks_registry_based(package) {
        stats.candidates = stats.candidates.saturating_add(1);
        stats.without_source_metadata = stats.without_source_metadata.saturating_add(1);
    }
}

fn is_fetchable_artifact(resolved: &str) -> bool {
    let resolved = resolved.to_ascii_lowercase();
    resolved.starts_with("https://")
        || resolved.starts_with("http://")
        || resolved.starts_with("registry.npmjs.org/")
}

fn is_non_artifact_source(resolved: &str) -> bool {
    let resolved = resolved.to_ascii_lowercase();
    resolved.starts_with("file:")
        || resolved.starts_with("link:")
        || resolved.starts_with("workspace:")
        || resolved.starts_with("git")
        || resolved.starts_with("ssh:")
        || resolved.starts_with("github:")
        || resolved.starts_with("gitlab:")
        || resolved.starts_with("bitbucket:")
        || resolved.starts_with("git@")
        || resolved.contains(".git#")
        || resolved.starts_with("./")
        || resolved.starts_with("../")
        || resolved.starts_with('/')
}

fn package_version_looks_registry_based(package: &Value) -> bool {
    package
        .get("version")
        .and_then(Value::as_str)
        .is_some_and(|version| {
            version.starts_with("npm:")
                || version.as_bytes().first().is_some_and(u8::is_ascii_digit)
        })
}

fn manifest_needs_lockfile(root: &Path, manifest: &Path, ecosystem: Ecosystem) -> bool {
    match ecosystem {
        Ecosystem::JavaScript => javascript_manifest_needs_lockfile(root, manifest),
        Ecosystem::Rust => true,
        Ecosystem::Go => read_bounded_utf8(&root.join(manifest), MAX_CONFIG_FILE_SIZE, true)
            .map_or(true, |content| go_mod_has_module_and_require(&content)),
    }
}

fn javascript_manifest_needs_lockfile(root: &Path, manifest: &Path) -> bool {
    read_bounded_utf8(&root.join(manifest), MAX_CONFIG_FILE_SIZE, true).map_or(true, |content| {
        let Ok(manifest) = serde_json::from_str::<Value>(&content) else {
            return true;
        };
        let Some(manifest) = manifest.as_object() else {
            return true;
        };

        manifest.contains_key("workspaces")
            || ["dependencies", "devDependencies", "optionalDependencies"]
                .iter()
                .any(|key| {
                    manifest.get(*key).is_some_and(|dependencies| {
                        dependencies
                            .as_object()
                            .is_none_or(|dependencies| !dependencies.is_empty())
                    })
                })
    })
}

fn go_mod_has_module_and_require(content: &str) -> bool {
    let mut has_module = false;
    let mut has_require = false;

    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("module ") {
            has_module = true;
        } else if trimmed.starts_with("require ") || trimmed == "require(" || trimmed == "require ("
        {
            has_require = true;
        }
    }

    has_module && has_require
}

fn acceptable_lockfile_paths(
    root: &Path,
    manifest: &Path,
    expected_paths: &[PathBuf],
) -> Vec<PathBuf> {
    let mut paths = expected_paths.to_vec();

    if manifest
        .file_name()
        .is_some_and(|name| name == "Cargo.toml")
    {
        paths.extend(cargo_workspace_lockfile_paths(root, manifest));
        paths.sort();
        paths.dedup();
    } else if manifest
        .file_name()
        .is_some_and(|name| name == "package.json")
    {
        paths.extend(js_workspace_lockfile_paths(root, manifest));
        paths.sort();
        paths.dedup();
    }

    paths
}

fn cargo_workspace_lockfile_paths(root: &Path, manifest: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut current = manifest.parent();

    while let Some(dir) = current {
        let workspace_manifest = dir.join("Cargo.toml");
        if workspace_manifest != manifest && cargo_manifest_is_workspace(root, &workspace_manifest)
        {
            paths.push(dir.join("Cargo.lock"));
        }
        current = dir.parent();
    }

    paths
}

fn cargo_manifest_is_workspace(root: &Path, manifest: &Path) -> bool {
    read_bounded_utf8(&root.join(manifest), MAX_CONFIG_FILE_SIZE, true).is_ok_and(|content| {
        content
            .lines()
            .any(|line| line.trim_start().starts_with("[workspace]"))
    })
}

fn js_workspace_lockfile_paths(root: &Path, manifest: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut current = manifest.parent();

    while let Some(dir) = current {
        let workspace_manifest = dir.join("package.json");
        if workspace_manifest != manifest && js_workspace_includes_member(root, dir, manifest) {
            paths.extend(javascript_lockfiles().iter().map(|name| dir.join(name)));
        }
        current = dir.parent();
    }

    paths
}

fn js_workspace_includes_member(root: &Path, workspace_dir: &Path, member: &Path) -> bool {
    let Some(member_dir) = member.parent() else {
        return false;
    };
    let Ok(member_rel) = member_dir.strip_prefix(workspace_dir) else {
        return false;
    };
    declares_member(&workspace_patterns(root, workspace_dir), member_rel)
}

fn git_metadata_warning(status: GitIndexStatus) -> Violation {
    let message = match status {
        GitIndexStatus::MissingMetadata => {
            "Git metadata not found; skipping tracked lockfile validation"
        }
        GitIndexStatus::MissingIndex => "Git index not found; skipping tracked lockfile validation",
        GitIndexStatus::ExceedsLimits => {
            "Git index exceeds safe resource limits; skipping tracked lockfile validation"
        }
        GitIndexStatus::UnsupportedIndex => {
            "Git index could not be parsed; skipping tracked lockfile validation"
        }
    };

    Violation::warning(message, ".git/index", "git-metadata-unavailable")
}

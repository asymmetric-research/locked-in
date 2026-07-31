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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_FILES_TO_SCAN: usize = 100_000;
const MAX_CONCURRENT_FILE_SCANS: usize = 2;

pub struct LintContext {
    pub bun_frozen_lockfile: bool,
    pub is_markdown: bool,
    pub is_package_json: bool,
}

pub fn lint_files(root: &Path) -> LintResult {
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
        .filter(|path| path.is_file() && should_check_file(path))
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

    checked_results.extend(check_tracked_lockfiles(root));

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

fn check_tracked_lockfiles(root: &Path) -> Vec<FileLintResult> {
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

    results
}

fn manifest_needs_lockfile(root: &Path, manifest: &Path, ecosystem: Ecosystem) -> bool {
    if ecosystem != Ecosystem::Go {
        return true;
    }

    read_bounded_utf8(&root.join(manifest), MAX_CONFIG_FILE_SIZE, true)
        .map_or(true, |content| go_mod_has_module_and_require(&content))
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

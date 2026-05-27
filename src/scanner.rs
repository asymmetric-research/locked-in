use crate::context::bun::bun_frozen_lockfile_enabled;
use crate::context::git::SubmodulePruner;
use crate::diagnostic::{FileLintResult, LintResult};
use crate::file_types::{
    comment_style_for_file, has_extension, is_excluded, is_package_json, should_check_file,
};
use crate::parsers::line::check_file;
use crate::report::print_violations;
use ignore::WalkBuilder;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct LintContext {
    pub bun_frozen_lockfile: bool,
    pub is_markdown: bool,
    pub is_package_json: bool,
}

pub fn lint_files(root: &Path) -> LintResult {
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

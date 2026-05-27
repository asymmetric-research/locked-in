use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    JavaScript,
    Rust,
    Go,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfileExpectation {
    pub ecosystem: Ecosystem,
    pub manifests: &'static [&'static str],
    pub lockfiles: &'static [&'static str],
}

const EXPECTATIONS: &[LockfileExpectation] = &[
    LockfileExpectation {
        ecosystem: Ecosystem::JavaScript,
        manifests: &["package.json"],
        lockfiles: &[
            "package-lock.json",
            "npm-shrinkwrap.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "bun.lockb",
            "bun.lock",
        ],
    },
    LockfileExpectation {
        ecosystem: Ecosystem::Rust,
        manifests: &["Cargo.toml"],
        lockfiles: &["Cargo.lock"],
    },
    LockfileExpectation {
        ecosystem: Ecosystem::Go,
        manifests: &["go.mod"],
        lockfiles: &["go.sum"],
    },
];

#[must_use]
pub const fn lockfile_expectations() -> &'static [LockfileExpectation] {
    EXPECTATIONS
}

#[must_use]
pub fn expected_lockfiles_for_manifest(path: &Path) -> Option<&'static LockfileExpectation> {
    let file_name = path.file_name()?.to_str()?;
    EXPECTATIONS
        .iter()
        .find(|expectation| expectation.manifests.contains(&file_name))
}

#[must_use]
pub fn expected_lockfile_paths(path: &Path) -> Option<Vec<PathBuf>> {
    let expectation = expected_lockfiles_for_manifest(path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new(""));

    Some(
        expectation
            .lockfiles
            .iter()
            .map(|lockfile| parent.join(lockfile))
            .collect(),
    )
}

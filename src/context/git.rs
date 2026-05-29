use std::fs;
use std::path::{Path, PathBuf};

pub struct SubmodulePruner {
    pub(crate) root: PathBuf,
    pub(crate) gitmodules_paths: Vec<PathBuf>,
}

impl SubmodulePruner {
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            gitmodules_paths: parse_gitmodules_paths(root),
        }
    }

    #[must_use]
    pub fn is_submodule_dir(&self, path: &Path) -> bool {
        self.is_declared_submodule_path(path)
            || (path != self.root && has_submodule_gitdir_file(path))
    }

    #[must_use]
    pub fn is_declared_submodule_path(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };

        self.gitmodules_paths
            .iter()
            .any(|submodule_path| relative == submodule_path)
    }
}

#[must_use]
pub fn parse_gitmodules_paths(root: &Path) -> Vec<PathBuf> {
    let Ok(content) = fs::read_to_string(root.join(".gitmodules")) else {
        return Vec::new();
    };

    parse_gitmodules_paths_from_content(&content)
}

#[must_use]
pub fn parse_gitmodules_paths_from_content(content: &str) -> Vec<PathBuf> {
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

#[must_use]
pub fn has_submodule_gitdir_file(path: &Path) -> bool {
    fs::read_to_string(path.join(".git"))
        .ok()
        .and_then(|content| parse_gitdir_target_from_content(&content))
        .is_some_and(|gitdir| gitdir_target_is_submodule(&gitdir))
}

#[must_use]
pub fn parse_gitdir_target_from_content(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        line.trim_start()
            .strip_prefix("gitdir:")
            .map(|gitdir| gitdir.trim().to_string())
    })
}

#[must_use]
pub fn gitdir_target_is_submodule(gitdir: &str) -> bool {
    let normalized = gitdir.replace('\\', "/");
    normalized.contains("/.git/modules/")
        || normalized.starts_with(".git/modules/")
        || normalized.starts_with("../.git/modules/")
}

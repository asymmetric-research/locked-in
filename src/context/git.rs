use crate::bounded_io::{
    BoundedReadError, MAX_CONFIG_FILE_SIZE, MAX_GIT_INDEX_SIZE, read_bounded_bytes,
    read_bounded_utf8,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitIndexStatus {
    MissingMetadata,
    MissingIndex,
    ExceedsLimits,
    UnsupportedIndex,
}

const MIN_GIT_INDEX_ENTRY_SIZE: usize = 64;
const MAX_GIT_INDEX_ENTRIES: usize = 100_000;
const MAX_GIT_PATH_LENGTH: usize = 4_096;
const MAX_SUBMODULE_PATHS: usize = 10_000;

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
    let Ok(content) = read_bounded_utf8(&root.join(".gitmodules"), MAX_CONFIG_FILE_SIZE, true)
    else {
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
                let path = value.trim().trim_matches('"');
                (path.len() <= MAX_GIT_PATH_LENGTH).then(|| PathBuf::from(path))
            } else {
                None
            }
        })
        .take(MAX_SUBMODULE_PATHS)
        .collect()
}

#[must_use]
pub fn has_submodule_gitdir_file(path: &Path) -> bool {
    read_bounded_utf8(&path.join(".git"), MAX_CONFIG_FILE_SIZE, false)
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

#[must_use]
pub fn git_metadata_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }

    let content = read_bounded_utf8(&dot_git, MAX_CONFIG_FILE_SIZE, false).ok()?;
    let gitdir = parse_gitdir_target_from_content(&content)?;
    let path = PathBuf::from(gitdir);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

/// Returns tracked paths from the repository index.
///
/// # Errors
///
/// Returns a [`GitIndexStatus`] when git metadata is unavailable or the index
/// cannot be parsed.
pub fn tracked_paths(root: &Path) -> Result<Vec<PathBuf>, GitIndexStatus> {
    let git_dir = git_metadata_dir(root).ok_or(GitIndexStatus::MissingMetadata)?;
    let index =
        read_bounded_bytes(&git_dir.join("index"), MAX_GIT_INDEX_SIZE, false).map_err(|error| {
            match error {
                BoundedReadError::Io(_) => GitIndexStatus::MissingIndex,
                BoundedReadError::TooLarge { .. } | BoundedReadError::Allocation => {
                    GitIndexStatus::ExceedsLimits
                }
                _ => GitIndexStatus::UnsupportedIndex,
            }
        })?;
    parse_tracked_paths_from_index(&index).ok_or(GitIndexStatus::UnsupportedIndex)
}

#[must_use]
pub fn parse_tracked_paths_from_index(index: &[u8]) -> Option<Vec<PathBuf>> {
    if index.len() < 12 || &index[0..4] != b"DIRC" {
        return None;
    }

    let version = read_u32(&index[4..8])?;
    if !matches!(version, 2 | 3) {
        return None;
    }

    let entry_count = usize::try_from(read_u32(&index[8..12])?).ok()?;
    let maximum_possible_entries = index
        .len()
        .checked_sub(12)?
        .checked_div(MIN_GIT_INDEX_ENTRY_SIZE)?;
    if entry_count > maximum_possible_entries || entry_count > MAX_GIT_INDEX_ENTRIES {
        return None;
    }
    let mut offset = 12usize;
    let mut paths = Vec::new();
    paths.try_reserve(entry_count).ok()?;

    for _ in 0..entry_count {
        let entry_start = offset;
        let path_start = entry_start.checked_add(62)?;
        if path_start > index.len() {
            return None;
        }

        let path_end = index[path_start..]
            .iter()
            .position(|byte| *byte == 0)
            .and_then(|position| path_start.checked_add(position))?;
        if path_end.checked_sub(path_start)? > MAX_GIT_PATH_LENGTH {
            return None;
        }
        let path = std::str::from_utf8(&index[path_start..path_end]).ok()?;
        paths.push(PathBuf::from(path));

        offset = path_end.checked_add(1)?;
        while !offset.checked_sub(entry_start)?.is_multiple_of(8) {
            offset = offset.checked_add(1)?;
        }
        if offset > index.len() {
            return None;
        }
    }

    Some(paths)
}

fn read_u32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.try_into().ok()?))
}

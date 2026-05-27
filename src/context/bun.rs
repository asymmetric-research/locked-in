use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn bun_frozen_lockfile_enabled(
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

        if let Ok(cache) = cache.lock()
            && let Some(enabled) = cache.get(&bunfig_path)
        {
            return *enabled;
        }

        let enabled = fs::read_to_string(&bunfig_path)
            .is_ok_and(|content| bunfig_has_frozen_lockfile(&content));
        if let Ok(mut cache) = cache.lock() {
            cache.insert(bunfig_path, enabled);
        }
        return enabled;
    }

    false
}

#[must_use]
pub fn bunfig_has_frozen_lockfile(content: &str) -> bool {
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

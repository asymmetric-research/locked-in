use crate::bounded_io::{MAX_CONFIG_FILE_SIZE, read_bounded_utf8};
use std::path::Path;

const MAX_WORKSPACE_PATTERNS: usize = 1_000;
const MAX_WORKSPACE_PATTERN_LENGTH: usize = 1_024;

/// Collects the workspace glob patterns declared at `dir` (relative to `root`).
///
/// Honors the workspace formats of the four JS package managers:
/// - yarn / npm / bun: a `"workspaces"` field in `package.json`, either an array
///   (`["packages/*"]`) or an object with a `packages` array
///   (`{ "packages": ["packages/*"] }`).
/// - pnpm: a `packages:` list in a `pnpm-workspace.yaml` alongside `package.json`.
///
/// Returns an empty list when neither file declares a workspace.
#[must_use]
pub fn workspace_patterns(root: &Path, dir: &Path) -> Vec<String> {
    let base = root.join(dir);
    let mut patterns = Vec::new();

    if let Ok(content) = read_bounded_utf8(&base.join("package.json"), MAX_CONFIG_FILE_SIZE, true) {
        patterns.extend(patterns_from_package_json(&content));
    }
    if let Ok(content) = read_bounded_utf8(
        &base.join("pnpm-workspace.yaml"),
        MAX_CONFIG_FILE_SIZE,
        true,
    ) {
        patterns.extend(patterns_from_pnpm_yaml(&content));
    }

    patterns
}

/// Returns `true` if `member_rel` (the member directory relative to the workspace
/// directory) is declared by `patterns`: it must match at least one positive pattern
/// and no negation (`!`-prefixed) pattern.
#[must_use]
pub fn declares_member(patterns: &[String], member_rel: &Path) -> bool {
    let rel = member_rel.to_string_lossy().replace('\\', "/");
    let rel = rel.trim_end_matches('/');

    let mut matched = false;
    for pattern in patterns.iter().take(MAX_WORKSPACE_PATTERNS) {
        if pattern.len() > MAX_WORKSPACE_PATTERN_LENGTH {
            continue;
        }
        if let Some(negated) = pattern.strip_prefix('!') {
            if matches_glob(negated, rel) {
                return false;
            }
        } else if matches_glob(pattern, rel) {
            matched = true;
        }
    }

    matched
}

#[must_use]
pub fn patterns_from_package_json(content: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(workspaces) = value.get("workspaces") else {
        return Vec::new();
    };

    let array = workspaces.as_array().or_else(|| {
        workspaces
            .as_object()
            .and_then(|object| object.get("packages"))
            .and_then(serde_json::Value::as_array)
    });

    array
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(String::from))
                .filter(|pattern| pattern.len() <= MAX_WORKSPACE_PATTERN_LENGTH)
                .take(MAX_WORKSPACE_PATTERNS)
                .collect()
        })
        .unwrap_or_default()
}

#[must_use]
pub fn patterns_from_pnpm_yaml(content: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut in_packages = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if in_packages {
            if let Some(item) = trimmed.strip_prefix('-') {
                let pattern = unquote(item.trim());
                if pattern.len() <= MAX_WORKSPACE_PATTERN_LENGTH
                    && patterns.len() < MAX_WORKSPACE_PATTERNS
                {
                    patterns.push(pattern.to_string());
                }
                continue;
            }
            // A non-list line that is itself unindented ends the packages block.
            if !line.starts_with(char::is_whitespace) {
                in_packages = false;
            }
        }

        if !line.starts_with(char::is_whitespace) && trimmed == "packages:" {
            in_packages = true;
        }
    }

    patterns
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(value)
}

/// Matches a workspace glob `pattern` against a `/`-separated relative `path`.
///
/// Matching is segment-aware, following the conventions shared by the JS package
/// managers: a `**` path segment spans any number of segments, `*` matches any run
/// of characters within a single segment, and `?` matches a single character.
fn matches_glob(pattern: &str, path: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.trim_end_matches('/').split('/').collect();
    let path_segments: Vec<&str> = path.split('/').collect();
    match_segments(&pattern_segments, &path_segments)
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    let mut previous = vec![false; path.len().saturating_add(1)];
    previous[0] = true;

    for segment in pattern {
        let mut current = vec![false; path.len().saturating_add(1)];
        if *segment == "**" {
            current[0] = previous[0];
            for (path_index, _) in path.iter().enumerate() {
                let current_index = path_index.saturating_add(1);
                current[current_index] = previous[current_index] || current[path_index];
            }
        } else {
            for (path_index, path_segment) in path.iter().enumerate() {
                let current_index = path_index.saturating_add(1);
                current[current_index] =
                    previous[path_index] && matches_segment(segment, path_segment);
            }
        }
        previous = current;
    }

    previous[path.len()]
}

/// Matches a single path segment, where `*` matches any run of characters and `?`
/// matches a single character (neither crosses a `/`, since segments are split out).
fn matches_segment(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    match_segment_chars(&pattern, &text)
}

fn match_segment_chars(pattern: &[char], text: &[char]) -> bool {
    let mut previous = vec![false; text.len().saturating_add(1)];
    previous[0] = true;

    for token in pattern {
        let mut current = vec![false; text.len().saturating_add(1)];
        if *token == '*' {
            current[0] = previous[0];
            for (text_index, _) in text.iter().enumerate() {
                let current_index = text_index.saturating_add(1);
                current[current_index] = previous[current_index] || current[text_index];
            }
        } else {
            for (text_index, character) in text.iter().enumerate() {
                let current_index = text_index.saturating_add(1);
                current[current_index] =
                    previous[text_index] && (*token == '?' || token == character);
            }
        }
        previous = current;
    }

    previous[text.len()]
}

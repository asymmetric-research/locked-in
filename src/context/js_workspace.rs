use std::fs;
use std::path::Path;

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

    if let Ok(content) = fs::read_to_string(base.join("package.json")) {
        patterns.extend(patterns_from_package_json(&content));
    }
    if let Ok(content) = fs::read_to_string(base.join("pnpm-workspace.yaml")) {
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
    for pattern in patterns {
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
                patterns.push(unquote(item.trim()).to_string());
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
    let Some((&segment, pattern_rest)) = pattern.split_first() else {
        return path.is_empty();
    };

    if segment == "**" {
        return (0..=path.len()).any(|skipped| match_segments(pattern_rest, &path[skipped..]));
    }

    let Some((&head, path_rest)) = path.split_first() else {
        return false;
    };

    matches_segment(segment, head) && match_segments(pattern_rest, path_rest)
}

/// Matches a single path segment, where `*` matches any run of characters and `?`
/// matches a single character (neither crosses a `/`, since segments are split out).
fn matches_segment(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    match_segment_chars(&pattern, &text)
}

fn match_segment_chars(pattern: &[char], text: &[char]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((&'*', pattern_rest)) => {
            match_segment_chars(pattern_rest, text)
                || text
                    .split_first()
                    .is_some_and(|(_, text_rest)| match_segment_chars(pattern, text_rest))
        }
        Some((&'?', pattern_rest)) => text
            .split_first()
            .is_some_and(|(_, text_rest)| match_segment_chars(pattern_rest, text_rest)),
        Some((&expected, pattern_rest)) => text.split_first().is_some_and(|(&actual, text_rest)| {
            actual == expected && match_segment_chars(pattern_rest, text_rest)
        }),
    }
}

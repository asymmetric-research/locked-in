use crate::Violation;
use crate::parsers::MAX_VIOLATIONS_PER_FILE;
use crate::rules::{check_bun, check_npm, check_pnpm, check_yarn};
use crate::scanner::LintContext;
use std::collections::{HashMap, HashSet};

pub fn check_package_json(content: &str, lint_context: &LintContext) -> Vec<Violation> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(scripts) = value.get("scripts").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let script_lines = find_script_lines(content, scripts.keys().map(String::as_str));

    let mut violations = Vec::new();
    for (name, script_value) in scripts {
        let Some(script) = script_value.as_str() else {
            continue;
        };
        let line_num = script_lines.get(name.as_str()).copied().unwrap_or(1);

        violations.extend(check_npm(script, line_num));
        violations.extend(check_pnpm(script, line_num));
        violations.extend(check_yarn(script, line_num));
        violations.extend(check_bun(
            script,
            line_num,
            lint_context.bun_frozen_lockfile,
        ));
        if violations.len() >= MAX_VIOLATIONS_PER_FILE {
            violations.truncate(MAX_VIOLATIONS_PER_FILE);
            violations.push(Violation::error(
                0,
                format!("Stopped after {MAX_VIOLATIONS_PER_FILE} violations to bound resource use"),
                "Additional violations were not retained",
                "scan-violation-limit",
            ));
            break;
        }
    }

    violations.sort_by_key(|v| v.line_num);
    violations
}

fn find_script_lines<'a>(
    content: &str,
    script_names: impl IntoIterator<Item = &'a str>,
) -> HashMap<String, usize> {
    let mut remaining: HashSet<&str> = script_names.into_iter().collect();
    let mut locations = HashMap::new();
    let mut in_scripts = false;
    for (idx, line) in content.lines().enumerate() {
        if !in_scripts {
            if line.contains("\"scripts\"") {
                in_scripts = true;
            }
            continue;
        }
        if let Some(key) = leading_json_key(line)
            && remaining.remove(key.as_str())
        {
            locations.insert(key, idx.saturating_add(1));
            if remaining.is_empty() {
                break;
            }
        }
    }
    locations
}

fn leading_json_key(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('"') {
        return None;
    }

    let mut escaped = false;
    for (index, character) in trimmed.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            let literal = trimmed.get(..=index)?;
            let rest = trimmed.get(index.saturating_add(1)..)?.trim_start();
            if rest.starts_with(':') {
                return serde_json::from_str(literal).ok();
            }
            return None;
        }
    }
    None
}

use crate::Violation;
use crate::rules::{check_bun, check_npm, check_pnpm, check_yarn};
use crate::scanner::LintContext;

pub fn check_package_json(content: &str, lint_context: &LintContext) -> Vec<Violation> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(scripts) = value.get("scripts").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut violations = Vec::new();
    for (name, script_value) in scripts {
        let Some(script) = script_value.as_str() else {
            continue;
        };
        let line_num = find_script_line(content, name).unwrap_or(1);

        violations.extend(check_npm(script, line_num));
        violations.extend(check_pnpm(script, line_num));
        violations.extend(check_yarn(script, line_num));
        violations.extend(check_bun(
            script,
            line_num,
            lint_context.bun_frozen_lockfile,
        ));
    }

    violations.sort_by_key(|v| v.line_num);
    violations
}

pub fn find_script_line(content: &str, script_name: &str) -> Option<usize> {
    let needle = format!("\"{script_name}\"");
    let mut in_scripts = false;
    for (idx, line) in content.lines().enumerate() {
        if !in_scripts {
            if line.contains("\"scripts\"") {
                in_scripts = true;
            }
            continue;
        }
        if line.contains(&needle) {
            return Some(idx.saturating_add(1));
        }
    }
    None
}

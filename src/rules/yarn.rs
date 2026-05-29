use crate::Violation;
use regex::Regex;
use std::sync::LazyLock;

use super::npm::has_version_pin;

static YARN_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\byarn(\s+install)?(\s+)?($|&&|;|\||#)").unwrap());
static YARN_FROZEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"--(frozen-lockfile|immutable)").unwrap());
static YARN_ADD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\byarn\s+(global\s+)?add\s").unwrap());

pub fn check_yarn(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    if YARN_INSTALL_RE.is_match(line) && !YARN_FROZEN_RE.is_match(line) {
        violations.push(Violation {
            line_num,
            message: "Use 'yarn install --frozen-lockfile' to respect lockfile".to_string(),
            line_content: line.trim().to_string(),
            rule_id: Some("yarn-frozen-lockfile".to_string()),
        });
    }

    if YARN_ADD_RE.is_match(line) && !has_version_pin(line) {
        violations.push(Violation {
            line_num,
            message:
                "yarn package installation without version pin (use 'yarn add package@version')"
                    .to_string(),
            line_content: line.trim().to_string(),
            rule_id: Some("yarn-version-pin".to_string()),
        });
    }

    violations
}

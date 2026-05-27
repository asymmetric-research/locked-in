use crate::Violation;
use regex::Regex;
use std::sync::LazyLock;

static NPM_CI_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bnpm\s+ci\b").unwrap());
static NPM_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bnpm\s+(install|i)(\s|$)").unwrap());
static VERSION_PIN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@[0-9]+\.[0-9]+").unwrap());
static BARE_NPM_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bnpm\s+(install|i)(\s+)?($|&&|;|\||#)").unwrap());

pub fn check_npm(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    if NPM_CI_RE.is_match(line) {
        return violations;
    }

    if NPM_INSTALL_RE.is_match(line) {
        if VERSION_PIN_RE.is_match(line) {
            return violations;
        }

        if BARE_NPM_INSTALL_RE.is_match(line) {
            violations.push(Violation {
                line_num,
                message: "Use 'npm ci' instead of 'npm install' for lockfile-based installations"
                    .to_string(),
                line_content: line.trim().to_string(),
                rule_id: Some("npm-install-bare".to_string()),
            });
        } else {
            violations.push(Violation {
                line_num,
                message:
                    "npm package installation without version pin (use 'npm i package@version')"
                        .to_string(),
                line_content: line.trim().to_string(),
                rule_id: Some("npm-version-pin".to_string()),
            });
        }
    }

    violations
}

pub(super) fn has_version_pin(line: &str) -> bool {
    VERSION_PIN_RE.is_match(line)
}

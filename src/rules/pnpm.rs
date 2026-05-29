use crate::Violation;
use regex::Regex;
use std::sync::LazyLock;

use super::npm::has_version_pin;

static PNPM_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bpnpm\s+install\b").unwrap());
static PNPM_ADD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bpnpm\s+add\s").unwrap());

pub fn check_pnpm(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    if PNPM_INSTALL_RE.is_match(line) && !line.contains("--frozen-lockfile") {
        violations.push(Violation {
            line_num,
            message: "Use 'pnpm install --frozen-lockfile' to respect lockfile".to_string(),
            line_content: line.trim().to_string(),
            rule_id: Some("pnpm-frozen-lockfile".to_string()),
        });
    }

    if PNPM_ADD_RE.is_match(line) && !has_version_pin(line) {
        violations.push(Violation {
            line_num,
            message:
                "pnpm package installation without version pin (use 'pnpm add package@version')"
                    .to_string(),
            line_content: line.trim().to_string(),
            rule_id: Some("pnpm-version-pin".to_string()),
        });
    }

    violations
}

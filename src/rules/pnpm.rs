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
        violations.push(Violation::error(
            line_num,
            "Use 'pnpm install --frozen-lockfile' to respect lockfile",
            line.trim(),
            "pnpm-frozen-lockfile",
        ));
    }

    if PNPM_ADD_RE.is_match(line) && !has_version_pin(line) {
        violations.push(Violation::error(
            line_num,
            "pnpm package installation without version pin (use 'pnpm add package@version')",
            line.trim(),
            "pnpm-version-pin",
        ));
    }

    violations
}

use crate::{Violation, ViolationKind};
use regex::Regex;
use std::sync::LazyLock;

use super::npm::has_version_pin;

static PNPM_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bpnpm\s+install\b").unwrap());
static PNPM_ADD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bpnpm\s+add\s").unwrap());

pub fn check_pnpm(line: &str, line_num: usize) -> Vec<Violation> {
    let mut violations = Vec::new();

    if PNPM_INSTALL_RE.is_match(line) && !line.contains("--frozen-lockfile") {
        violations.push(Violation::new(
            ViolationKind::PnpmFrozenLockfile,
            line_num,
            line.trim(),
        ));
    }

    if PNPM_ADD_RE.is_match(line) && !has_version_pin(line) {
        violations.push(Violation::new(
            ViolationKind::PnpmVersionPin,
            line_num,
            line.trim(),
        ));
    }

    violations
}

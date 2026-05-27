use crate::Violation;
use regex::Regex;
use std::sync::LazyLock;

use super::npm::has_version_pin;

static BUN_INSTALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bbun\s+install\b").unwrap());
static BUN_ADD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bbun\s+add\s").unwrap());

pub fn check_bun(line: &str, line_num: usize, bun_frozen_lockfile: bool) -> Vec<Violation> {
    let mut violations = Vec::new();

    if BUN_INSTALL_RE.is_match(line) && !line.contains("--frozen-lockfile") && !bun_frozen_lockfile
    {
        violations.push(Violation {
                line_num,
                message: "Use 'bun install --frozen-lockfile' unless repo-local bunfig.toml sets '[install].frozenLockfile = true' (https://bun.com/docs/runtime/bunfig#install-frozenlockfile)".to_string(),
                line_content: line.trim().to_string(),
                rule_id: Some("bun-frozen-lockfile".to_string()),
            });
    }

    if BUN_ADD_RE.is_match(line) && !has_version_pin(line) {
        violations.push(Violation {
            line_num,
            message: "bun package installation without version pin (use 'bun add package@version')"
                .to_string(),
            line_content: line.trim().to_string(),
            rule_id: Some("bun-version-pin".to_string()),
        });
    }

    violations
}

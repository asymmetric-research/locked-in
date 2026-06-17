use crate::{Violation, ViolationKind};
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
        violations.push(Violation::new(
            ViolationKind::YarnFrozenLockfile,
            line_num,
            line.trim(),
        ));
    }

    if YARN_ADD_RE.is_match(line) && !has_version_pin(line) {
        violations.push(Violation::new(
            ViolationKind::YarnVersionPin,
            line_num,
            line.trim(),
        ));
    }

    violations
}

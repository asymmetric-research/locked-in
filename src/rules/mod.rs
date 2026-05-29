mod bun;
mod npm;
mod pnpm;
mod yarn;

pub use bun::check_bun;
pub use npm::check_npm;
pub use pnpm::check_pnpm;
pub use yarn::check_yarn;

use crate::Violation;
use crate::scanner::LintContext;

pub trait Rule {
    fn check(&self, line: &str, line_num: usize, context: &LintContext) -> Vec<Violation>;
}

pub struct JavaScriptRules;

impl Rule for JavaScriptRules {
    fn check(&self, line: &str, line_num: usize, context: &LintContext) -> Vec<Violation> {
        let mut violations = Vec::new();
        violations.extend(check_npm(line, line_num));
        violations.extend(check_pnpm(line, line_num));
        violations.extend(check_yarn(line, line_num));
        violations.extend(check_bun(line, line_num, context.bun_frozen_lockfile));
        violations
    }
}

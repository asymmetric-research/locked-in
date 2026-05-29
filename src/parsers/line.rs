use crate::Violation;
use crate::ignore_directives::{
    CommentStyle, IgnoreDirective, is_ignore_directive, split_inline_ignore,
};
use crate::parsers::markdown::should_lint_markdown_code_block as markdown_should_lint_code_block;
use crate::parsers::package_json::check_package_json;
use crate::rules::{JavaScriptRules, Rule};
use crate::scanner::LintContext;

pub fn check_file(
    content: &str,
    lint_context: &LintContext,
    comment_style: &CommentStyle,
) -> Vec<Violation> {
    if lint_context.is_package_json {
        return check_package_json(content, lint_context);
    }

    let rules = JavaScriptRules;
    let mut violations = Vec::new();
    let mut in_code_block = false;
    let mut lint_code_block = false;
    let mut skip_next: Option<IgnoreDirective> = None;

    for (line_num, line) in content.lines().enumerate() {
        let line_num = line_num.saturating_add(1);

        if lint_context.is_markdown && line.trim().starts_with("```") {
            if in_code_block {
                lint_code_block = false;
            } else {
                lint_code_block = should_lint_markdown_code_block(line);
            }
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block && !lint_code_block {
            continue;
        }

        let (effective_line, inline_skip): (&str, Option<IgnoreDirective>) =
            if let Some((content, directive)) = split_inline_ignore(line, comment_style) {
                (content, Some(directive))
            } else {
                (line, None)
            };

        if inline_skip.is_none()
            && let Some(directive) = is_ignore_directive(effective_line, comment_style)
        {
            skip_next = Some(directive);
            continue;
        }

        let prev_skip = skip_next.take();

        if prev_skip
            .as_ref()
            .is_some_and(|d| matches!(d, IgnoreDirective::All))
            || inline_skip
                .as_ref()
                .is_some_and(|d| matches!(d, IgnoreDirective::All))
        {
            continue;
        }

        if is_comment_or_placeholder(effective_line) {
            continue;
        }

        let mut line_violations = rules.check(effective_line, line_num, lint_context);

        if let Some(IgnoreDirective::Specific(rule)) = &prev_skip {
            line_violations.retain(|v| v.rule_id.as_deref() != Some(rule.as_str()));
        }
        if let Some(IgnoreDirective::Specific(rule)) = &inline_skip {
            line_violations.retain(|v| v.rule_id.as_deref() != Some(rule.as_str()));
        }

        violations.extend(line_violations);
    }

    violations
}

pub fn is_comment_or_placeholder(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('#')
        || trimmed.contains("<package>")
        || trimmed.contains("<version>")
        || trimmed.starts_with('`')
        || trimmed.starts_with('>')
        || trimmed.starts_with('-')
}

pub fn should_lint_markdown_code_block(fence_line: &str) -> bool {
    markdown_should_lint_code_block(fence_line)
}

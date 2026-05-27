use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy)]
pub struct CommentStyle {
    pub prefix: &'static str,
    pub suffix: &'static str,
}

pub enum IgnoreDirective {
    All,
    Specific(String),
}

static IGNORE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"locked-in:\s*ignore(?:\s*\[([^\]]*)\])?").unwrap());

pub fn is_ignore_directive(line: &str, style: &CommentStyle) -> Option<IgnoreDirective> {
    let trimmed = line.trim();

    if !trimmed.starts_with(style.prefix) {
        return None;
    }

    if !style.suffix.is_empty() && !trimmed.ends_with(style.suffix) {
        return None;
    }

    let inner = trimmed.strip_prefix(style.prefix).and_then(|rest| {
        if style.suffix.is_empty() {
            Some(rest.trim())
        } else {
            rest.strip_suffix(style.suffix).map(str::trim)
        }
    });

    inner.and_then(|content| {
        IGNORE_RE.find(content).map(|m| {
            let caps = IGNORE_RE.captures(m.as_str());
            caps.and_then(|c| c.get(1))
                .map_or(IgnoreDirective::All, |rule_match| {
                    let rule = rule_match.as_str().trim();
                    if rule.is_empty() {
                        IgnoreDirective::All
                    } else {
                        IgnoreDirective::Specific(rule.to_string())
                    }
                })
        })
    })
}

pub fn split_inline_ignore<'a>(
    line: &'a str,
    style: &CommentStyle,
) -> Option<(&'a str, IgnoreDirective)> {
    let trimmed = line.trim();

    if trimmed.starts_with(style.prefix) {
        return None;
    }

    let directive_match = IGNORE_RE.find(line)?;
    let directive_start = directive_match.start();
    let before_directive = &line[..directive_start];
    let prefix_pos = before_directive.rfind(style.prefix)?;

    if line[..prefix_pos].trim().is_empty() {
        return None;
    }

    let comment_part = &line[prefix_pos..];
    is_ignore_directive(comment_part, style).map(|directive| (&line[..prefix_pos], directive))
}

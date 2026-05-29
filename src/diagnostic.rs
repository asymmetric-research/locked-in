use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug)]
pub struct Violation {
    pub severity: Severity,
    pub line_num: usize,
    pub message: String,
    pub line_content: String,
    pub rule_id: Option<String>,
}

impl Violation {
    pub fn error(
        line_num: usize,
        message: impl Into<String>,
        line_content: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Error,
            line_num,
            message: message.into(),
            line_content: line_content.into(),
            rule_id: Some(rule_id.into()),
        }
    }

    pub fn warning(
        message: impl Into<String>,
        line_content: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            line_num: 0,
            message: message.into(),
            line_content: line_content.into(),
            rule_id: Some(rule_id.into()),
        }
    }
}

pub struct LintResult {
    pub violations_found: usize,
    pub warnings_found: usize,
    pub files_checked: usize,
}

pub struct FileLintResult {
    pub path: PathBuf,
    pub violations: Vec<Violation>,
}

use std::path::PathBuf;

#[derive(Debug)]
pub struct Violation {
    pub line_num: usize,
    pub message: String,
    pub line_content: String,
    pub rule_id: Option<String>,
}

pub struct LintResult {
    pub violations_found: usize,
    pub files_checked: usize,
}

pub struct FileLintResult {
    pub path: PathBuf,
    pub violations: Vec<Violation>,
}

pub fn should_lint_markdown_code_block(fence_line: &str) -> bool {
    let info = fence_line.trim().trim_start_matches("```").trim();
    if info.is_empty() {
        return false;
    }

    let language = info.split_whitespace().next().unwrap_or("");
    matches!(
        language,
        "bash" | "sh" | "shell" | "zsh" | "console" | "terminal"
    )
}

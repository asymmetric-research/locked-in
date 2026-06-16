use crate::{FileLintResult, LintResult, Severity, Violation};
use colored::Colorize;
use serde_json::{Value, json};
use std::io::{self, Write};

/// Version of the `--format json` document shape; bump on any breaking schema change.
const JSON_SCHEMA_VERSION: u32 = 1;

/// Violations to display for one file: all of them, or errors-only under `--quiet`.
fn displayed_violations(violations: &[Violation], quiet: bool) -> Vec<&Violation> {
    violations
        .iter()
        .filter(|violation| !quiet || violation.severity == Severity::Error)
        .collect()
}

/// Human-facing label (text format).
const fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "Error",
        Severity::Warning => "Warning",
    }
}

/// Stable wire value (JSON format) — intentionally independent of the display label.
const fn severity_json(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// Render findings as human-readable text. Files with nothing left to show (e.g. a
/// warning-only file under `--quiet`) are skipped entirely.
pub fn render_text(w: &mut dyn Write, results: &[FileLintResult], quiet: bool) -> io::Result<()> {
    for file in results {
        let displayed = displayed_violations(&file.violations, quiet);
        if displayed.is_empty() {
            continue;
        }

        let has_errors = displayed
            .iter()
            .any(|violation| violation.severity == Severity::Error);
        if has_errors {
            writeln!(w, "{}", format!("✗ {}", file.path.display()).red())?;
        } else {
            writeln!(w, "{}", format!("! {}", file.path.display()).yellow())?;
        }

        for violation in displayed {
            let label = severity_label(violation.severity);
            let location = if violation.line_num == 0 {
                label.to_string()
            } else {
                format!("{label} line {}", violation.line_num)
            };
            writeln!(
                w,
                "  {} {}",
                format!("{location}:").yellow(),
                violation.message
            )?;
            writeln!(w, "  {} {}", ">".blue(), violation.line_content)?;
        }
        writeln!(w)?;
    }
    Ok(())
}

/// Render findings as a single JSON document. Counts always reflect the full scan;
/// `--quiet` filters warnings out of the per-file arrays for consistency with text.
/// Note: `files` lists only files with displayed findings, so `files.len()` is not
/// necessarily `files_checked`.
pub fn render_json(w: &mut dyn Write, result: &LintResult, quiet: bool) -> io::Result<()> {
    let files: Vec<Value> = result
        .results
        .iter()
        .filter_map(|file| {
            let displayed = displayed_violations(&file.violations, quiet);
            if displayed.is_empty() {
                return None;
            }
            let violations: Vec<Value> = displayed
                .iter()
                .map(|violation| {
                    let mut obj = serde_json::Map::new();
                    obj.insert(
                        "severity".to_string(),
                        json!(severity_json(violation.severity)),
                    );
                    obj.insert("rule_id".to_string(), json!(violation.rule_id));
                    if violation.line_num != 0 {
                        obj.insert("line".to_string(), json!(violation.line_num));
                    }
                    obj.insert("message".to_string(), json!(violation.message));
                    obj.insert("line_content".to_string(), json!(violation.line_content));
                    Value::Object(obj)
                })
                .collect();
            Some(json!({
                "path": file.path.display().to_string(),
                "violations": violations,
            }))
        })
        .collect();

    let doc = json!({
        "schema_version": JSON_SCHEMA_VERSION,
        "files_checked": result.files_checked,
        "violations_found": result.violations_found,
        "warnings_found": result.warnings_found,
        "files": files,
    });

    serde_json::to_string_pretty(&doc).map_or(Ok(()), |text| writeln!(w, "{text}"))
}

#[cfg(test)]
mod tests {
    use super::{render_json, render_text};
    use crate::{FileLintResult, LintResult, Violation};
    use std::path::PathBuf;

    fn sample() -> LintResult {
        let results = vec![FileLintResult {
            path: PathBuf::from("Dockerfile"),
            violations: vec![
                Violation::error(2, "use npm ci", "RUN npm install", "npm-install-bare"),
                Violation::warning("missing lockfile", "package.json", "missing-tracked-lockfile"),
            ],
        }];
        LintResult {
            results,
            violations_found: 1,
            warnings_found: 1,
            files_checked: 1,
        }
    }

    fn text(quiet: bool) -> String {
        let mut buf = Vec::new();
        render_text(&mut buf, &sample().results, quiet).unwrap();
        String::from_utf8(buf).unwrap()
    }

    fn json_doc(quiet: bool) -> serde_json::Value {
        let mut buf = Vec::new();
        render_json(&mut buf, &sample(), quiet).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    #[test]
    fn text_shows_errors_and_warnings() {
        let out = text(false);
        assert!(out.contains("Error line 2:"));
        assert!(out.contains("Warning:"));
        assert!(out.contains("use npm ci"));
    }

    #[test]
    fn text_quiet_drops_warning_lines() {
        let out = text(true);
        assert!(out.contains("Error line 2:"));
        assert!(!out.contains("Warning:"), "warnings suppressed: {out}");
    }

    #[test]
    fn json_has_counts_and_per_file_violations() {
        let doc = json_doc(false);

        assert_eq!(doc["schema_version"], 1);
        assert_eq!(doc["violations_found"], 1);
        assert_eq!(doc["warnings_found"], 1);
        assert_eq!(doc["files"][0]["path"], "Dockerfile");
        let first = &doc["files"][0]["violations"][0];
        assert_eq!(first["severity"], "error");
        assert_eq!(first["line"], 2);
        assert_eq!(first["rule_id"], "npm-install-bare");
    }

    #[test]
    fn json_quiet_filters_warnings_but_keeps_counts() {
        let doc = json_doc(true);

        assert_eq!(doc["warnings_found"], 1, "count reflects full scan");
        let severities: Vec<&str> = doc["files"][0]["violations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["severity"].as_str().unwrap())
            .collect();
        assert_eq!(severities, ["error"], "warnings filtered from arrays");
    }

    #[test]
    fn json_omits_line_for_file_level_warnings() {
        let doc = json_doc(false);
        // The warning (line_num 0) must not carry a "line" key.
        let warning = &doc["files"][0]["violations"][1];
        assert_eq!(warning["severity"], "warning");
        assert!(warning.get("line").is_none(), "no line for file-level warning");
    }
}

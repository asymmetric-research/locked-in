use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// Distinctive substring from a `git-metadata-unavailable` warning line (`src/scanner.rs`).
const WARNING_MARKER: &str = "Git metadata not found";

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("locked-in-it-{name}-{nonce}"));
    fs::create_dir(&path).unwrap();
    path
}

/// A repo with exactly one error (bare `npm install` in a Dockerfile) and one warning
/// (no `.git`, so tracked-lockfile validation is skipped with a warning).
fn fixture_error_and_warning(name: &str) -> PathBuf {
    let root = temp_dir(name);
    fs::write(root.join("Dockerfile"), "FROM node:20\nRUN npm install\n").unwrap();
    root
}

/// A repo with zero errors and exactly one warning (empty dir, no `.git`).
fn fixture_warning_only(name: &str) -> PathBuf {
    temp_dir(name)
}

struct Output {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(root: &PathBuf, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_locked-in"))
        .args(args)
        .arg(root)
        .env("NO_COLOR", "1")
        .output()
        .expect("failed to run locked-in binary");
    Output {
        stdout: String::from_utf8(output.stdout).expect("stdout was not valid utf-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr was not valid utf-8"),
        code: output.status.code().expect("process had no exit code"),
    }
}

#[test]
fn findings_go_to_stdout_chrome_goes_to_stderr() {
    let root = fixture_error_and_warning("streams");
    let out = run(&root, &[]);
    fs::remove_dir_all(&root).unwrap();

    // Findings on stdout.
    assert!(out.stdout.contains("Error"), "error finding on stdout: {}", out.stdout);
    assert!(
        out.stdout.contains(WARNING_MARKER),
        "warning finding on stdout: {}",
        out.stdout
    );
    // Progress + summary chrome on stderr, not stdout.
    assert!(
        out.stderr.contains("Checking for package manager violations"),
        "progress line on stderr: {}",
        out.stderr
    );
    assert!(out.stderr.contains("violation(s)"), "summary on stderr: {}", out.stderr);
    assert!(!out.stdout.contains("violation(s)"), "summary not on stdout: {}", out.stdout);
    assert_eq!(out.code, 1, "errors must fail");
}

#[test]
fn quiet_suppresses_warning_findings_but_keeps_errors() {
    let root = fixture_error_and_warning("quiet");
    let out = run(&root, &["--quiet"]);
    fs::remove_dir_all(&root).unwrap();

    assert!(out.stdout.contains("Error"), "errors must still print: {}", out.stdout);
    assert!(
        !out.stdout.contains(WARNING_MARKER),
        "warning findings must be suppressed: {}",
        out.stdout
    );
    assert!(out.stderr.contains("violation(s)"), "summary still on stderr: {}", out.stderr);
    assert_eq!(out.code, 1, "exit code unchanged by --quiet");
}

#[test]
fn max_warnings_zero_fails_on_a_warning_with_no_errors() {
    let root = fixture_warning_only("max-warnings");
    let out = run(&root, &["--max-warnings", "0"]);
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(out.code, 1, "any warning must fail at --max-warnings 0");
}

#[test]
fn warnings_do_not_fail_by_default() {
    let root = fixture_warning_only("default-warning");
    let out = run(&root, &[]);
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(out.code, 0, "warnings alone must not fail with no flags");
}

#[test]
fn fail_on_warning_matches_max_warnings_zero() {
    let root_a = fixture_warning_only("fail-on-a");
    let a = run(&root_a, &["--fail-on", "warning"]);
    fs::remove_dir_all(&root_a).unwrap();

    let root_b = fixture_warning_only("fail-on-b");
    let b = run(&root_b, &["--max-warnings", "0"]);
    fs::remove_dir_all(&root_b).unwrap();

    assert_eq!(a.code, 1);
    assert_eq!(a.code, b.code, "--fail-on warning must alias --max-warnings 0");
}

#[test]
fn no_summary_omits_trailing_block_but_keeps_progress_line() {
    let root = fixture_error_and_warning("no-summary");
    let out = run(&root, &["--no-summary"]);
    fs::remove_dir_all(&root).unwrap();

    assert!(
        out.stderr.contains("Checking for package manager violations"),
        "progress line must remain on stderr: {}",
        out.stderr
    );
    assert!(!out.stderr.contains("═══"), "summary divider must be gone: {}", out.stderr);
    assert!(
        !out.stderr.contains("violation(s)"),
        "summary block must be gone: {}",
        out.stderr
    );
    assert_eq!(out.code, 1, "exit code unaffected by --no-summary");
}

#[test]
fn quiet_and_max_warnings_compose() {
    let root = fixture_error_and_warning("compose");
    let out = run(&root, &["--quiet", "--max-warnings", "0"]);
    fs::remove_dir_all(&root).unwrap();

    assert!(out.stdout.contains("Error"), "errors must print: {}", out.stdout);
    assert!(
        !out.stdout.contains(WARNING_MARKER),
        "warnings must be suppressed under --quiet: {}",
        out.stdout
    );
    assert_eq!(out.code, 1, "exit 1 from the error (and the warning threshold)");
}

#[test]
fn json_format_emits_parseable_document_with_counts() {
    let root = fixture_error_and_warning("json");
    let out = run(&root, &["--format", "json"]);
    fs::remove_dir_all(&root).unwrap();

    // stdout must be pure JSON (no chrome).
    assert!(!out.stdout.contains("Checking"), "no chrome on stdout: {}", out.stdout);
    let doc: serde_json::Value =
        serde_json::from_str(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(doc["violations_found"], 1);
    assert_eq!(doc["warnings_found"], 1);
    assert!(doc["files"].as_array().is_some_and(|f| !f.is_empty()));
    assert_eq!(out.code, 1);
}

#[test]
fn quiet_json_drops_warnings_but_keeps_counts() {
    let root = fixture_error_and_warning("json-quiet");
    let out = run(&root, &["--format", "json", "--quiet"]);
    fs::remove_dir_all(&root).unwrap();

    let doc: serde_json::Value =
        serde_json::from_str(&out.stdout).expect("stdout must be valid JSON");
    // Counts always reflect the full scan.
    assert_eq!(doc["warnings_found"], 1);
    // But no warning severities appear in the per-file arrays.
    let severities: Vec<String> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|f| f["violations"].as_array().unwrap())
        .map(|v| v["severity"].as_str().unwrap().to_string())
        .collect();
    assert!(
        severities.iter().all(|s| s == "error"),
        "only errors under --quiet: {severities:?}"
    );
}

#[test]
fn help_text_goes_to_stdout() {
    let root = fixture_warning_only("help-stream");
    let out = run(&root, &["--help"]);
    fs::remove_dir_all(&root).unwrap();

    assert!(out.stdout.contains("Usage: locked-in"), "help on stdout: {}", out.stdout);
    assert!(out.stderr.is_empty(), "nothing on stderr: {:?}", out.stderr);
    assert_eq!(out.code, 0);
}

#[test]
fn usage_errors_go_to_stderr() {
    let root = fixture_warning_only("err-stream");
    let out = run(&root, &["--nope"]);
    fs::remove_dir_all(&root).unwrap();

    assert!(out.stderr.contains("Unknown option"), "error on stderr: {}", out.stderr);
    assert!(out.stdout.is_empty(), "nothing on stdout: {:?}", out.stdout);
    assert_eq!(out.code, 2);
}

/// Piping stdout into a reader that closes early must exit cleanly with the conventional
/// SIGPIPE code (141), never panicking (101) and never masking the failure as success (0).
/// The fixture is large enough to overflow the OS pipe buffer, so the child genuinely
/// blocks on a write and hits `EPIPE`.
#[test]
fn broken_pipe_exits_141_not_panic() {
    let root = temp_dir("broken-pipe");
    // Each finding is well over a hundred bytes; ~3000 of them dwarfs the 64 KB pipe buffer.
    let mut dockerfile = String::from("FROM node:20\n");
    for _ in 0..3000 {
        dockerfile.push_str("RUN npm install\n");
    }
    fs::write(root.join("Dockerfile"), dockerfile).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_locked-in"))
        .arg(&root)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");

    // Read a single byte to ensure the child has started writing, then close the read end.
    {
        let mut out = child.stdout.take().expect("piped stdout");
        let mut one = [0u8; 1];
        let _ = out.read(&mut one);
        // `out` drops here, closing the read end while the child is mid-write.
    }
    let status = child.wait().expect("wait");
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(
        status.code(),
        Some(141),
        "broken pipe must exit 141 (not 101 panic, not 0 masking findings)"
    );
}

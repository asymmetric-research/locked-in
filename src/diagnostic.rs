use crate::context::git::GitIndexStatus;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// The structured identity of a violation.
///
/// The logic layer emits these; `rule_id` and `severity` are derived from the kind, and the
/// user-facing message is produced separately by the presentation layer (`crate::messages`).
/// Variants carry any data the message needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationKind {
    NpmInstallBare,
    NpmVersionPin,
    PnpmFrozenLockfile,
    PnpmVersionPin,
    YarnFrozenLockfile,
    YarnVersionPin,
    BunFrozenLockfile,
    BunVersionPin,
    MissingTrackedLockfile { candidates: Vec<String> },
    GitMetadataUnavailable(GitIndexStatus),
}

impl ViolationKind {
    /// Stable identifier used for ignore-directive suppression and JSON output.
    #[must_use]
    pub const fn rule_id(&self) -> &'static str {
        match self {
            Self::NpmInstallBare => "npm-install-bare",
            Self::NpmVersionPin => "npm-version-pin",
            Self::PnpmFrozenLockfile => "pnpm-frozen-lockfile",
            Self::PnpmVersionPin => "pnpm-version-pin",
            Self::YarnFrozenLockfile => "yarn-frozen-lockfile",
            Self::YarnVersionPin => "yarn-version-pin",
            Self::BunFrozenLockfile => "bun-frozen-lockfile",
            Self::BunVersionPin => "bun-version-pin",
            Self::MissingTrackedLockfile { .. } => "missing-tracked-lockfile",
            Self::GitMetadataUnavailable(_) => "git-metadata-unavailable",
        }
    }

    /// Whether this kind fails the run (error) or is informational (warning).
    #[must_use]
    pub const fn severity(&self) -> Severity {
        match self {
            Self::MissingTrackedLockfile { .. } | Self::GitMetadataUnavailable(_) => {
                Severity::Warning
            }
            _ => Severity::Error,
        }
    }
}

#[derive(Debug)]
pub struct Violation {
    pub kind: ViolationKind,
    pub line_num: usize,
    pub line_content: String,
}

impl Violation {
    pub fn new(kind: ViolationKind, line_num: usize, line_content: impl Into<String>) -> Self {
        Self {
            kind,
            line_num,
            line_content: line_content.into(),
        }
    }

    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.kind.severity()
    }

    #[must_use]
    pub const fn rule_id(&self) -> &'static str {
        self.kind.rule_id()
    }
}

pub struct LintResult {
    pub files: Vec<FileLintResult>,
    pub violations_found: usize,
    pub warnings_found: usize,
    pub files_checked: usize,
}

pub struct FileLintResult {
    pub path: PathBuf,
    pub violations: Vec<Violation>,
}

use crate::context::git::GitIndexStatus;
use crate::{Severity, ViolationKind};

/// The human-readable message for a violation. This is the presentation layer's job — the
/// logic layer (`rules`, `scanner`) only emits structured `ViolationKind`s.
pub fn violation_message(kind: &ViolationKind) -> String {
    match kind {
        ViolationKind::NpmInstallBare => {
            "Use 'npm ci' instead of 'npm install' for lockfile-based installations".to_string()
        }
        ViolationKind::NpmVersionPin => {
            "npm package installation without version pin (use 'npm i package@version')".to_string()
        }
        ViolationKind::PnpmFrozenLockfile => {
            "Use 'pnpm install --frozen-lockfile' to respect lockfile".to_string()
        }
        ViolationKind::PnpmVersionPin => {
            "pnpm package installation without version pin (use 'pnpm add package@version')"
                .to_string()
        }
        ViolationKind::YarnFrozenLockfile => {
            "Use 'yarn install --frozen-lockfile' to respect lockfile".to_string()
        }
        ViolationKind::YarnVersionPin => {
            "yarn package installation without version pin (use 'yarn add package@version')"
                .to_string()
        }
        ViolationKind::BunFrozenLockfile => {
            "Use 'bun install --frozen-lockfile' unless repo-local bunfig.toml sets '[install].frozenLockfile = true' (https://bun.com/docs/runtime/bunfig#install-frozenlockfile)"
                .to_string()
        }
        ViolationKind::BunVersionPin => {
            "bun package installation without version pin (use 'bun add package@version')"
                .to_string()
        }
        ViolationKind::MissingTrackedLockfile { candidates } => {
            format!(
                "Tracked manifest is missing a tracked lockfile: {}",
                candidates.join(", ")
            )
        }
        ViolationKind::GitMetadataUnavailable(status) => match status {
            GitIndexStatus::MissingMetadata => {
                "Git metadata not found; skipping tracked lockfile validation".to_string()
            }
            GitIndexStatus::MissingIndex => {
                "Git index not found; skipping tracked lockfile validation".to_string()
            }
            GitIndexStatus::UnsupportedIndex => {
                "Git index could not be parsed; skipping tracked lockfile validation".to_string()
            }
        },
    }
}

/// Human-facing severity label used in text output.
pub const fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "Error",
        Severity::Warning => "Warning",
    }
}

#[cfg(test)]
mod tests {
    use super::{severity_label, violation_message};
    use crate::context::git::GitIndexStatus;
    use crate::{Severity, ViolationKind};

    #[test]
    fn static_kind_message_is_exact() {
        assert_eq!(
            violation_message(&ViolationKind::NpmInstallBare),
            "Use 'npm ci' instead of 'npm install' for lockfile-based installations"
        );
    }

    #[test]
    fn missing_lockfile_joins_candidates() {
        let kind = ViolationKind::MissingTrackedLockfile {
            candidates: vec!["a/package-lock.json".to_string(), "a/yarn.lock".to_string()],
        };
        assert_eq!(
            violation_message(&kind),
            "Tracked manifest is missing a tracked lockfile: a/package-lock.json, a/yarn.lock"
        );
    }

    #[test]
    fn git_status_variants_each_have_a_message() {
        assert_eq!(
            violation_message(&ViolationKind::GitMetadataUnavailable(
                GitIndexStatus::MissingMetadata
            )),
            "Git metadata not found; skipping tracked lockfile validation"
        );
        assert_eq!(
            violation_message(&ViolationKind::GitMetadataUnavailable(
                GitIndexStatus::MissingIndex
            )),
            "Git index not found; skipping tracked lockfile validation"
        );
        assert_eq!(
            violation_message(&ViolationKind::GitMetadataUnavailable(
                GitIndexStatus::UnsupportedIndex
            )),
            "Git index could not be parsed; skipping tracked lockfile validation"
        );
    }

    #[test]
    fn labels_match_severity() {
        assert_eq!(severity_label(Severity::Error), "Error");
        assert_eq!(severity_label(Severity::Warning), "Warning");
    }
}

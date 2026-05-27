pub mod cli;
pub mod context;
mod diagnostic;
mod file_types;
mod ignore_directives;
mod parsers;
mod report;
mod rules;
mod scanner;

pub use diagnostic::{FileLintResult, LintResult, Severity, Violation};
pub use scanner::lint_files;

#[cfg(test)]
mod tests {
    use crate::context::bun::{bun_frozen_lockfile_enabled, bunfig_has_frozen_lockfile};
    use crate::context::git::{
        SubmodulePruner, gitdir_target_is_submodule, parse_gitdir_target_from_content,
        parse_gitmodules_paths_from_content, parse_tracked_paths_from_index,
    };
    use crate::context::lockfiles::expected_lockfile_paths;
    use crate::file_types::{
        comment_style_for_file, is_excluded, is_package_json, should_check_file,
    };
    use crate::ignore_directives::{
        CommentStyle, IgnoreDirective, is_ignore_directive, split_inline_ignore,
    };
    use crate::parsers::line::{
        check_file, is_comment_or_placeholder, should_lint_markdown_code_block,
    };
    use crate::rules::{check_bun, check_npm, check_pnpm, check_yarn};
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn context(
        is_markdown: bool,
        is_package_json: bool,
        bun_frozen_lockfile: bool,
    ) -> crate::scanner::LintContext {
        crate::scanner::LintContext {
            bun_frozen_lockfile,
            is_markdown,
            is_package_json,
        }
    }

    fn shell_style() -> CommentStyle {
        CommentStyle {
            prefix: "#",
            suffix: "",
        }
    }

    #[test]
    fn npm_ci_allowed() {
        assert!(check_npm("npm ci", 1).is_empty());
    }

    #[test]
    fn npm_install_bare_violation() {
        let violations = check_npm("npm install", 1);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id.as_deref(), Some("npm-install-bare"));
    }

    #[test]
    fn npm_install_global_yarn_is_still_npm() {
        let violations = check_npm("npm install -g yarn", 1);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id.as_deref(), Some("npm-version-pin"));
        assert!(violations[0].message.contains("npm"));
    }

    #[test]
    fn npm_install_with_version_allowed() {
        assert!(check_npm("npm i eslint@8.50.0", 1).is_empty());
        assert!(check_npm("npm i @types/node@18.0.0", 1).is_empty());
    }

    #[test]
    fn package_managers_require_lockfile_or_pin() {
        assert_eq!(check_pnpm("pnpm install", 1).len(), 1);
        assert!(check_pnpm("pnpm install --frozen-lockfile", 1).is_empty());
        assert_eq!(check_yarn("yarn install", 1).len(), 1);
        assert!(check_yarn("yarn install --immutable", 1).is_empty());
        assert_eq!(check_bun("bun install", 1, false).len(), 1);
        assert!(check_bun("bun install", 1, true).is_empty());
    }

    #[test]
    fn add_commands_require_version_pins() {
        assert_eq!(check_pnpm("pnpm add @types/react", 1).len(), 1);
        assert!(check_pnpm("pnpm add @types/react@18.0.0", 1).is_empty());
        assert_eq!(check_yarn("yarn add react", 1).len(), 1);
        assert!(check_yarn("yarn add react@18.2.0", 1).is_empty());
        assert_eq!(check_bun("bun add react", 1, false).len(), 1);
        assert!(check_bun("bun add react@18.2.0", 1, false).is_empty());
    }

    #[test]
    fn file_type_detection_matches_supported_inputs() {
        for file_name in ["install.sh", "install.bash", "install.zsh", "install.fish"] {
            assert!(should_check_file(Path::new(file_name)));
        }
        for file_name in ["Makefile", "makefile", "GNUmakefile", "rules.mk"] {
            assert!(should_check_file(Path::new(file_name)));
        }
        assert!(should_check_file(Path::new(".github/workflows/ci.yml")));
        assert!(should_check_file(Path::new("package.json")));
        assert!(is_package_json(Path::new("/repo/package.json")));
        assert!(!is_excluded(Path::new(".github/workflows/ci.yml")));
    }

    #[test]
    fn comment_styles_are_extension_aware() {
        let sh_style = comment_style_for_file(Path::new("script.sh")).unwrap();
        assert_eq!(sh_style.prefix, "#");
        assert_eq!(sh_style.suffix, "");

        let md_style = comment_style_for_file(Path::new("README.md")).unwrap();
        assert_eq!(md_style.prefix, "<!--");
        assert_eq!(md_style.suffix, "-->");
    }

    #[test]
    fn ignore_directives_support_inline_and_previous_line() {
        let style = shell_style();
        assert!(matches!(
            is_ignore_directive("# locked-in: ignore", &style),
            Some(IgnoreDirective::All)
        ));
        assert!(
            matches!(is_ignore_directive("# locked-in: ignore[yarn-frozen-lockfile]", &style), Some(IgnoreDirective::Specific(ref s)) if s == "yarn-frozen-lockfile")
        );
        assert!(split_inline_ignore("bun install  # locked-in: ignore", &style).is_some());
        assert!(split_inline_ignore("# locked-in: ignore", &style).is_none());
    }

    #[test]
    fn ignore_directives_filter_specific_rules() {
        let content = "# locked-in: ignore[bun-frozen-lockfile]\nbun install\nbun add react\n";
        let violations = check_file(content, &context(false, false, false), &shell_style());
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id.as_deref(), Some("bun-version-pin"));
    }

    #[test]
    fn markdown_lints_only_shell_code_blocks() {
        let content = r"
```bash
bun install
```

```
bun install
```
";
        let style = CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        };
        let violations = check_file(content, &context(true, false, false), &style);
        assert_eq!(violations.len(), 1);
        assert!(should_lint_markdown_code_block("```bash"));
        assert!(!should_lint_markdown_code_block("```text"));
    }

    #[test]
    fn comments_and_placeholders_are_skipped() {
        assert!(is_comment_or_placeholder("# npm install"));
        assert!(is_comment_or_placeholder("npm install <package>"));
        assert!(is_comment_or_placeholder("- `bun install`"));
    }

    #[test]
    fn package_json_scripts_are_linted() {
        let content = r#"{
  "name": "demo",
  "description": "Run npm install to set up",
  "scripts": {
    "setup": "npm install",
    "ci": "npm ci"
  }
}"#;
        let violations = check_file(content, &context(false, true, false), &shell_style());
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].line_num, 5);
    }

    #[test]
    fn bunfig_policy_is_parsed() {
        assert!(bunfig_has_frozen_lockfile(
            "[install]\nfrozenLockfile = true"
        ));
        assert!(!bunfig_has_frozen_lockfile("[test]\nfrozenLockfile = true"));
        assert!(!bunfig_has_frozen_lockfile(
            "[install]\nfrozenLockfile = false"
        ));
    }

    #[test]
    fn bunfig_lookup_stays_within_root() {
        let root = Path::new("/tmp/project");
        let file = Path::new("/tmp/project/docs/README.md");
        let cache = Mutex::new(HashMap::from([(PathBuf::from("/tmp/bunfig.toml"), true)]));

        assert!(!bun_frozen_lockfile_enabled(root, file, &cache));
    }

    #[test]
    fn git_submodule_helpers_parse_common_formats() {
        let content = r#"
[submodule "vendor/foo"]
    path = vendor/foo
[submodule "deps/bar"]
    path = "deps/bar"
"#;
        assert_eq!(
            parse_gitmodules_paths_from_content(content),
            vec![PathBuf::from("vendor/foo"), PathBuf::from("deps/bar")]
        );
        assert_eq!(
            parse_gitdir_target_from_content("gitdir: ../.git/modules/deps/foo\n"),
            Some("../.git/modules/deps/foo".to_string())
        );
        assert!(gitdir_target_is_submodule("../.git/modules/deps/foo"));
        assert!(!gitdir_target_is_submodule("/repo/.git/worktrees/feature"));

        let pruner = SubmodulePruner {
            root: PathBuf::from("/repo"),
            gitmodules_paths: vec![PathBuf::from("deps/foo")],
        };
        assert!(pruner.is_declared_submodule_path(Path::new("/repo/deps/foo")));
        assert!(!pruner.is_declared_submodule_path(Path::new("/repo/deps/foo/src")));
    }

    #[test]
    fn git_index_parser_extracts_tracked_paths() {
        let index = git_index_with_paths(&["package.json", "crates/app/Cargo.toml"]);

        assert_eq!(
            parse_tracked_paths_from_index(&index).unwrap(),
            vec![
                PathBuf::from("package.json"),
                PathBuf::from("crates/app/Cargo.toml")
            ]
        );
    }

    #[test]
    fn lockfile_expectations_match_manifest_siblings() {
        assert_eq!(
            expected_lockfile_paths(Path::new("apps/web/package.json")).unwrap(),
            vec![
                PathBuf::from("apps/web/package-lock.json"),
                PathBuf::from("apps/web/npm-shrinkwrap.json"),
                PathBuf::from("apps/web/pnpm-lock.yaml"),
                PathBuf::from("apps/web/yarn.lock"),
                PathBuf::from("apps/web/bun.lockb"),
                PathBuf::from("apps/web/bun.lock"),
            ]
        );
    }

    #[test]
    fn lockfile_validation_uses_tracked_files_not_filesystem_presence() {
        let root = temp_repo("untracked-lockfile");
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join("package.json"), r#"{"scripts": {}}"#).unwrap();
        fs::write(root.join("package-lock.json"), "{}").unwrap();
        fs::write(
            root.join(".git/index"),
            git_index_with_paths(&["package.json"]),
        )
        .unwrap();

        let result = crate::lint_files(&root);

        fs::remove_dir_all(root).unwrap();
        assert_eq!(result.violations_found, 1);
        assert_eq!(result.warnings_found, 0);
    }

    #[test]
    fn missing_git_metadata_is_warning_only() {
        let root = temp_repo("missing-git");
        fs::write(root.join("package.json"), r#"{"scripts": {}}"#).unwrap();

        let result = crate::lint_files(&root);

        fs::remove_dir_all(root).unwrap();
        assert_eq!(result.violations_found, 0);
        assert_eq!(result.warnings_found, 1);
    }

    fn temp_repo(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("locked-in-{name}-{nonce}"));
        fs::create_dir(&path).unwrap();
        path
    }

    fn git_index_with_paths(paths: &[&str]) -> Vec<u8> {
        let mut index = Vec::new();
        index.extend_from_slice(b"DIRC");
        index.extend_from_slice(&2u32.to_be_bytes());
        index.extend_from_slice(&u32::try_from(paths.len()).unwrap().to_be_bytes());

        for path in paths {
            let entry_start = index.len();
            index.extend_from_slice(&[0; 40]);
            index.extend_from_slice(&[0; 20]);
            index.extend_from_slice(&u16::try_from(path.len()).unwrap().to_be_bytes());
            index.extend_from_slice(path.as_bytes());
            index.push(0);
            while index.len().checked_sub(entry_start).unwrap() % 8 != 0 {
                index.push(0);
            }
        }

        index
    }
}

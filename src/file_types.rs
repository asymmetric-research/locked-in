use crate::ignore_directives::CommentStyle;
use std::ffi::OsStr;
use std::path::Path;

pub fn is_excluded(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str();
        name == OsStr::new("node_modules")
            || name == OsStr::new(".git")
            || name == OsStr::new("target")
            || name == OsStr::new("dist")
            || name == OsStr::new("build")
            || name == OsStr::new("coverage")
            || name == OsStr::new("vendor")
            || name == OsStr::new(".next")
            || name == OsStr::new(".nuxt")
            || name == OsStr::new(".turbo")
            || name == OsStr::new(".cache")
    }) || path.file_name() == Some(OsStr::new("lint-package-install.sh"))
}

pub fn should_check_file(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let path_str = path.to_string_lossy();

    if is_makefile(file_name) {
        return true;
    }

    if file_name.starts_with("Dockerfile") || file_name.ends_with(".dockerfile") {
        return true;
    }

    if has_extension(path, "md") {
        return true;
    }

    if is_shell_file(path) {
        return true;
    }

    if is_package_json(path) {
        return true;
    }

    let is_yaml = has_extension(path, "yml") || has_extension(path, "yaml");
    is_yaml && path_str.contains(".github/workflows")
}

pub fn is_package_json(path: &Path) -> bool {
    path.file_name() == Some(OsStr::new("package.json"))
}

pub fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

pub fn is_shell_file(path: &Path) -> bool {
    ["sh", "bash", "zsh", "fish", "ksh", "csh"]
        .iter()
        .any(|extension| has_extension(path, extension))
}

pub fn is_makefile(file_name: &str) -> bool {
    file_name == "Makefile"
        || file_name == "makefile"
        || file_name == "GNUmakefile"
        || has_extension(Path::new(file_name), "mk")
}

pub fn comment_style_for_file(path: &Path) -> Option<CommentStyle> {
    if has_extension(path, "md") {
        return Some(CommentStyle {
            prefix: "<!--",
            suffix: "-->",
        });
    }

    if is_shell_file(path) || is_makefile(path.file_name().and_then(|n| n.to_str()).unwrap_or("")) {
        return Some(CommentStyle {
            prefix: "#",
            suffix: "",
        });
    }

    if has_extension(path, "yml") || has_extension(path, "yaml") {
        return Some(CommentStyle {
            prefix: "#",
            suffix: "",
        });
    }

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if file_name.starts_with("Dockerfile") || file_name.ends_with(".dockerfile") {
        return Some(CommentStyle {
            prefix: "#",
            suffix: "",
        });
    }

    None
}

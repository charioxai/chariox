use std::path::{Component, Path};

use unicode_normalization::UnicodeNormalization;

pub(crate) fn is_portable_relative_path(path: &str) -> bool {
    if path.is_empty() || path.contains('\0') || path.contains('\\') {
        return false;
    }
    let candidate = Path::new(path);
    if candidate.is_absolute()
        || candidate
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }
    path.split('/').all(is_portable_component)
}

pub(crate) fn portable_path_alias_key(path: &str) -> Option<String> {
    if !is_portable_relative_path(path) {
        return None;
    }
    Some(
        path.split('/')
            .map(normalized_component)
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn is_portable_component(component: &str) -> bool {
    if component.is_empty()
        || component.contains(':')
        || component.ends_with([' ', '.'])
        || component.chars().any(char::is_control)
    {
        return false;
    }
    let normalized = normalized_component(component);
    !normalized.is_empty()
        && !normalized.contains(['/', '\\', ':'])
        && !normalized.ends_with([' ', '.'])
        && !normalized.chars().any(char::is_control)
        && !is_portable_git_admin_component(&normalized)
        && !is_windows_reserved_component(&normalized)
}

fn normalized_component(component: &str) -> String {
    component
        .nfkc()
        .filter(|character| {
            !matches!(
                *character,
                '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{feff}'
            )
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_portable_git_admin_component(component: &str) -> bool {
    if component == ".git" {
        return true;
    }
    let short = component.strip_prefix('.').unwrap_or(component);
    short.strip_prefix("git~").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn is_windows_reserved_component(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(stem, "con" | "prn" | "aux" | "nul")
        || stem
            .strip_prefix("com")
            .or_else(|| stem.strip_prefix("lpt"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_paths_reject_cross_platform_aliases() {
        assert!(is_portable_relative_path("bin/tool.sh"));
        for path in [
            "a\\b",
            "name.",
            "name ",
            "name:stream",
            "CON.txt",
            "com1",
            ".GIT/config",
            "git~1/config",
            "../outside",
        ] {
            assert!(!is_portable_relative_path(path), "{path}");
        }
        assert_eq!(
            portable_path_alias_key("Docs/Readme"),
            portable_path_alias_key("docs/readme")
        );
        assert_eq!(
            portable_path_alias_key("cafe\u{301}.txt"),
            portable_path_alias_key("caf\u{e9}.txt")
        );
    }
}

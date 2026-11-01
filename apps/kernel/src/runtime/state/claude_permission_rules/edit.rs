//! Edit rules cover the authorized working directory, regardless of filenames.
//! Claude folds Write, Edit and MultiEdit into Edit and writes an absolute path as //path.
use std::path::{Component, Path};

fn is_plain(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-' | '/' | '+' | '@' | ',')
}

fn names(path: &Path) -> impl Iterator<Item = &str> {
    path.components().filter_map(|component| match component {
        Component::Normal(name) => name.to_str(),
        _ => None,
    })
}

fn is_project_directory(directory: &Path) -> bool {
    let plain = directory
        .to_str()
        .is_some_and(|text| text.chars().all(is_plain));
    let normal = directory
        .components()
        .all(|component| matches!(component, Component::RootDir | Component::Normal(_)));
    let parts: Vec<&str> = names(directory).collect();
    let home = matches!(parts.as_slice(), ["home" | "Users", _] | ["root"])
        || std::env::var_os("HOME").is_some_and(|home| Path::new(&home) == directory);
    directory.is_absolute() && plain && normal && parts.len() >= 2 && !home
}

pub(super) fn edit_rule(file_path: &str, directory: &Path) -> Option<String> {
    let file = Path::new(file_path);
    let inside = file.is_absolute()
        && file
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && file.starts_with(directory);
    if !is_project_directory(directory) || !inside {
        return None;
    }
    Some(format!("/{}/**", directory.to_str()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_file_names_get_the_authorized_directory_rule() {
        for directory in [
            "/root/work/app",
            "/root/.ssh/project",
            "/root/.aws/project",
            "/root/.config/gh/project",
            "/root/.chariox/dev/project",
            "/work/.env/project",
        ] {
            for file in [
                "main.rs",
                "credentials.rs",
                ".env",
                ".env.customer",
                "server.pem",
                "tls.key",
                ".ssh/id_ed25519",
                ".aws/credentials",
                ".config/gh/hosts.yml",
            ] {
                assert_eq!(
                    edit_rule(&format!("{directory}/{file}"), Path::new(directory)),
                    Some(format!("/{directory}/**")),
                    "{directory}/{file}"
                );
            }
        }
    }

    #[test]
    fn no_rule_is_stated_outside_a_project_directory() {
        for directory in [
            "/",
            "/root",
            "/home",
            "/home/miguel",
            "/Users/miguel",
            "/tmp",
            "work",
            "",
        ] {
            assert!(
                edit_rule(&format!("{directory}/a.rs"), Path::new(directory)).is_none(),
                "{directory:?}"
            );
        }
    }

    #[test]
    fn no_rule_is_stated_for_a_file_outside_the_directory() {
        for file in [
            "/root/work/other/a.rs",
            "/root/work/app/../other/a.rs",
            "src/a.rs",
            "/root/work/application/a.rs",
        ] {
            assert!(
                edit_rule(file, Path::new("/root/work/app")).is_none(),
                "{file:?}"
            );
        }
    }

    #[test]
    fn directories_the_rule_parser_reads_specially_get_no_rule() {
        for directory in [
            "/root/work/a b",
            "/root/work/a*",
            "/root/work/(x)",
            "/root/work/a[1]",
            "/root/work/a?",
            "/root/work/a\\b",
            "/root/work/app/..",
        ] {
            assert!(
                edit_rule(&format!("{directory}/a.rs"), Path::new(directory)).is_none(),
                "{directory:?}"
            );
        }
    }
}

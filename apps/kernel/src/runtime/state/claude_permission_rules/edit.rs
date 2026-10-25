//! The session rule for file edits: the session's own working directory, as a path-prefix rule.
//!
//! Claude folds Write, Edit and MultiEdit into the `Edit` rule family (a `Write(path)` rule never
//! matches) and writes an absolute path as `//path`.

use std::path::{Component, Path};

/// Names that hold credentials. A working directory that contains one gets no session rule, and
/// inside the granted directory these names keep asking. A name that only looks like one
/// (`credentials.rs`, `.env.example`) is an ordinary file.
enum Sensitive {
    Name(&'static str),
    Prefix(&'static str),
    Suffix(&'static str),
    /// `.env` and the `.env.*` files that hold values, not the templates that are committed.
    EnvFile,
}

const SENSITIVE: &[Sensitive] = &[
    Sensitive::Name(".ssh"),
    Sensitive::Name(".aws"),
    Sensitive::Name(".gnupg"),
    Sensitive::Name(".kube"),
    Sensitive::Name(".azure"),
    Sensitive::Name(".gcloud"),
    Sensitive::Name(".chariox"),
    Sensitive::Name(".claude"),
    Sensitive::Name(".codex"),
    Sensitive::Name(".netrc"),
    Sensitive::Name(".npmrc"),
    Sensitive::Name(".pypirc"),
    Sensitive::Name(".pgpass"),
    Sensitive::Name(".git-credentials"),
    Sensitive::Name("keychains"),
    Sensitive::EnvFile,
    Sensitive::Prefix("id_rsa"),
    Sensitive::Prefix("id_ed25519"),
    Sensitive::Prefix("id_ecdsa"),
    Sensitive::Prefix("id_dsa"),
    Sensitive::Suffix(".pem"),
    Sensitive::Suffix(".key"),
    Sensitive::Suffix(".p12"),
    Sensitive::Suffix(".pfx"),
    Sensitive::Suffix(".keystore"),
    Sensitive::Suffix(".kdbx"),
];

/// The parts of a `.env.*` name that make it a template.
const ENV_TEMPLATE_PARTS: &[&str] = &["example", "sample", "template", "dist", "defaults"];

fn is_env_file(name: &str) -> bool {
    name == ".env"
        || name.strip_prefix(".env.").is_some_and(|rest| {
            !rest
                .split('.')
                .any(|part| ENV_TEMPLATE_PARTS.contains(&part))
        })
}

impl Sensitive {
    fn matches(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        match self {
            Self::Name(sensitive) => name == *sensitive,
            Self::Prefix(prefix) => name.starts_with(prefix),
            Self::Suffix(suffix) => name.ends_with(suffix),
            Self::EnvFile => is_env_file(&name),
        }
    }

    /// How the name reads in the prompt text.
    fn shown(&self) -> String {
        match self {
            Self::Name(name) => (*name).to_string(),
            Self::Prefix(prefix) => format!("{prefix}*"),
            Self::Suffix(suffix) => format!("*{suffix}"),
            Self::EnvFile => ".env, .env.* (not templates)".to_string(),
        }
    }

    /// The gitignore patterns that cover this name anywhere under a directory.
    fn patterns(&self) -> Vec<String> {
        match self {
            Self::Name(name) => vec![format!("**/{name}"), format!("**/{name}/**")],
            Self::Prefix(prefix) => vec![format!("**/{prefix}*")],
            Self::Suffix(suffix) => vec![format!("**/*{suffix}")],
            // Templates also reach the hook, where the directory grant answers them without
            // involving the person. Keep arbitrary environment names covered here.
            Self::EnvFile => vec!["**/.env".to_string(), "**/.env.*".to_string()],
        }
    }
}

/// `~/.config/gh` is the one credential directory that is not named by a single component.
const CONFIG_GH: &str = "**/.config/gh/**";

/// Characters Claude's rule parser reads literally in a path.
fn is_plain(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-' | '/' | '+' | '@' | ',')
}

fn names(path: &Path) -> impl Iterator<Item = &str> {
    path.components().filter_map(|component| match component {
        Component::Normal(name) => name.to_str(),
        _ => None,
    })
}

fn holds_credentials(path: &Path) -> bool {
    let all: Vec<&str> = names(path).collect();
    let in_config_gh = all.windows(2).any(|pair| pair == [".config", "gh"]);
    in_config_gh
        || all
            .iter()
            .any(|name| SENSITIVE.iter().any(|sensitive| sensitive.matches(name)))
}

/// A directory only the work of one project lives in: absolute, plain, at least a few levels deep
/// and not a home directory.
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

/// What an edit session grant adds to the run: the allow rule, and for the working directory an
/// ask rule for each credential name inside it, so those still prompt.
pub(super) struct EditGrant {
    pub allow: String,
    pub ask: Vec<String>,
    /// The credential names behind the ask rules, for the prompt text.
    pub ask_names: Vec<String>,
}

/// The grant for an edit of `file_path` in a session whose working directory is `directory`: the
/// whole directory for an ordinary file, and only the file itself for a credential path.
pub(super) fn edit_grant(file_path: &str, directory: &Path) -> Option<EditGrant> {
    let file = Path::new(file_path);
    let inside = file.is_absolute()
        && file
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && file.starts_with(directory);
    if !is_project_directory(directory) || !inside || holds_credentials(directory) {
        return None;
    }
    let root = format!("/{}", directory.to_str()?);
    if holds_credentials(file) {
        let text = file.to_str().filter(|text| text.chars().all(is_plain))?;
        return Some(EditGrant {
            allow: format!("/{text}"),
            ask: Vec::new(),
            ask_names: Vec::new(),
        });
    }
    let ask = SENSITIVE
        .iter()
        .flat_map(Sensitive::patterns)
        .chain([CONFIG_GH.to_string()])
        .map(|pattern| format!("{root}/{pattern}"))
        .collect();
    let ask_names = SENSITIVE
        .iter()
        .map(Sensitive::shown)
        .chain([".config/gh".to_string()])
        .collect();
    Some(EditGrant {
        allow: format!("{root}/**"),
        ask,
        ask_names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(file: &str, directory: &str) -> Option<EditGrant> {
        edit_grant(file, Path::new(directory))
    }

    #[test]
    fn a_file_in_the_working_directory_grants_the_directory_prefix() {
        let granted = grant("/root/work/app/src/a.rs", "/root/work/app").expect("grant");
        assert_eq!(granted.allow, "//root/work/app/**");
        assert!(granted.ask.contains(&"//root/work/app/**/.env".to_string()));
        assert!(granted
            .ask
            .contains(&"//root/work/app/**/.ssh/**".to_string()));
        assert!(granted
            .ask
            .contains(&"//root/work/app/**/.config/gh/**".to_string()));
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
                grant(&format!("{directory}/a.rs"), directory).is_none(),
                "{directory:?}"
            );
        }
    }

    #[test]
    fn no_rule_is_stated_for_a_directory_that_holds_credentials() {
        for directory in [
            "/root/.ssh/x",
            "/root/.aws/x",
            "/root/.config/gh/x",
            "/root/.chariox/dev/x",
            "/work/.env/x",
        ] {
            assert!(
                grant(&format!("{directory}/a.rs"), directory).is_none(),
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
        ] {
            assert!(grant(file, "/root/work/app").is_none(), "{file:?}");
        }
        assert!(grant("/root/work/application/a.rs", "/root/work/app").is_none());
    }

    #[test]
    fn a_credential_file_is_granted_by_itself_and_the_directory_keeps_asking_for_it() {
        for file in [
            "/root/work/app/.env",
            "/root/work/app/.env.local",
            "/root/work/app/.env.production",
            "/root/work/app/.env.customer",
            "/root/work/app/web/.env.staging",
            "/root/work/app/.ssh/id_ed25519",
            "/root/work/app/.aws/credentials",
            "/root/work/app/deploy/server.pem",
            "/root/work/app/deploy/tls.key",
            "/root/work/app/.config/gh/hosts.yml",
        ] {
            let granted = grant(file, "/root/work/app").unwrap_or_else(|| panic!("{file:?}"));
            assert_eq!(granted.allow, format!("/{file}"), "{file:?}");
            assert!(granted.ask.is_empty(), "{file:?}");
        }
        let directory = grant("/root/work/app/a.rs", "/root/work/app").expect("grant");
        for pattern in [
            ".env",
            ".env.*",
            ".ssh/**",
            ".aws/**",
            "*.pem",
            "*.key",
            "id_ed25519*",
            ".config/gh/**",
        ] {
            assert!(
                directory
                    .ask
                    .contains(&format!("//root/work/app/**/{pattern}")),
                "{pattern}"
            );
        }
    }

    #[test]
    fn files_that_only_look_like_credentials_are_ordinary_files() {
        for file in [
            "/root/work/app/src/credentials.rs",
            "/root/work/app/credentials.json",
            "/root/work/app/src/secrets.rs",
            "/root/work/app/src/secret_santa.ts",
            "/root/work/app/vault/mod.rs",
            "/root/work/app/.env.example",
            "/root/work/app/.env.sample",
            "/root/work/app/.env.template",
            "/root/work/app/.env.production.example",
            "/root/work/app/env.rs",
        ] {
            let granted = grant(file, "/root/work/app").unwrap_or_else(|| panic!("{file:?}"));
            assert_eq!(granted.allow, "//root/work/app/**", "{file:?}");
            assert!(!granted.ask.is_empty(), "{file:?}");
            assert!(!holds_credentials(Path::new(file)), "{file:?}");
            assert!(
                granted
                    .ask
                    .iter()
                    .all(|rule| !rule.contains("*credential*") && !rule.contains("/secret*")),
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
            "/root/work/[x]",
        ] {
            assert!(
                grant(&format!("{directory}/a.rs"), directory).is_none(),
                "{directory:?}"
            );
        }
    }
}

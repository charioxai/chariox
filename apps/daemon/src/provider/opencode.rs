use std::env;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;

const OPENCODE_ENV_OVERRIDE: &str = "ARROBA_OPENCODE_BIN";

pub fn resolve_opencode_executable() -> Result<PathBuf, DaemonError> {
    if let Some(path) = env::var_os(OPENCODE_ENV_OVERRIDE).map(PathBuf::from) {
        return resolve_candidate(path, true).ok_or_else(|| {
            DaemonError::ProviderExecutableNotFound {
                adapter_key: "opencode".to_string(),
                executable: env::var(OPENCODE_ENV_OVERRIDE)
                    .unwrap_or_else(|_| "opencode".to_string()),
            }
        });
    }

    resolve_candidate(PathBuf::from("opencode"), false).ok_or_else(|| {
        DaemonError::ProviderExecutableNotFound {
            adapter_key: "opencode".to_string(),
            executable: "opencode".to_string(),
        }
    })
}

fn resolve_candidate(candidate: PathBuf, treat_as_literal_path: bool) -> Option<PathBuf> {
    if treat_as_literal_path || candidate.components().count() > 1 {
        return candidate.exists().then_some(candidate);
    }

    if candidate.is_absolute() && candidate.exists() {
        return Some(candidate);
    }

    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|directory| directory.join(&candidate))
        .find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::resolve_opencode_executable;

    #[test]
    fn resolves_override_path_for_tests() {
        let path = std::env::temp_dir().join(format!(
            "arroba-opencode-resolve-test-{}",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fixture should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &path);

        let resolved = resolve_opencode_executable().expect("override path should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        let _ = fs::remove_file(&path);
        assert_eq!(resolved, path);
    }
}

//! The directory the kernel was launched from, when it can be a user workspace.
//!
//! A kernel started by a service manager has no meaningful working directory:
//! it is often its own release bundle or state directory. Offering that as the
//! default workspace is wrong; the last-used workspace or the user's home is not.

use std::path::{Path, PathBuf};

pub(super) fn launched_workspace_directory() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let executable_directory = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf));
    let chariox_home = crate::credential::chariox_home();
    (!is_kernel_owned(
        &cwd,
        chariox_home.as_deref(),
        executable_directory.as_deref(),
    ))
    .then_some(cwd)
}

pub(super) fn user_home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute() && home.is_dir())
}

/// The filesystem root, the Chariox state tree and the directory holding the
/// running binary belong to the kernel, never to a user's project.
fn is_kernel_owned(
    cwd: &Path,
    chariox_home: Option<&Path>,
    executable_directory: Option<&Path>,
) -> bool {
    let resolved = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let cwd = resolved(cwd);
    cwd.parent().is_none()
        || chariox_home.is_some_and(|home| cwd.starts_with(resolved(home)))
        || executable_directory.is_some_and(|directory| cwd == resolved(directory))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_bundle_state_and_root_directories_are_not_workspaces() {
        let home = Path::new("/home/user/.chariox/dev/md-staging");
        let bundle = home.join("releases/186c31ef/chariox-main-native-linux-x86_64");
        assert!(is_kernel_owned(&bundle, Some(home), None));
        assert!(is_kernel_owned(home, Some(home), None));
        assert!(is_kernel_owned(Path::new("/"), None, None));
        assert!(is_kernel_owned(
            Path::new("/opt/chariox"),
            None,
            Some(Path::new("/opt/chariox"))
        ));
    }

    #[test]
    fn a_project_directory_is_a_workspace_even_when_it_holds_the_dev_binary() {
        let home = Path::new("/home/user/.chariox/dev/task");
        let repository = Path::new("/home/user/src/project");
        let binary_directory = repository.join("target/debug");
        assert!(!is_kernel_owned(
            repository,
            Some(home),
            Some(&binary_directory)
        ));
    }
}

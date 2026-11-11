//! A whole machine as the place a copy reads from and writes to: absolute paths, links followed as
//! the OS does, and everything below the kernel's own state refused.
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::StateGuard;
use crate::agent_file_copy::{Access, Place};

/// The prefix of the hidden file a copy stages its bytes in beside the destination.
const STAGING_PREFIX: &str = ".chariox-copy-";

#[derive(Debug, Clone)]
pub(crate) struct Machine {
    guard: Arc<StateGuard>,
}

impl Machine {
    pub(crate) fn new(guard: StateGuard) -> Self {
        Self {
            guard: Arc::new(guard),
        }
    }

    pub(crate) fn guard(&self) -> &StateGuard {
        &self.guard
    }

    /// The absolute path `path` names: `~/` is the kernel user's home, `.` and `..` are refused so a
    /// path means what it says.
    pub(crate) fn absolute(path: &str) -> Result<PathBuf, String> {
        let expanded = match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
            (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
            _ => PathBuf::from(path),
        };
        if !expanded.is_absolute() {
            return Err(format!(
                "`{path}` is not an absolute path; start it with / or ~/"
            ));
        }
        if expanded
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        {
            return Err(format!("`{path}` must not contain `.` or `..`"));
        }
        Ok(expanded)
    }
}

impl Place for Machine {
    fn resolve(&self, path: &str, access: Access) -> Result<PathBuf, String> {
        let absolute = Self::absolute(path)?;
        let name = absolute
            .file_name()
            .ok_or_else(|| format!("`{path}` names no file"))?;
        if name.to_string_lossy().starts_with(STAGING_PREFIX) {
            return Err(format!(
                "`{path}` is one of Chariox's own transfer files; choose another name"
            ));
        }
        self.guard.check(&absolute)?;
        match access {
            Access::Read => {
                let real = absolute
                    .canonicalize()
                    .map_err(|_| format!("`{path}` does not exist"))?;
                self.guard.check(&real)?;
                if real.is_dir() {
                    return Err(format!("`{path}` is a folder; copy it as a folder"));
                }
                Ok(real)
            }
            Access::Write => {
                let folder = absolute.parent().unwrap_or(Path::new("/"));
                std::fs::create_dir_all(folder)
                    .map_err(|error| format!("`{path}` could not be reached: {error}"))?;
                let real_folder = folder
                    .canonicalize()
                    .map_err(|_| format!("`{path}` could not be reached"))?;
                let target = real_folder.join(name);
                self.guard.check(&target)?;
                if target.is_dir() {
                    return Err(format!("`{path}` is a folder"));
                }
                Ok(target)
            }
        }
    }

    fn accepts_files(&self) -> Result<(), String> {
        Ok(())
    }

    fn max_file_bytes(&self) -> u64 {
        u64::MAX
    }

    fn root(&self) -> &Path {
        Path::new("/")
    }
}

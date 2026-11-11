//! What Files on a machine never touches: the kernel's own state. Its home, config, state and logs
//! hold keys, tokens and history, and the Vault holds the secrets; no client and no agent lists,
//! reads or writes there through Files, however the path is spelled (links and `..` included).
use std::path::{Component, Path, PathBuf};

/// The folders and the Vault file Files stays out of.
#[derive(Debug, Clone, Default)]
pub(crate) struct StateGuard {
    protected: Vec<PathBuf>,
}

impl StateGuard {
    /// This kernel's state folders plus the Vault at `vault` (a path as configured, `~/` allowed).
    pub(crate) fn new(vault: Option<&str>) -> Self {
        let mut protected = crate::config::kernel_state_roots();
        protected.extend(vault.map(expand_home));
        protected.sort();
        protected.dedup();
        Self {
            protected: protected.iter().map(|path| settle(path)).collect(),
        }
    }

    /// Why `path` may not be used, or `Ok` when it lies outside the kernel's state.
    pub(crate) fn check(&self, path: &Path) -> Result<(), String> {
        let settled = settle(path);
        if self.protected.iter().any(|root| settled.starts_with(root)) {
            return Err(format!(
                "`{}` is part of this kernel's own state or Vault, which Files never lists, reads or writes",
                path.display()
            ));
        }
        Ok(())
    }

    /// Why `path` may not be replaced: besides what `check` refuses, a folder that holds the
    /// kernel's state or Vault would take them along when it is set aside and removed.
    pub(crate) fn check_replace(&self, path: &Path) -> Result<(), String> {
        self.check(path)?;
        let settled = settle(path);
        if self.protected.iter().any(|root| root.starts_with(&settled)) {
            return Err(format!(
                "`{}` holds this kernel's own state or Vault, which Files never replaces or deletes",
                path.display()
            ));
        }
        Ok(())
    }
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// `path` with its links resolved as far as it exists and `.`/`..` folded in below that, so a
/// path that does not exist yet (a file about to be written) is placed where it would land.
fn settle(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut resolved = PathBuf::new();
    let mut components = absolute.components().peekable();
    while let Some(component) = components.next() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => {
                resolved.push(other);
                // A link anywhere on the way is followed at once, so `..` after it folds into
                // the real parent as the OS would.
                if let Ok(real) = resolved.canonicalize() {
                    resolved = real;
                } else {
                    // Below the last real folder nothing can be a link: fold the rest by name.
                    for rest in components.by_ref() {
                        match rest {
                            Component::ParentDir => {
                                resolved.pop();
                            }
                            Component::CurDir => {}
                            rest => resolved.push(rest),
                        }
                    }
                    break;
                }
            }
        }
    }
    resolved
}

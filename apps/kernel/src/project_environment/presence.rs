//! What this kernel holds for the saved Environment: computed on every read, never stored.
use super::{environment_error, project_environment_vault_service, relative_project_path};
use crate::{config::DaemonConfig, error::DaemonError};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretPresence {
    Present,
    Missing,
    /// The Vault is locked, so the kernel cannot tell.
    Locked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilePresence {
    Present { size_bytes: u64 },
    Missing,
}

pub(crate) fn secret_presence(
    config: &DaemonConfig,
    project_id: &str,
    names: &[String],
) -> Result<Vec<SecretPresence>, DaemonError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let vault = crate::secret::project_environment_vault(config)?;
    let service = project_environment_vault_service(project_id);
    let mut presence = Vec::with_capacity(names.len());
    for name in names {
        presence.push(match vault.find_secret(&service, name) {
            Ok(Some(_)) => SecretPresence::Present,
            Ok(None) => SecretPresence::Missing,
            Err(error) if crate::secret::is_chariox_vault_locked_error(&error) => {
                SecretPresence::Locked
            }
            Err(_) => return Err(environment_error("the Vault could not be read")),
        });
    }
    Ok(presence)
}

/// A file counts only when it is a regular file inside the checkout; links are not followed.
pub(crate) fn file_presence(checkout: &Path, relative: &str) -> FilePresence {
    if relative_project_path(relative).is_err() {
        return FilePresence::Missing;
    }
    match std::fs::symlink_metadata(checkout.join(relative)) {
        Ok(metadata) if metadata.is_file() => FilePresence::Present {
            size_bytes: metadata.len(),
        },
        _ => FilePresence::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linked_or_absent_file_is_missing_and_a_regular_one_reports_its_size() {
        let dir =
            std::env::temp_dir().join(format!("chariox-presence-{:032x}", rand::random::<u128>()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(dir.join("config/a.json"), b"12345").unwrap();
        std::os::unix::fs::symlink("config/a.json", dir.join("link")).unwrap();
        assert_eq!(
            file_presence(&dir, "config/a.json"),
            FilePresence::Present { size_bytes: 5 }
        );
        assert_eq!(file_presence(&dir, "link"), FilePresence::Missing);
        assert_eq!(file_presence(&dir, "nope"), FilePresence::Missing);
        assert_eq!(file_presence(&dir, "../escape"), FilePresence::Missing);
        let _ = std::fs::remove_dir_all(dir);
    }
}

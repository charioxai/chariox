use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{validate_credentials, UserCredentialConfig};
use crate::error::DaemonError;
use crate::mcp::validate_registry_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrobaCredentialRegistry {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRegistryEntry {
    pub credential: UserCredentialConfig,
    pub path: PathBuf,
}

impl ArrobaCredentialRegistry {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn user_root() -> Option<PathBuf> {
        arroba_home().map(|home| home.join("credentials"))
    }

    pub fn user() -> Result<Self, DaemonError> {
        let root = Self::user_root().ok_or_else(|| DaemonError::InvalidConfig {
            field: "credential registry root",
            message: "HOME must be set to resolve ~/.arroba/credentials",
        })?;
        Ok(Self::new(root))
    }

    pub fn install_from_file(
        &self,
        source: &Path,
    ) -> Result<(UserCredentialConfig, PathBuf), DaemonError> {
        if !source.is_file() {
            return Err(DaemonError::InvalidConfig {
                field: "credential file",
                message: "credential registration requires a YAML file",
            });
        }
        let credential = Self::read_yaml(source)?;
        validate_credentials(std::slice::from_ref(&credential))?;
        ensure_private_dir(&self.root, "credential.register")?;
        let path = self.path_for(&credential.id)?;
        let payload =
            serde_yaml::to_string(&credential).map_err(|error| DaemonError::LocalTransport {
                operation: "credential.register",
                message: format!(
                    "failed to serialize credential `{}`: {error}",
                    credential.id
                ),
            })?;
        atomic_write_private(&path, payload.as_bytes(), "credential.register")?;
        Ok((credential, path))
    }

    pub fn upsert(
        &self,
        credential: UserCredentialConfig,
    ) -> Result<(UserCredentialConfig, PathBuf), DaemonError> {
        validate_credentials(std::slice::from_ref(&credential))?;
        ensure_private_dir(&self.root, "credential.upsert")?;
        let path = self.path_for(&credential.id)?;
        let payload =
            serde_yaml::to_string(&credential).map_err(|error| DaemonError::LocalTransport {
                operation: "credential.upsert",
                message: format!(
                    "failed to serialize credential `{}`: {error}",
                    credential.id
                ),
            })?;
        atomic_write_private(&path, payload.as_bytes(), "credential.upsert")?;
        Ok((credential, path))
    }

    pub fn remove(&self, id: &str) -> Result<(UserCredentialConfig, PathBuf), DaemonError> {
        let path = self
            .find_path(id)?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "credential.remove",
                message: format!("credential `{id}` is not registered"),
            })?;
        let credential = Self::read_yaml(&path)?;
        fs::remove_file(&path).map_err(io_error("credential.remove"))?;
        Ok((credential, path))
    }

    pub fn get(&self, id: &str) -> Result<Option<UserCredentialConfig>, DaemonError> {
        let Some(path) = self.find_path(id)? else {
            return Ok(None);
        };
        Self::read_yaml(&path).map(Some)
    }

    pub fn list(&self) -> Result<Vec<UserCredentialConfig>, DaemonError> {
        let mut entries = BTreeMap::new();
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        for entry in fs::read_dir(&self.root).map_err(io_error("credential.list"))? {
            let path = entry.map_err(io_error("credential.list"))?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
                continue;
            }
            let credential = Self::read_yaml(&path)?;
            entries.entry(credential.id.clone()).or_insert(credential);
        }
        Ok(entries.into_values().collect())
    }

    pub fn path_for(&self, id: &str) -> Result<PathBuf, DaemonError> {
        validate_registry_name(id, "credential id")?;
        Ok(self.root.join(format!("{id}.yaml")))
    }

    fn find_path(&self, id: &str) -> Result<Option<PathBuf>, DaemonError> {
        let path = self.path_for(id)?;
        Ok(path.exists().then_some(path))
    }

    fn read_yaml(path: &Path) -> Result<UserCredentialConfig, DaemonError> {
        let contents = fs::read_to_string(path).map_err(io_error("credential.read"))?;
        let credential =
            serde_yaml::from_str::<UserCredentialConfig>(&contents).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "credential.read",
                    message: format!("failed to parse credential `{}`: {error}", path.display()),
                }
            })?;
        validate_credentials(std::slice::from_ref(&credential))?;
        Ok(credential)
    }
}

pub fn load_user_credentials() -> Result<Vec<UserCredentialConfig>, DaemonError> {
    ArrobaCredentialRegistry::user()?.list()
}

fn arroba_home() -> Option<PathBuf> {
    std::env::var_os("ARROBA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".arroba")))
}

fn ensure_private_dir(path: &Path, operation: &'static str) -> Result<(), DaemonError> {
    fs::create_dir_all(path).map_err(io_error(operation))?;
    set_private_dir_permissions(path, operation)
}

fn atomic_write_private(
    path: &Path,
    bytes: &[u8],
    operation: &'static str,
) -> Result<(), DaemonError> {
    let parent = path.parent().ok_or_else(|| DaemonError::LocalTransport {
        operation,
        message: "registry path has no parent".to_string(),
    })?;
    ensure_private_dir(parent, operation)?;
    let tmp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("credential"),
        std::process::id()
    ));
    {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_path)
            .map_err(io_error(operation))?;
        set_private_file_permissions(&tmp_path, operation)?;
        file.write_all(bytes).map_err(io_error(operation))?;
        file.sync_all().map_err(io_error(operation))?;
    }
    fs::rename(&tmp_path, path).map_err(io_error(operation))?;
    set_private_file_permissions(path, operation)
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path, operation: &'static str) -> Result<(), DaemonError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error(operation))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path, _operation: &'static str) -> Result<(), DaemonError> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path, operation: &'static str) -> Result<(), DaemonError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io_error(operation))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path, _operation: &'static str) -> Result<(), DaemonError> {
    Ok(())
}

fn io_error(operation: &'static str) -> impl Fn(std::io::Error) -> DaemonError {
    move |error| DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        UserCredentialInjectionConfig, UserCredentialSourceConfig, UserCredentialUse,
    };

    #[test]
    fn upsert_writes_and_replaces_credential_metadata() {
        let root = std::env::temp_dir().join(format!(
            "arroba-credential-upsert-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let registry = ArrobaCredentialRegistry::new(root.clone());

        let first = UserCredentialConfig {
            id: "demo-token".to_string(),
            description: Some("first".to_string()),
            source: UserCredentialSourceConfig::Vault {
                key: "demo-token".to_string(),
            },
            allowed_hosts: vec!["api.example.com".to_string()],
            allowed_uses: vec![UserCredentialUse::Http],
            injection: UserCredentialInjectionConfig::Header {
                name: "authorization".to_string(),
                value: "Bearer ${secret}".to_string(),
            },
        };
        registry.upsert(first).expect("first upsert should write");

        let second = UserCredentialConfig {
            id: "demo-token".to_string(),
            description: Some("second".to_string()),
            source: UserCredentialSourceConfig::Vault {
                key: "demo-token".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
        };
        registry
            .upsert(second.clone())
            .expect("second upsert should replace");

        assert_eq!(
            registry
                .get("demo-token")
                .expect("credential should read")
                .expect("credential should exist"),
            second
        );
        let _ = fs::remove_dir_all(&root);
    }
}

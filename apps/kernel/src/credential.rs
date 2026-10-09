use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{validate_credentials, UserCredentialConfig};
use crate::error::DaemonError;
use crate::mcp::validate_registry_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharioxCredentialRegistry {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRegistryEntry {
    pub credential: UserCredentialConfig,
    pub path: PathBuf,
}

impl CharioxCredentialRegistry {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn user_root() -> Option<PathBuf> {
        chariox_home().map(|home| home.join("credentials"))
    }

    pub fn user() -> Result<Self, DaemonError> {
        let root = Self::user_root().ok_or(DaemonError::InvalidConfig {
            field: "credential registry root",
            message: "HOME must be set to resolve ~/.chariox/credentials",
        })?;
        Ok(Self::new(root))
    }

    pub fn install_from_file(
        &self,
        source: &Path,
    ) -> Result<(UserCredentialConfig, PathBuf), DaemonError> {
        self.upsert(Self::read_registration_file(source)?)
    }

    pub(crate) fn read_registration_file(
        source: &Path,
    ) -> Result<UserCredentialConfig, DaemonError> {
        if !source.is_file() {
            return Err(DaemonError::InvalidConfig {
                field: "credential file",
                message: "credential registration requires a YAML file",
            });
        }
        let credential = Self::read_yaml(source)?;
        validate_credential_registration(&credential)?;
        Ok(credential)
    }

    pub fn upsert(
        &self,
        credential: UserCredentialConfig,
    ) -> Result<(UserCredentialConfig, PathBuf), DaemonError> {
        validate_credential_registration(&credential)?;
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

// MP-08/MP-10: validate new registrations without making legacy metadata
// unreadable. Never infer an injection mode from a username or rewrite a handle.
pub(crate) fn validate_credential_registration(
    credential: &UserCredentialConfig,
) -> Result<(), DaemonError> {
    validate_credentials(std::slice::from_ref(credential))?;
    if credential
        .allowed_uses
        .contains(&crate::config::UserCredentialUse::Browser)
        && !matches!(
            credential.injection,
            crate::config::UserCredentialInjectionConfig::Browser
        )
    {
        return Err(DaemonError::LocalTransport {
            operation: "credential.register",
            message: format!(
                "credential `{}` allows browser use but requires injection.kind=browser; basic is HTTP Basic authentication",
                credential.id,
            ),
        });
    }
    Ok(())
}

pub fn load_user_credentials() -> Result<Vec<UserCredentialConfig>, DaemonError> {
    CharioxCredentialRegistry::user()?.list()
}

fn chariox_home() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return Some(root.join("user"));
    }
    std::env::var_os("CHARIOX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".chariox")))
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
    crate::config::write_private_file(path, bytes).map_err(io_error(operation))
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path, operation: &'static str) -> Result<(), DaemonError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(io_error(operation))?;
    directory
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(io_error(operation))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path, _operation: &'static str) -> Result<(), DaemonError> {
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

    // MP-08/MP-10: reject incompatible metadata at registration, before storage.
    #[test]
    fn browser_registration_rejects_non_browser_injection_before_writing() {
        let root = crate::test_support::TestWorktree::new("browser-credential-registration");
        let registry_root = root.path().join("registry");
        let registry = CharioxCredentialRegistry::new(registry_root.clone());
        let mut credential = UserCredentialConfig {
            id: "fixture-browser".into(),
            description: None,
            source: UserCredentialSourceConfig::Vault {
                key: "fixture-browser".into(),
            },
            allowed_hosts: vec!["fixture.test".into()],
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Basic {
                username: "fixture-user".into(),
            },
            metadata: None,
        };
        let source = root.path().join("credential.yaml");
        fs::write(&source, serde_yaml::to_string(&credential).unwrap()).unwrap();
        let error = registry.install_from_file(&source).unwrap_err();
        assert!(error.to_string().contains("injection.kind=browser"));
        assert!(
            !registry_root.exists(),
            "invalid metadata must not create registry state"
        );
        let error = registry.upsert(credential.clone()).unwrap_err();
        assert!(error.to_string().contains("injection.kind=browser"));
        assert!(!registry_root.exists());
        // Basic remains valid for HTTP, including the legacy unrestricted use list.
        credential.allowed_uses = vec![UserCredentialUse::Http];
        registry.upsert(credential.clone()).unwrap();
        credential.allowed_uses.clear();
        registry.upsert(credential.clone()).unwrap();
        credential.allowed_uses = vec![UserCredentialUse::Browser];
        credential.injection = UserCredentialInjectionConfig::Browser;
        registry.upsert(credential.clone()).unwrap();
        assert_eq!(registry.get(&credential.id).unwrap(), Some(credential));
    }

    #[test]
    fn upsert_writes_and_replaces_credential_metadata() {
        let root = std::env::temp_dir().join(format!(
            "chariox-credential-upsert-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let registry = CharioxCredentialRegistry::new(root.clone());

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
            metadata: None,
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
            metadata: None,
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

#[cfg(all(test, unix))]
mod mp11_private_write_tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    #[test]
    fn mp11_registry_staging_alias_cannot_modify_an_unrelated_file() {
        let root =
            std::env::temp_dir().join(format!("mp11-registry-{:016x}", rand::random::<u64>()));
        fs::create_dir(&root).unwrap();
        let outside = root.join("unrelated");
        fs::write(&outside, b"unchanged").unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o644)).unwrap();
        let path = root.join("registration.yaml");
        let legacy_stage = root.join(format!(".registration.yaml.{}.tmp", std::process::id()));
        symlink(&outside, &legacy_stage).unwrap();
        atomic_write_private(&path, b"owned", "test").unwrap();
        let intact = fs::read(&outside).unwrap() == b"unchanged";
        let mode = fs::metadata(&outside).unwrap().permissions().mode() & 0o777;
        fs::remove_dir_all(root).unwrap();
        assert!(intact, "staging symlink overwrote unrelated bytes");
        assert_eq!(mode, 0o644, "staging symlink changed unrelated permissions");
    }

    #[test]
    fn mp11_registry_concurrent_writers_publish_complete_private_files() {
        let root =
            std::env::temp_dir().join(format!("mp11-registry-race-{:016x}", rand::random::<u64>()));
        fs::create_dir(&root).unwrap();
        let path = root.join("registration.yaml");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads = (0..8)
            .map(|index| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    atomic_write_private(&path, &vec![b'a' + index; 32768], "test").is_ok()
                })
            })
            .collect::<Vec<_>>();
        let successful = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|ok| *ok)
            .count();
        let bytes = fs::read(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        fs::remove_dir_all(root).unwrap();
        assert_eq!(successful, 8, "same-ID staging paths collided");
        assert_eq!(bytes.len(), 32768);
        assert!(bytes.iter().all(|byte| *byte == bytes[0]));
        assert_eq!(mode, 0o600);
    }
}

#[cfg(all(test, unix))]
#[test]
fn mp11_registry_fifo_staging_cannot_block_publication() {
    crate::test_support::assert_fifo_rejected(|fifo| {
        use std::os::unix::fs::symlink;
        let parent = fifo.parent().unwrap();
        let destination = parent.join("registration.yaml");
        let legacy = parent.join(format!(".registration.yaml.{}.tmp", std::process::id()));
        symlink(&fifo, legacy).unwrap();
        atomic_write_private(&destination, b"synthetic", "test").is_ok()
    });
}

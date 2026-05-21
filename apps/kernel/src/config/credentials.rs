use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

use super::{validate_config_key_path, validate_non_empty};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCredentialConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: UserCredentialSourceConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_uses: Vec<UserCredentialUse>,
    pub injection: UserCredentialInjectionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserCredentialSourceConfig {
    Env { name: String },
    File { path: String },
    Vault { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCredentialVaultConfig {
    #[serde(default = "default_credential_vault_backend")]
    pub backend: CredentialVaultBackend,
    #[serde(default = "default_credential_vault_service")]
    pub service: String,
}

impl Default for UserCredentialVaultConfig {
    fn default() -> Self {
        Self {
            backend: default_credential_vault_backend(),
            service: default_credential_vault_service(),
        }
    }
}

impl UserCredentialVaultConfig {
    pub fn service_name(&self) -> &str {
        self.service.trim()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialVaultBackend {
    OsKeychain,
}

fn default_credential_vault_backend() -> CredentialVaultBackend {
    CredentialVaultBackend::OsKeychain
}

fn default_credential_vault_service() -> String {
    "arroba".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserCredentialUse {
    Http,
    Pty,
    Connector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UserCredentialInjectionConfig {
    Header {
        name: String,
        value: String,
    },
    Query {
        name: String,
    },
    Basic {
        #[serde(default)]
        username: String,
    },
    Hmac {
        #[serde(default = "default_hmac_timestamp_header")]
        timestamp_header: String,
        #[serde(default = "default_hmac_signature_header")]
        signature_header: String,
    },
    Pty,
}

fn default_hmac_timestamp_header() -> String {
    "x-arroba-timestamp".to_string()
}

fn default_hmac_signature_header() -> String {
    "x-arroba-signature".to_string()
}

pub fn validate_credentials(credentials: &[UserCredentialConfig]) -> Result<(), DaemonError> {
    let mut seen = std::collections::BTreeSet::new();
    for credential in credentials {
        validate_config_key_path(&credential.id)?;
        if !seen.insert(credential.id.as_str()) {
            return Err(DaemonError::InvalidConfig {
                field: "credentials",
                message: "credential ids must be unique",
            });
        }
        match &credential.source {
            UserCredentialSourceConfig::Env { name } => {
                validate_non_empty("credentials.source.name", name)?;
            }
            UserCredentialSourceConfig::File { path } => {
                validate_non_empty("credentials.source.path", path)?;
            }
            UserCredentialSourceConfig::Vault { key } => {
                validate_non_empty("credentials.source.key", key)?;
            }
        }
        for host in &credential.allowed_hosts {
            validate_non_empty("credentials.allowed_hosts", host)?;
        }
        match &credential.injection {
            UserCredentialInjectionConfig::Header { name, value } => {
                validate_non_empty("credentials.injection.name", name)?;
                validate_non_empty("credentials.injection.value", value)?;
            }
            UserCredentialInjectionConfig::Query { name } => {
                validate_non_empty("credentials.injection.name", name)?;
            }
            UserCredentialInjectionConfig::Basic { .. } => {}
            UserCredentialInjectionConfig::Hmac {
                timestamp_header,
                signature_header,
            } => {
                validate_non_empty("credentials.injection.timestamp_header", timestamp_header)?;
                validate_non_empty("credentials.injection.signature_header", signature_header)?;
            }
            UserCredentialInjectionConfig::Pty => {}
        }
    }
    Ok(())
}

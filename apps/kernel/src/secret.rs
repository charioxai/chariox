use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{
    UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialSourceConfig,
    UserCredentialUse,
};
use crate::credential::ArrobaCredentialRegistry;
use crate::error::DaemonError;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialHandleView {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_uses: Vec<UserCredentialUse>,
    pub injection_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialHttpRequest {
    pub credential_id: String,
    #[serde(default = "default_http_method")]
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_json: Option<serde_json::Value>,
    #[serde(default = "default_http_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_http_max_response_bytes")]
    pub max_response_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialHttpResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSecretService {
    credentials: Vec<UserCredentialConfig>,
    vault_service: String,
    vault_store: Arc<dyn CredentialVaultStore>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultCredentialUpsertResult {
    pub credential_id: String,
    pub vault_key: String,
    pub stored: bool,
    pub metadata_path: PathBuf,
}

impl RuntimeSecretService {
    pub fn new(credentials: Vec<UserCredentialConfig>) -> Self {
        Self::with_vault_store(
            credentials,
            "arroba",
            Arc::new(PlatformKeychainCredentialVaultStore),
        )
    }

    pub fn with_vault_service(
        credentials: Vec<UserCredentialConfig>,
        vault_service: impl Into<String>,
    ) -> Self {
        Self::with_vault_store(
            credentials,
            vault_service,
            Arc::new(PlatformKeychainCredentialVaultStore),
        )
    }

    pub fn with_vault_store(
        credentials: Vec<UserCredentialConfig>,
        vault_service: impl Into<String>,
        vault_store: Arc<dyn CredentialVaultStore>,
    ) -> Self {
        Self {
            credentials,
            vault_service: vault_service.into(),
            vault_store,
        }
    }

    pub fn credential_env_names(&self) -> BTreeSet<String> {
        self.credentials
            .iter()
            .filter_map(|credential| match &credential.source {
                UserCredentialSourceConfig::Env { name } => Some(name.clone()),
                UserCredentialSourceConfig::File { .. }
                | UserCredentialSourceConfig::Vault { .. } => None,
            })
            .collect()
    }

    pub fn credential_env_names_from(credentials: &[UserCredentialConfig]) -> BTreeSet<String> {
        Self::new(credentials.to_vec()).credential_env_names()
    }

    pub fn list_handles(&self) -> Vec<CredentialHandleView> {
        self.credentials
            .iter()
            .map(|credential| CredentialHandleView {
                id: credential.id.clone(),
                description: credential.description.clone(),
                allowed_hosts: credential.allowed_hosts.clone(),
                allowed_uses: credential.allowed_uses.clone(),
                injection_kind: injection_kind(&credential.injection).to_string(),
            })
            .collect()
    }

    pub fn http_request_with_credential(
        &self,
        request: CredentialHttpRequest,
    ) -> Result<CredentialHttpResponse, DaemonError> {
        let credential = self.credential(&request.credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Http)?;
        let target = url::Url::parse(&request.url).map_err(|error| {
            secret_error(
                "http_request_with_credential",
                format!("invalid url: {error}"),
            )
        })?;
        self.ensure_host_allowed(credential, &target)?;
        let secret = self.resolve_secret(credential)?;

        let method = request.method.trim().to_ascii_uppercase();
        let mut headers = request.headers;
        let body = request_body(request.body_text, request.body_json)?;
        let mut target = target;

        match &credential.injection {
            UserCredentialInjectionConfig::Header { name, value } => {
                headers.insert(name.clone(), value.replace("${secret}", &secret));
            }
            UserCredentialInjectionConfig::Query { name } => {
                target.query_pairs_mut().append_pair(name, &secret);
            }
            UserCredentialInjectionConfig::Basic { username } => {
                let value = base64::engine::general_purpose::STANDARD
                    .encode(format!("{username}:{secret}"));
                headers.insert("authorization".to_string(), format!("Basic {value}"));
            }
            UserCredentialInjectionConfig::Hmac {
                timestamp_header,
                signature_header,
            } => {
                let timestamp = crate::session::unix_epoch_ms() / 1000;
                let body_hash = sha256_hex(body.as_deref().unwrap_or(""));
                let path_and_query = target[url::Position::BeforePath..].to_string();
                let canonical = format!("{method}\n{path_and_query}\n{body_hash}\n{timestamp}");
                let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|error| {
                    secret_error(
                        "http_request_with_credential",
                        format!("failed to initialize hmac: {error}"),
                    )
                })?;
                mac.update(canonical.as_bytes());
                headers.insert(timestamp_header.clone(), timestamp.to_string());
                headers.insert(
                    signature_header.clone(),
                    hex_bytes(&mac.finalize().into_bytes()),
                );
            }
            UserCredentialInjectionConfig::Pty => {
                return Err(secret_error(
                    "http_request_with_credential",
                    format!(
                        "credential `{}` is configured for terminal input",
                        credential.id
                    ),
                ));
            }
            UserCredentialInjectionConfig::Browser => {
                return Err(secret_error(
                    "http_request_with_credential",
                    format!(
                        "credential `{}` is configured for browser input",
                        credential.id
                    ),
                ));
            }
        }

        if request.timeout_ms == 0 {
            return Err(secret_error(
                "http_request_with_credential",
                "timeout_ms must be greater than zero".to_string(),
            ));
        }
        if request.max_response_bytes == 0 {
            return Err(secret_error(
                "http_request_with_credential",
                "max_response_bytes must be greater than zero".to_string(),
            ));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_millis(request.timeout_ms))
            .build();
        let mut http_request = match method.as_str() {
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" => {
                agent.request(&method, target.as_str())
            }
            _ => {
                return Err(secret_error(
                    "http_request_with_credential",
                    format!("unsupported HTTP method `{method}`"),
                ))
            }
        };
        for (name, value) in headers {
            http_request = http_request.set(&name, &value);
        }
        let response = if let Some(body) = body {
            http_request.send_string(&body)
        } else {
            http_request.call()
        }
        .map_err(|error| http_error("http_request_with_credential", error))?;

        decode_http_response(response, request.max_response_bytes)
    }

    pub fn terminal_secret_input(&self, credential_id: &str) -> Result<String, DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Pty)?;
        if !matches!(credential.injection, UserCredentialInjectionConfig::Pty) {
            return Err(secret_error(
                "credential_policy",
                format!("credential `{credential_id}` is not configured for terminal input"),
            ));
        }
        self.resolve_secret(credential)
    }

    pub fn browser_secret_input(&self, credential_id: &str) -> Result<String, DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Browser)?;
        if !matches!(credential.injection, UserCredentialInjectionConfig::Browser) {
            return Err(secret_error(
                "credential_policy",
                format!("credential `{credential_id}` is not configured for browser input"),
            ));
        }
        self.resolve_secret(credential)
    }

    pub fn resolve_connector_secret(
        &self,
        credential_id: &str,
    ) -> Result<(UserCredentialConfig, String), DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Connector)?;
        let secret = self.resolve_secret(credential)?;
        Ok((credential.clone(), secret))
    }

    pub fn resolve_mcp_secret(&self, credential_id: &str) -> Result<String, DaemonError> {
        let credential = self.credential(credential_id)?;
        self.ensure_use_allowed(credential, UserCredentialUse::Mcp)?;
        self.resolve_secret(credential)
    }

    pub fn set_vault_secret(&self, key: &str, value: &str) -> Result<(), DaemonError> {
        validate_vault_key(key)?;
        if value.is_empty() {
            return Err(secret_error(
                "credential_vault",
                "credential value must not be empty".to_string(),
            ));
        }
        self.vault_store
            .set_secret(self.vault_service_name()?, key.trim(), value)
    }

    pub fn delete_vault_secret(&self, key: &str) -> Result<(), DaemonError> {
        validate_vault_key(key)?;
        self.vault_store
            .delete_secret(self.vault_service_name()?, key.trim())
    }

    pub fn upsert_vault_backed_credential_with_secret(
        &self,
        registry: &ArrobaCredentialRegistry,
        credential: UserCredentialConfig,
        secret: &str,
        overwrite: bool,
    ) -> Result<VaultCredentialUpsertResult, DaemonError> {
        let vault_key = match &credential.source {
            UserCredentialSourceConfig::Vault { key } => key.trim().to_string(),
            UserCredentialSourceConfig::Env { .. } | UserCredentialSourceConfig::File { .. } => {
                return Err(secret_error(
                    "credential_vault_upsert",
                    "runtime-created credentials must use a vault source".to_string(),
                ));
            }
        };
        validate_vault_key(&vault_key)?;
        if !overwrite && registry.get(&credential.id)?.is_some() {
            return Err(secret_error(
                "credential_vault_upsert",
                format!(
                    "credential `{}` already exists; pass overwrite=true to replace it",
                    credential.id
                ),
            ));
        }
        self.set_vault_secret(&vault_key, secret)?;
        match registry.upsert(credential.clone()) {
            Ok((_credential, path)) => Ok(VaultCredentialUpsertResult {
                credential_id: credential.id,
                vault_key,
                stored: true,
                metadata_path: path,
            }),
            Err(error) => {
                let _ = self.delete_vault_secret(&vault_key);
                Err(error)
            }
        }
    }

    fn credential(&self, id: &str) -> Result<&UserCredentialConfig, DaemonError> {
        self.credentials
            .iter()
            .find(|credential| credential.id == id)
            .ok_or_else(|| secret_error("credential_lookup", format!("unknown credential `{id}`")))
    }

    fn resolve_secret(&self, credential: &UserCredentialConfig) -> Result<String, DaemonError> {
        match &credential.source {
            UserCredentialSourceConfig::Env { name } => std::env::var(name).map_err(|_| {
                secret_error(
                    "credential_resolve",
                    format!("credential `{}` env `{name}` is not set", credential.id),
                )
            }),
            UserCredentialSourceConfig::File { path } => {
                let path = expand_user_path(path);
                fs::read_to_string(&path)
                    .map(|value| value.trim_end().to_string())
                    .map_err(|error| {
                        secret_error(
                            "credential_resolve",
                            format!(
                                "failed to read credential `{}` file `{}`: {error}",
                                credential.id,
                                path.display()
                            ),
                        )
                    })
            }
            UserCredentialSourceConfig::Vault { key } => self
                .vault_store
                .get_secret(self.vault_service_name()?, key.trim())
                .map_err(|error| {
                    secret_error(
                        "credential_resolve",
                        format!(
                            "failed to resolve credential `{}` from vault key `{}`: {error}",
                            credential.id, key
                        ),
                    )
                }),
        }
    }

    fn vault_service_name(&self) -> Result<&str, DaemonError> {
        let service = self.vault_service.trim();
        if service.is_empty() {
            return Err(secret_error(
                "credential_vault",
                "credential vault service must not be empty".to_string(),
            ));
        }
        Ok(service)
    }

    fn ensure_use_allowed(
        &self,
        credential: &UserCredentialConfig,
        requested: UserCredentialUse,
    ) -> Result<(), DaemonError> {
        if credential.allowed_uses.is_empty() || credential.allowed_uses.contains(&requested) {
            return Ok(());
        }
        Err(secret_error(
            "credential_policy",
            format!(
                "credential `{}` is not allowed for {:?}",
                credential.id, requested
            ),
        ))
    }

    fn ensure_host_allowed(
        &self,
        credential: &UserCredentialConfig,
        target: &url::Url,
    ) -> Result<(), DaemonError> {
        if credential.allowed_hosts.is_empty() {
            return Ok(());
        }
        let Some(host) = target.host_str() else {
            return Err(secret_error(
                "credential_policy",
                format!("credential `{}` target has no host", credential.id),
            ));
        };
        let host_with_port = match target.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        if credential
            .allowed_hosts
            .iter()
            .any(|allowed| allowed == host || allowed == &host_with_port)
        {
            return Ok(());
        }
        Err(secret_error(
            "credential_policy",
            format!(
                "credential `{}` is not allowed for host `{host_with_port}`",
                credential.id
            ),
        ))
    }
}

pub trait CredentialVaultStore: Send + Sync + std::fmt::Debug {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError>;
    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError>;
    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError>;
}

#[derive(Debug, Default)]
pub struct PlatformKeychainCredentialVaultStore;

impl CredentialVaultStore for PlatformKeychainCredentialVaultStore {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        keyring_entry(service, key)?
            .get_password()
            .map_err(|error| vault_error("get", key, error))
    }

    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        keyring_entry(service, key)?
            .set_password(value)
            .map_err(|error| vault_error("set", key, error))
    }

    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        keyring_entry(service, key)?
            .delete_credential()
            .map_err(|error| vault_error("delete", key, error))
    }
}

pub fn secret_like_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.ends_with("_TOKEN")
        || upper.ends_with("_SECRET")
        || upper.ends_with("_PASSWORD")
        || upper.ends_with("_PASS")
        || upper.ends_with("_API_KEY")
        || upper.ends_with("_PRIVATE_KEY")
        || upper.ends_with("_ACCESS_KEY")
        || matches!(
            upper.as_str(),
            "TOKEN"
                | "SECRET"
                | "PASSWORD"
                | "API_KEY"
                | "PRIVATE_KEY"
                | "ACCESS_KEY"
                | "GITHUB_TOKEN"
                | "GH_TOKEN"
                | "OPENAI_API_KEY"
                | "ANTHROPIC_API_KEY"
        )
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_http_timeout_ms() -> u64 {
    30_000
}

fn default_http_max_response_bytes() -> u64 {
    1_048_576
}

fn request_body(
    body_text: Option<String>,
    body_json: Option<serde_json::Value>,
) -> Result<Option<String>, DaemonError> {
    match (body_text, body_json) {
        (Some(text), None) => Ok(Some(text)),
        (None, Some(json)) => serde_json::to_string(&json).map(Some).map_err(|error| {
            secret_error(
                "http_request_with_credential",
                format!("failed to encode JSON request body: {error}"),
            )
        }),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err(secret_error(
            "http_request_with_credential",
            "body_text and body_json are mutually exclusive".to_string(),
        )),
    }
}

fn decode_http_response(
    response: ureq::Response,
    max_response_bytes: u64,
) -> Result<CredentialHttpResponse, DaemonError> {
    let status = response.status();
    let mut body_text = String::new();
    let mut reader = response
        .into_reader()
        .take(max_response_bytes.saturating_add(1));
    reader.read_to_string(&mut body_text).map_err(|error| {
        secret_error(
            "http_request_with_credential",
            format!("failed to read response body: {error}"),
        )
    })?;
    if body_text.len() as u64 > max_response_bytes {
        return Err(secret_error(
            "http_request_with_credential",
            format!("response exceeded max_response_bytes ({max_response_bytes})"),
        ));
    }
    let body_json = serde_json::from_str::<serde_json::Value>(&body_text).ok();
    Ok(CredentialHttpResponse {
        status,
        body_text: body_json.is_none().then_some(body_text),
        body_json,
    })
}

fn http_error(operation: &'static str, error: ureq::Error) -> DaemonError {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response
                .into_string()
                .unwrap_or_else(|error| format!("failed to read error response: {error}"));
            secret_error(operation, format!("HTTP {code}: {body}"))
        }
        ureq::Error::Transport(error) => secret_error(operation, error.to_string()),
    }
}

fn sha256_hex(value: &str) -> String {
    hex_bytes(&Sha256::digest(value.as_bytes()))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn injection_kind(injection: &UserCredentialInjectionConfig) -> &'static str {
    match injection {
        UserCredentialInjectionConfig::Header { .. } => "header",
        UserCredentialInjectionConfig::Query { .. } => "query",
        UserCredentialInjectionConfig::Basic { .. } => "basic",
        UserCredentialInjectionConfig::Hmac { .. } => "hmac",
        UserCredentialInjectionConfig::Pty => "pty",
        UserCredentialInjectionConfig::Browser => "browser",
    }
}

fn expand_user_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn validate_vault_key(key: &str) -> Result<(), DaemonError> {
    if key.trim().is_empty() {
        return Err(secret_error(
            "credential_vault",
            "credential key must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn keyring_entry(service: &str, key: &str) -> Result<keyring::Entry, DaemonError> {
    keyring::Entry::new(service, key).map_err(|error| vault_error("open", key, error))
}

fn vault_error(operation: &'static str, key: &str, error: keyring::Error) -> DaemonError {
    secret_error(
        "credential_vault",
        format!(
            "failed to {operation} credential `{key}` in {}: {error}",
            platform_keychain_backend_name()
        ),
    )
}

fn platform_keychain_backend_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS Keychain"
    }
    #[cfg(target_os = "windows")]
    {
        "Windows Credential Manager"
    }
    #[cfg(target_os = "linux")]
    {
        "Linux keyutils/Secret Service"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "platform keychain"
    }
}

fn secret_error(operation: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport { operation, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        UserCredentialInjectionConfig, UserCredentialSourceConfig, UserCredentialUse,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[derive(Debug, Default)]
    struct MemoryVaultStore {
        secrets: Mutex<BTreeMap<(String, String), String>>,
    }

    impl CredentialVaultStore for MemoryVaultStore {
        fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
            self.secrets
                .lock()
                .unwrap()
                .get(&(service.to_string(), key.to_string()))
                .cloned()
                .ok_or_else(|| {
                    secret_error("credential_vault", format!("credential `{key}` not found"))
                })
        }

        fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
            self.secrets
                .lock()
                .unwrap()
                .insert((service.to_string(), key.to_string()), value.to_string());
            Ok(())
        }

        fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
            self.secrets
                .lock()
                .unwrap()
                .remove(&(service.to_string(), key.to_string()));
            Ok(())
        }
    }

    #[test]
    fn credential_handles_do_not_include_sources_or_values() {
        let service = RuntimeSecretService::new(vec![UserCredentialConfig {
            id: "github".to_string(),
            description: Some("GitHub API".to_string()),
            source: UserCredentialSourceConfig::Env {
                name: "GH_TOKEN".to_string(),
            },
            allowed_hosts: vec!["api.github.com".to_string()],
            allowed_uses: vec![UserCredentialUse::Http],
            injection: UserCredentialInjectionConfig::Header {
                name: "authorization".to_string(),
                value: "Bearer ${secret}".to_string(),
            },
        }]);

        let serialized = serde_json::to_string(&service.list_handles()).unwrap();
        assert!(serialized.contains("github"));
        assert!(!serialized.contains("GH_TOKEN"));
        assert!(!serialized.contains("${secret}"));
    }

    #[test]
    fn secret_like_env_name_matches_common_tokens() {
        assert!(secret_like_env_name("GITHUB_TOKEN"));
        assert!(secret_like_env_name("OPENAI_API_KEY"));
        assert!(secret_like_env_name("DB_PASSWORD"));
        assert!(!secret_like_env_name("PATH"));
    }

    #[test]
    fn http_request_with_credential_injects_header_without_returning_secret() {
        let _guard = crate::env_lock::lock();
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).expect("request should read");
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let saw_auth = request.contains("authorization: Bearer test-secret")
                || request.contains("Authorization: Bearer test-secret");
            let body = serde_json::json!({ "ok": saw_auth }).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should write");
        });

        std::env::set_var("ARROBA_TEST_SECRET_HTTP_TOKEN", "test-secret");
        let service = RuntimeSecretService::new(vec![UserCredentialConfig {
            id: "demo".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Env {
                name: "ARROBA_TEST_SECRET_HTTP_TOKEN".to_string(),
            },
            allowed_hosts: vec![format!("127.0.0.1:{port}")],
            allowed_uses: vec![UserCredentialUse::Http],
            injection: UserCredentialInjectionConfig::Header {
                name: "authorization".to_string(),
                value: "Bearer ${secret}".to_string(),
            },
        }]);

        let response = service
            .http_request_with_credential(CredentialHttpRequest {
                credential_id: "demo".to_string(),
                method: "GET".to_string(),
                url: format!("http://127.0.0.1:{port}/demo"),
                headers: BTreeMap::new(),
                body_text: None,
                body_json: None,
                timeout_ms: 30_000,
                max_response_bytes: 1_048_576,
            })
            .expect("credential request should succeed");
        std::env::remove_var("ARROBA_TEST_SECRET_HTTP_TOKEN");
        server.join().expect("server should finish");

        assert_eq!(response.status, 200);
        assert_eq!(response.body_json, Some(serde_json::json!({ "ok": true })));
        assert!(!serde_json::to_string(&response)
            .unwrap()
            .contains("test-secret"));
    }

    #[test]
    fn http_request_with_credential_rejects_wrong_host_before_secret_read() {
        let _guard = crate::env_lock::lock();
        std::env::remove_var("ARROBA_TEST_SECRET_MISSING_TOKEN");
        let service = RuntimeSecretService::new(vec![UserCredentialConfig {
            id: "demo".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Env {
                name: "ARROBA_TEST_SECRET_MISSING_TOKEN".to_string(),
            },
            allowed_hosts: vec!["api.example.com".to_string()],
            allowed_uses: vec![UserCredentialUse::Http],
            injection: UserCredentialInjectionConfig::Header {
                name: "authorization".to_string(),
                value: "Bearer ${secret}".to_string(),
            },
        }]);

        let error = service
            .http_request_with_credential(CredentialHttpRequest {
                credential_id: "demo".to_string(),
                method: "GET".to_string(),
                url: "http://127.0.0.1:1/demo".to_string(),
                headers: BTreeMap::new(),
                body_text: None,
                body_json: None,
                timeout_ms: 30_000,
                max_response_bytes: 1_048_576,
            })
            .expect_err("wrong host should be rejected");

        assert!(error.to_string().contains("not allowed for host"));
        assert!(!error
            .to_string()
            .contains("ARROBA_TEST_SECRET_MISSING_TOKEN"));
    }

    #[test]
    fn terminal_secret_input_requires_pty_injection() {
        let _guard = crate::env_lock::lock();
        std::env::set_var("ARROBA_TEST_TERMINAL_PASSWORD", "terminal-secret");
        let service = RuntimeSecretService::new(vec![UserCredentialConfig {
            id: "ssh_password".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Env {
                name: "ARROBA_TEST_TERMINAL_PASSWORD".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![UserCredentialUse::Pty],
            injection: UserCredentialInjectionConfig::Pty,
        }]);

        assert_eq!(
            service
                .terminal_secret_input("ssh_password")
                .expect("terminal secret should resolve"),
            "terminal-secret"
        );
        std::env::remove_var("ARROBA_TEST_TERMINAL_PASSWORD");
    }

    #[test]
    fn browser_secret_input_requires_browser_use() {
        let _guard = crate::env_lock::lock();
        std::env::set_var("ARROBA_TEST_BROWSER_PASSWORD", "browser-secret");
        let service = RuntimeSecretService::new(vec![UserCredentialConfig {
            id: "browser_password".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Env {
                name: "ARROBA_TEST_BROWSER_PASSWORD".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
        }]);

        assert_eq!(
            service
                .browser_secret_input("browser_password")
                .expect("browser secret should resolve"),
            "browser-secret"
        );
        std::env::remove_var("ARROBA_TEST_BROWSER_PASSWORD");
    }

    #[test]
    fn browser_secret_input_requires_browser_injection() {
        let _guard = crate::env_lock::lock();
        std::env::set_var("ARROBA_TEST_BROWSER_PASSWORD", "browser-secret");
        let service = RuntimeSecretService::new(vec![UserCredentialConfig {
            id: "browser_password".to_string(),
            description: None,
            source: UserCredentialSourceConfig::Env {
                name: "ARROBA_TEST_BROWSER_PASSWORD".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Header {
                name: "authorization".to_string(),
                value: "Bearer ${secret}".to_string(),
            },
        }]);

        let error = service
            .browser_secret_input("browser_password")
            .expect_err("browser input should require browser injection");
        assert!(error
            .to_string()
            .contains("not configured for browser input"));
        std::env::remove_var("ARROBA_TEST_BROWSER_PASSWORD");
    }

    #[test]
    fn upsert_vault_backed_credential_stores_secret_and_metadata() {
        let root = std::env::temp_dir().join(format!(
            "arroba-vault-credential-upsert-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let registry = ArrobaCredentialRegistry::new(root.clone());
        let vault = Arc::new(MemoryVaultStore::default());
        let service = RuntimeSecretService::with_vault_store(Vec::new(), "arroba-test", vault);
        let credential = UserCredentialConfig {
            id: "generated-browser-password".to_string(),
            description: Some("generated".to_string()),
            source: UserCredentialSourceConfig::Vault {
                key: "generated-browser-password".to_string(),
            },
            allowed_hosts: vec!["accounts.example.test".to_string()],
            allowed_uses: vec![UserCredentialUse::Browser],
            injection: UserCredentialInjectionConfig::Browser,
        };

        let result = service
            .upsert_vault_backed_credential_with_secret(
                &registry,
                credential.clone(),
                "secret-value",
                false,
            )
            .expect("vault-backed credential should store");

        assert_eq!(result.credential_id, "generated-browser-password");
        assert_eq!(result.vault_key, "generated-browser-password");
        assert_eq!(
            registry
                .get("generated-browser-password")
                .expect("credential should read"),
            Some(credential)
        );
        let resolving_service = RuntimeSecretService::with_vault_store(
            vec![registry
                .get("generated-browser-password")
                .expect("credential should read")
                .expect("credential should exist")],
            "arroba-test",
            service.vault_store.clone(),
        );
        assert_eq!(
            resolving_service
                .browser_secret_input("generated-browser-password")
                .expect("stored secret should resolve"),
            "secret-value"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn vault_source_resolves_without_exposing_secret_in_handles() {
        let vault = Arc::new(MemoryVaultStore::default());
        let service = RuntimeSecretService::with_vault_store(
            vec![UserCredentialConfig {
                id: "github".to_string(),
                description: None,
                source: UserCredentialSourceConfig::Vault {
                    key: "github-token".to_string(),
                },
                allowed_hosts: Vec::new(),
                allowed_uses: vec![UserCredentialUse::Pty],
                injection: UserCredentialInjectionConfig::Pty,
            }],
            "arroba-test",
            vault,
        );

        service
            .set_vault_secret("github-token", "vault-secret")
            .expect("vault secret should store");

        assert_eq!(
            service
                .terminal_secret_input("github")
                .expect("vault secret should resolve"),
            "vault-secret"
        );
        let serialized = serde_json::to_string(&service.list_handles()).unwrap();
        assert!(!serialized.contains("vault-secret"));
        assert!(!serialized.contains("github-token"));
    }
}

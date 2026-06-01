use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use jsonschema::JSONSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use wait_timeout::ChildExt;

use crate::config::{UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialUse};
use crate::error::DaemonError;
use crate::mcp::validate_registry_name;

pub const CONNECTOR_ADAPTER_PROTOCOL_VERSION: &str = "arroba-connector-adapter-v2";

#[derive(Debug, Clone, PartialEq)]
pub struct ArrobaConnectorRegistry {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrobaConnectorAdapterRegistry {
    user_root: PathBuf,
    bundled_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaConnectorDefinition {
    pub kind: String,
    pub name: String,
    pub description: String,
    pub adapter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<ConnectorCredentialPolicy>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: u64,
    pub operations: Vec<ConnectorOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaConnectorAdapterDefinition {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub adapter_protocol: String,
    pub command: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ConnectorAdapterSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorAdapterSource {
    User,
    Bundled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorCredentialPolicy {
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorOperation {
    pub name: String,
    pub description: String,
    #[serde(default = "default_safety")]
    pub safety: ConnectorSafety,
    pub input_schema: Value,
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorSafety {
    Read,
    Write,
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorExecution {
    pub connector: String,
    pub operation: String,
    pub safety: ConnectorSafety,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub request_type: ConnectorAdapterRequestType,
    pub connector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<ConnectorAdapterOperationValidation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<ConnectorAdapterCredential>,
    pub timeout_ms: u64,
    pub max_response_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorAdapterRequestType {
    Validate,
    Prepare,
    Call,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterOperationValidation {
    pub name: String,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterCredential {
    pub id: String,
    pub secret: String,
    pub injection: UserCredentialInjectionConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterPrepareResult {
    #[serde(default)]
    pub credential_targets: Vec<ConnectorAdapterCredentialTarget>,
    pub prepared_config: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectorAdapterCredentialTarget {
    Host {
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterResponse {
    pub id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub struct ConnectorAdapterProcessPool {
    processes: Arc<Mutex<BTreeMap<String, Arc<Mutex<WarmConnectorAdapterProcess>>>>>,
}

#[derive(Debug)]
struct WarmConnectorAdapterProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
    sequence: u64,
}

#[derive(Debug, Clone)]
pub struct PreparedConnectorCall {
    pub connector: String,
    pub operation: String,
    pub safety: ConnectorSafety,
    pub request: ConnectorAdapterRequest,
    pub adapter: ArrobaConnectorAdapterDefinition,
}

mod adapter_process;
mod adapter_registry;
mod definitions;
mod registry;

pub fn connector_tool_name(connector: &str, operation: &str) -> String {
    format!("{connector}_{operation}")
}

pub fn adapter_response_to_execution(
    prepared: PreparedConnectorCall,
    response: ConnectorAdapterResponse,
) -> Result<ConnectorExecution, DaemonError> {
    if !response.ok {
        return Err(connector_error(
            "connector.execute",
            response
                .error
                .unwrap_or_else(|| "connector adapter call failed".to_string()),
        ));
    }
    Ok(ConnectorExecution {
        connector: prepared.connector,
        operation: prepared.operation,
        safety: prepared.safety,
        result: response.result.unwrap_or(Value::Null),
    })
}

fn adapter_response_to_prepare_result(
    response: ConnectorAdapterResponse,
) -> Result<ConnectorAdapterPrepareResult, DaemonError> {
    if !response.ok {
        return Err(connector_error(
            "connector.prepare",
            response
                .error
                .unwrap_or_else(|| "connector adapter prepare failed".to_string()),
        ));
    }
    let result = response.result.ok_or_else(|| {
        connector_error(
            "connector.prepare",
            "connector adapter prepare returned no result".to_string(),
        )
    })?;
    serde_json::from_value::<ConnectorAdapterPrepareResult>(result).map_err(|error| {
        connector_error(
            "connector.prepare",
            format!("connector adapter prepare returned invalid result: {error}"),
        )
    })
}

fn validate_connector_with_adapter(
    definition: &ArrobaConnectorDefinition,
    adapter: &ArrobaConnectorAdapterDefinition,
) -> Result<(), DaemonError> {
    let request = ConnectorAdapterRequest {
        id: "validate-1".to_string(),
        request_type: ConnectorAdapterRequestType::Validate,
        connector: definition.name.clone(),
        operation: None,
        arguments: None,
        config: None,
        operations: definition
            .operations
            .iter()
            .map(|operation| ConnectorAdapterOperationValidation {
                name: operation.name.clone(),
                config: operation.config.clone(),
            })
            .collect(),
        credential: None,
        timeout_ms: definition.timeout_ms,
        max_response_bytes: definition.max_response_bytes.max(4096),
    };
    let response = run_adapter_request_once(adapter, &request)?;
    if response.ok {
        Ok(())
    } else {
        Err(connector_error(
            "connector.register",
            response
                .error
                .unwrap_or_else(|| "connector adapter validation failed".to_string()),
        ))
    }
}

fn run_adapter_request_once(
    adapter: &ArrobaConnectorAdapterDefinition,
    request: &ConnectorAdapterRequest,
) -> Result<ConnectorAdapterResponse, DaemonError> {
    let command = adapter.resolved_command()?;
    let mut child = Command::new(&command)
        .args(&adapter.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            connector_error(
                "connector.adapter",
                format!(
                    "failed to launch adapter `{}` with `{}`: {error}",
                    adapter.name,
                    command.display()
                ),
            )
        })?;
    let mut stdin = child.stdin.take().ok_or_else(|| {
        connector_error(
            "connector.adapter",
            format!("adapter `{}` did not expose stdin", adapter.name),
        )
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        connector_error(
            "connector.adapter",
            format!("adapter `{}` did not expose stdout", adapter.name),
        )
    })?;
    let payload = serde_json::to_string(request)
        .map_err(|error| connector_error("connector.adapter", error.to_string()))?;
    stdin
        .write_all(payload.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(io_error("connector.adapter"))?;
    drop(stdin);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = std::io::BufReader::new(stdout)
            .read_line(&mut line)
            .map(|_| line);
        let _ = sender.send(result);
    });
    let line = match receiver.recv_timeout(Duration::from_millis(request.timeout_ms)) {
        Ok(Ok(line)) => line,
        Ok(Err(error)) => return Err(io_error("connector.adapter")(error)),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(connector_error(
                "connector.adapter",
                format!("adapter request timed out after {}ms", request.timeout_ms),
            ));
        }
    };
    if line.len() > request.max_response_bytes as usize {
        let _ = child.kill();
        let _ = child.wait();
        return Err(connector_error(
            "connector.adapter",
            format!(
                "adapter response exceeded {} bytes",
                request.max_response_bytes
            ),
        ));
    }
    let _ = child.wait_timeout(Duration::from_millis(250));
    let _ = child.kill();
    let _ = child.wait();
    let response = serde_json::from_str::<ConnectorAdapterResponse>(&line).map_err(|error| {
        connector_error(
            "connector.adapter",
            format!("adapter returned invalid JSON: {error}"),
        )
    })?;
    if response.id != request.id {
        return Err(connector_error(
            "connector.adapter",
            format!(
                "adapter response id `{}` did not match request id `{}`",
                response.id, request.id
            ),
        ));
    }
    Ok(response)
}

fn connector_credential_metadata(
    credential_id: Option<&str>,
    required: bool,
) -> Result<Option<UserCredentialConfig>, DaemonError> {
    match (credential_id, required) {
        (None, true) => {
            return Err(connector_error(
                "connector.execute",
                "connector requires a credential grant".to_string(),
            ))
        }
        (None, false) => return Ok(None),
        (Some(_), _) => {}
    }
    let credential_id = credential_id.unwrap();
    let credential = crate::credential::ArrobaCredentialRegistry::user()?
        .get(credential_id)?
        .ok_or_else(|| {
            connector_error(
                "connector.execute",
                format!("unknown credential `{credential_id}`"),
            )
        })?;
    if !(credential.allowed_uses.is_empty()
        || credential
            .allowed_uses
            .contains(&UserCredentialUse::Connector))
    {
        return Err(connector_error(
            "connector.execute",
            format!("credential `{credential_id}` is not allowed for connector"),
        ));
    }
    Ok(Some(credential))
}

fn resolve_connector_credential(
    credential: Option<&UserCredentialConfig>,
    targets: &[ConnectorAdapterCredentialTarget],
    vault_service: impl Into<String>,
) -> Result<Option<ConnectorAdapterCredential>, DaemonError> {
    let Some(credential) = credential else {
        return Ok(None);
    };
    enforce_connector_credential_targets(credential, targets)?;
    let service = crate::secret::RuntimeSecretService::with_vault_service(
        vec![credential.clone()],
        vault_service.into(),
    );
    let (credential, secret) = service.resolve_connector_secret(&credential.id)?;
    Ok(Some(ConnectorAdapterCredential {
        id: credential.id,
        secret,
        injection: credential.injection,
        allowed_hosts: credential.allowed_hosts,
    }))
}

fn enforce_connector_credential_targets(
    credential: &UserCredentialConfig,
    targets: &[ConnectorAdapterCredentialTarget],
) -> Result<(), DaemonError> {
    if credential.allowed_hosts.is_empty() {
        return Ok(());
    }
    let mut saw_host = false;
    for target in targets {
        match target {
            ConnectorAdapterCredentialTarget::Host { host, port } => {
                saw_host = true;
                let host_with_port = port
                    .map(|port| format!("{host}:{port}"))
                    .unwrap_or_else(|| host.clone());
                if !credential
                    .allowed_hosts
                    .iter()
                    .any(|allowed| allowed == host || allowed == &host_with_port)
                {
                    return Err(connector_error(
                        "connector.execute",
                        format!(
                            "credential `{}` is not allowed for adapter-declared target `{host_with_port}`",
                            credential.id
                        ),
                    ));
                }
            }
        }
    }
    if !saw_host {
        return Err(connector_error(
            "connector.execute",
            format!(
                "credential `{}` is host-restricted but adapter did not declare a credential host target",
                credential.id
            ),
        ));
    }
    Ok(())
}

fn validate_arguments(schema: &Value, arguments: &Value) -> Result<(), DaemonError> {
    let compiled = JSONSchema::compile(schema).map_err(|error| {
        connector_error(
            "connector.execute",
            format!("invalid input schema: {error}"),
        )
    })?;
    if let Err(errors) = compiled.validate(arguments) {
        let details = errors.map(|error| error.to_string()).collect::<Vec<_>>();
        return Err(connector_error(
            "connector.execute",
            format!("invalid connector input: {}", details.join("; ")),
        ));
    }
    Ok(())
}

fn read_adapter_root(
    root: &Path,
    source: ConnectorAdapterSource,
    entries: &mut BTreeMap<String, ArrobaConnectorAdapterDefinition>,
) -> Result<(), DaemonError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(io_error("connector.adapter.list"))? {
        let path = entry.map_err(io_error("connector.adapter.list"))?.path();
        let manifest = path.join("adapter.yaml");
        if !manifest.exists() {
            continue;
        }
        let adapter = ArrobaConnectorAdapterRegistry::read_yaml(&manifest, source)?;
        entries.entry(adapter.name.clone()).or_insert(adapter);
    }
    Ok(())
}

fn copy_adapter_package(source_manifest: &Path, destination: &Path) -> Result<(), DaemonError> {
    let source_dir = source_manifest.parent().ok_or_else(|| {
        connector_error(
            "connector.adapter.register",
            "adapter manifest path has no parent".to_string(),
        )
    })?;
    for entry in fs::read_dir(source_dir).map_err(io_error("connector.adapter.register"))? {
        let entry = entry.map_err(io_error("connector.adapter.register"))?;
        let path = entry.path();
        let dest = destination.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            fs::copy(&path, &dest).map_err(io_error("connector.adapter.register"))?;
            set_private_file_permissions(&dest, "connector.adapter.register")?;
        }
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), DaemonError> {
    fs::create_dir_all(destination).map_err(io_error("connector.adapter.register"))?;
    set_private_dir_permissions(destination, "connector.adapter.register")?;
    for entry in fs::read_dir(source).map_err(io_error("connector.adapter.register"))? {
        let entry = entry.map_err(io_error("connector.adapter.register"))?;
        let path = entry.path();
        let dest = destination.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            fs::copy(&path, &dest).map_err(io_error("connector.adapter.register"))?;
            set_private_file_permissions(&dest, "connector.adapter.register")?;
        }
    }
    Ok(())
}

fn adapter_process_key(
    run_id: &str,
    connector: &str,
    adapter: &ArrobaConnectorAdapterDefinition,
) -> Result<String, DaemonError> {
    Ok(format!(
        "{}:{}:{}:{}",
        run_id,
        connector,
        adapter.name,
        adapter.resolved_command()?.display()
    ))
}

fn bundled_adapter_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = std::env::var_os("ARROBA_CONNECTOR_ADAPTER_BUNDLED_DIR") {
        roots.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.join("connector-adapters"));
            roots.push(parent.join("connectors").join("adapters"));
        }
    }
    roots
}

fn arroba_home() -> Option<PathBuf> {
    std::env::var_os("ARROBA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".arroba")))
}

fn default_timeout_ms() -> u64 {
    30_000
}

fn default_max_response_bytes() -> u64 {
    1_048_576
}

fn default_safety() -> ConnectorSafety {
    ConnectorSafety::Read
}

pub fn connector_error(operation: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport { operation, message }
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
            .unwrap_or("connector"),
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

#[allow(dead_code)]
fn _assert_credential_use_connector_exists(_: UserCredentialUse) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "arroba-connector-adapter-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn connector_validation_requires_adapter_name() {
        let definition = ArrobaConnectorDefinition {
            kind: "connector".to_string(),
            name: "demo".to_string(),
            description: "Demo connector".to_string(),
            adapter: "".to_string(),
            credential: None,
            timeout_ms: 30_000,
            max_response_bytes: 1024,
            operations: vec![ConnectorOperation {
                name: "lookup".to_string(),
                description: "Lookup".to_string(),
                safety: ConnectorSafety::Read,
                input_schema: serde_json::json!({"type": "object"}),
                config: serde_json::json!({}),
            }],
        };
        assert!(definition.validate().is_err());
    }

    #[test]
    fn connector_definition_hash_changes_with_projected_operations() {
        let mut definition = ArrobaConnectorDefinition {
            kind: "connector".to_string(),
            name: "demo".to_string(),
            description: "Demo connector".to_string(),
            adapter: "demo_adapter".to_string(),
            credential: None,
            timeout_ms: 30_000,
            max_response_bytes: 1024,
            operations: vec![ConnectorOperation {
                name: "lookup".to_string(),
                description: "Lookup".to_string(),
                safety: ConnectorSafety::Read,
                input_schema: serde_json::json!({"type": "object"}),
                config: serde_json::json!({}),
            }],
        };
        let initial = definition
            .definition_hash()
            .expect("connector hash should compute");
        definition.operations[0].description = "Lookup v2".to_string();
        let updated = definition
            .definition_hash()
            .expect("updated connector hash should compute");

        assert_ne!(initial, updated);
    }

    #[test]
    fn adapter_registry_reads_user_adapter() {
        let root = temp_root("adapter-registry");
        let adapter_dir = root.join("adapters").join("echo");
        fs::create_dir_all(&adapter_dir).unwrap();
        fs::write(
            adapter_dir.join("adapter.yaml"),
            r#"
kind: connector_adapter
name: echo
adapter_protocol: arroba-connector-adapter-v2
command: bin/echo-adapter
"#,
        )
        .unwrap();
        let registry = ArrobaConnectorAdapterRegistry::new(root.join("adapters"), Vec::new());
        let adapter = registry.get("echo").unwrap().unwrap();
        assert_eq!(adapter.name, "echo");
        assert_eq!(adapter.source, Some(ConnectorAdapterSource::User));
        assert_eq!(
            adapter.resolved_command().unwrap(),
            adapter_dir.join("bin/echo-adapter")
        );
    }

    #[test]
    fn adapter_command_without_path_uses_path_lookup() {
        let adapter = ArrobaConnectorAdapterDefinition {
            kind: "connector_adapter".to_string(),
            name: "path_lookup_adapter".to_string(),
            description: None,
            version: None,
            adapter_protocol: CONNECTOR_ADAPTER_PROTOCOL_VERSION.to_string(),
            command: PathBuf::from("any-adapter-command"),
            args: Vec::new(),
            source: Some(ConnectorAdapterSource::Bundled),
            manifest_path: Some(PathBuf::from(
                "/opt/arroba/connector-adapters/path_lookup_adapter/adapter.yaml",
            )),
        };
        assert_eq!(
            adapter.resolved_command().unwrap(),
            PathBuf::from("any-adapter-command")
        );
    }

    #[test]
    fn adapter_registry_reads_bundled_adapter() {
        let root = temp_root("bundled-adapters");
        let adapter_dir = root.join("bundled").join("any_adapter");
        fs::create_dir_all(&adapter_dir).unwrap();
        fs::write(
            adapter_dir.join("adapter.yaml"),
            r#"
kind: connector_adapter
name: any_adapter
adapter_protocol: arroba-connector-adapter-v2
command: any-adapter-command
"#,
        )
        .unwrap();
        let registry =
            ArrobaConnectorAdapterRegistry::new(root.join("user"), vec![root.join("bundled")]);
        let adapters = registry.list().expect("bundled adapters should parse");
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].name, "any_adapter");
        assert_eq!(adapters[0].source, Some(ConnectorAdapterSource::Bundled));
    }

    #[test]
    fn connector_credential_target_allows_matching_host() {
        let credential = test_connector_credential(vec!["api.example.com:443".to_string()]);
        enforce_connector_credential_targets(
            &credential,
            &[ConnectorAdapterCredentialTarget::Host {
                host: "api.example.com".to_string(),
                port: Some(443),
            }],
        )
        .expect("matching host target should be allowed");
    }

    #[test]
    fn connector_credential_target_requires_declared_host_for_restricted_credentials() {
        let credential = test_connector_credential(vec!["api.example.com".to_string()]);
        let error = enforce_connector_credential_targets(&credential, &[])
            .expect_err("restricted credential should require a host target");
        assert!(format!("{error}").contains("did not declare a credential host target"));
    }

    #[test]
    fn connector_credential_target_rejects_unlisted_host() {
        let credential = test_connector_credential(vec!["api.example.com".to_string()]);
        let error = enforce_connector_credential_targets(
            &credential,
            &[ConnectorAdapterCredentialTarget::Host {
                host: "other.example.com".to_string(),
                port: None,
            }],
        )
        .expect_err("unlisted host should be rejected");
        assert!(format!("{error}").contains("is not allowed for adapter-declared target"));
    }

    #[test]
    fn connector_credential_target_rejects_partially_allowed_targets() {
        let credential = test_connector_credential(vec!["api.example.com".to_string()]);
        let error = enforce_connector_credential_targets(
            &credential,
            &[
                ConnectorAdapterCredentialTarget::Host {
                    host: "api.example.com".to_string(),
                    port: None,
                },
                ConnectorAdapterCredentialTarget::Host {
                    host: "other.example.com".to_string(),
                    port: None,
                },
            ],
        )
        .expect_err("every declared target should be allowed");
        assert!(format!("{error}").contains("other.example.com"));
    }

    fn test_connector_credential(allowed_hosts: Vec<String>) -> UserCredentialConfig {
        UserCredentialConfig {
            id: "test-credential".to_string(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Env {
                name: "TEST_CREDENTIAL".to_string(),
            },
            allowed_hosts,
            allowed_uses: vec![UserCredentialUse::Connector],
            injection: UserCredentialInjectionConfig::Basic {
                username: "user".to_string(),
            },
        }
    }
}

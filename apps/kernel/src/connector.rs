use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use jsonschema::JSONSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::DaemonError;
use crate::mcp::validate_registry_name;

#[derive(Debug, Clone, PartialEq)]
pub struct ArrobaConnectorRegistry {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaConnectorDefinition {
    pub kind: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub connector_type: ConnectorType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<ConnectorCredentialPolicy>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: u64,
    pub operations: Vec<ConnectorOperation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorType {
    Http,
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
    pub request: HttpConnectorRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorSafety {
    Read,
    Write,
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpConnectorRequest {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_json: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorExecution {
    pub connector: String,
    pub operation: String,
    pub safety: ConnectorSafety,
    pub response: crate::secret::CredentialHttpResponse,
}

impl ArrobaConnectorRegistry {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn user_root() -> Option<PathBuf> {
        arroba_home().map(|home| home.join("connectors"))
    }

    pub fn user() -> Result<Self, DaemonError> {
        let root = Self::user_root().ok_or_else(|| DaemonError::InvalidConfig {
            field: "connector registry root",
            message: "HOME must be set to resolve ~/.arroba/connectors",
        })?;
        Ok(Self::new(root))
    }

    pub fn install_from_file(
        &self,
        source: &Path,
    ) -> Result<(ArrobaConnectorDefinition, PathBuf), DaemonError> {
        if !source.is_file() {
            return Err(DaemonError::InvalidConfig {
                field: "connector file",
                message: "connector registration requires a YAML file",
            });
        }
        let definition = Self::read_yaml(source)?;
        definition.validate()?;
        ensure_private_dir(&self.root, "connector.register")?;
        let path = self.path_for(&definition.name)?;
        let payload =
            serde_yaml::to_string(&definition).map_err(|error| DaemonError::LocalTransport {
                operation: "connector.register",
                message: format!(
                    "failed to serialize connector `{}`: {error}",
                    definition.name
                ),
            })?;
        atomic_write_private(&path, payload.as_bytes(), "connector.register")?;
        Ok((definition, path))
    }

    pub fn remove(&self, name: &str) -> Result<(ArrobaConnectorDefinition, PathBuf), DaemonError> {
        let path = self
            .find_path(name)?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "connector.remove",
                message: format!("connector `{name}` is not registered"),
            })?;
        let definition = Self::read_yaml(&path)?;
        fs::remove_file(&path).map_err(io_error("connector.remove"))?;
        Ok((definition, path))
    }

    pub fn list(&self) -> Result<Vec<ArrobaConnectorDefinition>, DaemonError> {
        let mut entries = BTreeMap::new();
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        for entry in fs::read_dir(&self.root).map_err(io_error("connector.list"))? {
            let path = entry.map_err(io_error("connector.list"))?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
                continue;
            }
            let definition = Self::read_yaml(&path)?;
            entries.entry(definition.name.clone()).or_insert(definition);
        }
        Ok(entries.into_values().collect())
    }

    pub fn get(&self, name: &str) -> Result<Option<ArrobaConnectorDefinition>, DaemonError> {
        let Some(path) = self.find_path(name)? else {
            return Ok(None);
        };
        Self::read_yaml(&path).map(Some)
    }

    pub fn execute(
        &self,
        connector_name: &str,
        operation_name: &str,
        credential_id: Option<&str>,
        max_safety: ConnectorSafety,
        arguments: Value,
        vault_service: impl Into<String>,
    ) -> Result<ConnectorExecution, DaemonError> {
        let definition = self.get(connector_name)?.ok_or_else(|| {
            connector_error(
                "connector.execute",
                format!("connector `{connector_name}` is not registered"),
            )
        })?;
        let operation = definition.operation(operation_name)?.clone();
        if operation.safety > max_safety {
            return Err(connector_error(
                "connector.execute",
                format!(
                    "operation `{connector_name}.{operation_name}` requires {:?} safety but grant allows {:?}",
                    operation.safety, max_safety
                ),
            ));
        }
        validate_arguments(&operation.input_schema, &arguments)?;
        let request = render_http_request(&definition, &operation, &arguments, credential_id)?;
        let response = if let Some(credential_id) = credential_id {
            let credentials = crate::credential::load_user_credentials()?;
            let service = crate::secret::RuntimeSecretService::with_vault_service(
                credentials,
                vault_service.into(),
            );
            service.http_request_with_credential(crate::secret::CredentialHttpRequest {
                credential_id: credential_id.to_string(),
                method: request.method,
                url: request.url,
                headers: request.headers,
                body_text: request.body_text,
                body_json: request.body_json,
                timeout_ms: request.timeout_ms,
                max_response_bytes: request.max_response_bytes,
            })?
        } else {
            execute_plain_http_request(request)?
        };
        Ok(ConnectorExecution {
            connector: connector_name.to_string(),
            operation: operation_name.to_string(),
            safety: operation.safety,
            response,
        })
    }

    pub fn path_for(&self, name: &str) -> Result<PathBuf, DaemonError> {
        validate_registry_name(name, "connector name")?;
        Ok(self.root.join(format!("{name}.yaml")))
    }

    fn find_path(&self, name: &str) -> Result<Option<PathBuf>, DaemonError> {
        let path = self.path_for(name)?;
        Ok(path.exists().then_some(path))
    }

    fn read_yaml(path: &Path) -> Result<ArrobaConnectorDefinition, DaemonError> {
        let contents = fs::read_to_string(path).map_err(io_error("connector.read"))?;
        let definition =
            serde_yaml::from_str::<ArrobaConnectorDefinition>(&contents).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "connector.read",
                    message: format!("failed to parse connector `{}`: {error}", path.display()),
                }
            })?;
        definition.validate()?;
        Ok(definition)
    }
}

impl ArrobaConnectorDefinition {
    pub fn validate(&self) -> Result<(), DaemonError> {
        if self.kind != "connector" {
            return Err(DaemonError::InvalidConfig {
                field: "kind",
                message: "connector YAML kind must be `connector`",
            });
        }
        validate_registry_name(&self.name, "connector name")?;
        if self.description.trim().is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "description",
                message: "connector description must not be empty",
            });
        }
        match self.connector_type {
            ConnectorType::Http => {
                let base_url = self.base_url.as_deref().ok_or(DaemonError::InvalidConfig {
                    field: "base_url",
                    message: "HTTP connectors require base_url",
                })?;
                url::Url::parse(base_url).map_err(|error| {
                    connector_error("connector.validate", format!("invalid base_url: {error}"))
                })?;
            }
        }
        if self.timeout_ms == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "timeout_ms",
                message: "connector timeout_ms must be greater than zero",
            });
        }
        if self.max_response_bytes == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "max_response_bytes",
                message: "connector max_response_bytes must be greater than zero",
            });
        }
        if self.operations.is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "operations",
                message: "connector must define at least one operation",
            });
        }
        let mut seen = BTreeSet::new();
        for operation in &self.operations {
            validate_registry_name(&operation.name, "operation name")?;
            if !seen.insert(operation.name.as_str()) {
                return Err(DaemonError::InvalidConfig {
                    field: "operations.name",
                    message: "operation names must be unique",
                });
            }
            if operation.description.trim().is_empty() {
                return Err(DaemonError::InvalidConfig {
                    field: "operations.description",
                    message: "operation description must not be empty",
                });
            }
            JSONSchema::compile(&operation.input_schema).map_err(|error| {
                connector_error(
                    "connector.validate",
                    format!("invalid JSON schema: {error}"),
                )
            })?;
            if operation.request.path.trim().is_empty() {
                return Err(DaemonError::InvalidConfig {
                    field: "operations.request.path",
                    message: "HTTP operation path must not be empty",
                });
            }
            validate_http_method(&operation.request.method)?;
            if operation.request.body_json.is_some() && operation.request.body_text.is_some() {
                return Err(DaemonError::InvalidConfig {
                    field: "operations.request",
                    message: "body_json and body_text are mutually exclusive",
                });
            }
        }
        Ok(())
    }

    pub fn operation(&self, name: &str) -> Result<&ConnectorOperation, DaemonError> {
        self.operations
            .iter()
            .find(|operation| operation.name == name)
            .ok_or_else(|| {
                connector_error(
                    "connector.operation",
                    format!("connector `{}` has no operation `{name}`", self.name),
                )
            })
    }

    pub fn operation_tool_name(&self, operation: &str) -> String {
        connector_tool_name(&self.name, operation)
    }

    pub fn allowed_operation_tool_names(&self, max_safety: ConnectorSafety) -> Vec<String> {
        self.operations
            .iter()
            .filter(|operation| operation.safety <= max_safety)
            .map(|operation| self.operation_tool_name(&operation.name))
            .collect()
    }
}

impl ConnectorSafety {
    pub fn parse(value: Option<&str>) -> Result<Self, DaemonError> {
        match value.unwrap_or("read") {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "destructive" => Ok(Self::Destructive),
            other => Err(connector_error(
                "connector.safety",
                format!("unknown connector safety `{other}`"),
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
        }
    }
}

#[derive(Debug, Clone)]
struct RenderedHttpRequest {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body_text: Option<String>,
    body_json: Option<Value>,
    timeout_ms: u64,
    max_response_bytes: u64,
}

fn render_http_request(
    definition: &ArrobaConnectorDefinition,
    operation: &ConnectorOperation,
    arguments: &Value,
    credential_id: Option<&str>,
) -> Result<RenderedHttpRequest, DaemonError> {
    if definition
        .credential
        .as_ref()
        .map(|credential| credential.required)
        .unwrap_or(false)
        && credential_id.is_none()
    {
        return Err(connector_error(
            "connector.execute",
            format!(
                "connector `{}` requires a credential grant",
                definition.name
            ),
        ));
    }
    let base_url = definition
        .base_url
        .as_deref()
        .ok_or(DaemonError::InvalidConfig {
            field: "base_url",
            message: "HTTP connectors require base_url",
        })?;
    let path = render_template_string(&operation.request.path, arguments)?;
    let mut base = base_url.to_string();
    if !base.ends_with('/') {
        base.push('/');
    }
    let mut url = url::Url::parse(&base)
        .map_err(|error| {
            connector_error("connector.execute", format!("invalid base_url: {error}"))
        })?
        .join(path.trim_start_matches('/'))
        .map_err(|error| {
            connector_error(
                "connector.execute",
                format!("invalid request path: {error}"),
            )
        })?;
    if !operation.request.query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (name, value) in &operation.request.query {
            pairs.append_pair(name, &render_template_string(value, arguments)?);
        }
    }
    let mut headers = operation
        .request
        .headers
        .iter()
        .map(|(name, value)| Ok((name.clone(), render_template_string(value, arguments)?)))
        .collect::<Result<BTreeMap<_, _>, DaemonError>>()?;
    if operation.request.body_json.is_some()
        && !headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        headers.insert("content-type".to_string(), "application/json".to_string());
    }
    Ok(RenderedHttpRequest {
        method: operation.request.method.trim().to_ascii_uppercase(),
        url: url.to_string(),
        headers,
        body_text: operation
            .request
            .body_text
            .as_ref()
            .map(|value| render_template_string(value, arguments))
            .transpose()?,
        body_json: operation
            .request
            .body_json
            .as_ref()
            .map(|value| render_json_template(value, arguments))
            .transpose()?,
        timeout_ms: definition.timeout_ms,
        max_response_bytes: definition.max_response_bytes,
    })
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

fn execute_plain_http_request(
    request: RenderedHttpRequest,
) -> Result<crate::secret::CredentialHttpResponse, DaemonError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_millis(request.timeout_ms))
        .build();
    let mut http_request = match request.method.as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" => {
            agent.request(&request.method, &request.url)
        }
        _ => {
            return Err(connector_error(
                "connector.execute",
                format!("unsupported HTTP method `{}`", request.method),
            ))
        }
    };
    for (name, value) in request.headers {
        http_request = http_request.set(&name, &value);
    }
    let body = match (request.body_text, request.body_json) {
        (Some(text), None) => Some(text),
        (None, Some(json)) => serde_json::to_string(&json)
            .map(Some)
            .map_err(|error| connector_error("connector.execute", error.to_string()))?,
        (None, None) => None,
        (Some(_), Some(_)) => {
            return Err(connector_error(
                "connector.execute",
                "body_text and body_json are mutually exclusive".to_string(),
            ))
        }
    };
    let response = if let Some(body) = body {
        http_request.send_string(&body)
    } else {
        http_request.call()
    }
    .map_err(|error| http_error("connector.execute", error))?;
    decode_http_response(response, request.max_response_bytes)
}

fn render_json_template(value: &Value, arguments: &Value) -> Result<Value, DaemonError> {
    match value {
        Value::String(text) if exact_template_key(text).is_some() => {
            let key = exact_template_key(text).unwrap();
            argument_value(arguments, key).cloned()
        }
        Value::String(text) => render_template_string(text, arguments).map(Value::String),
        Value::Array(items) => items
            .iter()
            .map(|item| render_json_template(item, arguments))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| Ok((key.clone(), render_json_template(value, arguments)?)))
            .collect::<Result<serde_json::Map<_, _>, DaemonError>>()
            .map(Value::Object),
        other => Ok(other.clone()),
    }
}

fn render_template_string(template: &str, arguments: &Value) -> Result<String, DaemonError> {
    let mut rendered = template.to_string();
    while let Some(start) = rendered.find("{{") {
        let Some(end) = rendered[start + 2..]
            .find("}}")
            .map(|index| start + 2 + index)
        else {
            return Err(connector_error(
                "connector.template",
                format!("unclosed template in `{template}`"),
            ));
        };
        let key = rendered[start + 2..end].trim();
        let value = argument_value(arguments, key)?;
        let replacement = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            Value::Null => String::new(),
            other => serde_json::to_string(other).map_err(|error| {
                connector_error(
                    "connector.template",
                    format!("failed to render `{key}`: {error}"),
                )
            })?,
        };
        rendered.replace_range(start..end + 2, &replacement);
    }
    Ok(rendered)
}

fn exact_template_key(template: &str) -> Option<&str> {
    let trimmed = template.trim();
    trimmed
        .strip_prefix("{{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn argument_value<'a>(arguments: &'a Value, key: &str) -> Result<&'a Value, DaemonError> {
    arguments.get(key).ok_or_else(|| {
        connector_error(
            "connector.template",
            format!("missing connector input field `{key}`"),
        )
    })
}

fn validate_http_method(method: &str) -> Result<(), DaemonError> {
    match method.trim().to_ascii_uppercase().as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" => Ok(()),
        other => Err(connector_error(
            "connector.validate",
            format!("unsupported HTTP method `{other}`"),
        )),
    }
}

fn decode_http_response(
    response: ureq::Response,
    max_response_bytes: u64,
) -> Result<crate::secret::CredentialHttpResponse, DaemonError> {
    let status = response.status();
    let mut body_text = String::new();
    let mut reader = response
        .into_reader()
        .take(max_response_bytes.saturating_add(1));
    reader.read_to_string(&mut body_text).map_err(|error| {
        connector_error(
            "connector.execute",
            format!("failed to read response body: {error}"),
        )
    })?;
    if body_text.len() as u64 > max_response_bytes {
        return Err(connector_error(
            "connector.execute",
            format!("response exceeded max_response_bytes ({max_response_bytes})"),
        ));
    }
    let body_json = serde_json::from_str::<Value>(&body_text).ok();
    Ok(crate::secret::CredentialHttpResponse {
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
            connector_error(operation, format!("HTTP {code}: {body}"))
        }
        ureq::Error::Transport(error) => connector_error(operation, error.to_string()),
    }
}

fn default_safety() -> ConnectorSafety {
    ConnectorSafety::Read
}

fn default_timeout_ms() -> u64 {
    30_000
}

fn default_max_response_bytes() -> u64 {
    1_048_576
}

pub fn connector_tool_name(connector: &str, operation: &str) -> String {
    format!("{connector}_{operation}")
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
    let parent = path
        .parent()
        .ok_or_else(|| connector_error(operation, "registry path has no parent".to_string()))?;
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
    move |error| connector_error(operation, error.to_string())
}

fn connector_error(operation: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport { operation, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn connector_fixture() -> ArrobaConnectorDefinition {
        ArrobaConnectorDefinition {
            kind: "connector".to_string(),
            name: "demo_api".to_string(),
            description: "Demo API".to_string(),
            connector_type: ConnectorType::Http,
            base_url: Some("http://127.0.0.1/base".to_string()),
            credential: None,
            timeout_ms: 500,
            max_response_bytes: 128,
            operations: vec![ConnectorOperation {
                name: "lookup".to_string(),
                description: "Lookup a value".to_string(),
                safety: ConnectorSafety::Read,
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["id", "payload"],
                    "properties": {
                        "id": {"type": "string"},
                        "payload": {}
                    },
                    "additionalProperties": false
                }),
                request: HttpConnectorRequest {
                    method: "POST".to_string(),
                    path: "/items/{{id}}".to_string(),
                    query: BTreeMap::new(),
                    headers: BTreeMap::new(),
                    body_json: Some(serde_json::json!({"payload": "{{payload}}"})),
                    body_text: None,
                },
            }],
        }
    }

    #[test]
    fn render_http_request_preserves_limits_and_sets_json_content_type() {
        let connector = connector_fixture();
        let operation = connector.operation("lookup").unwrap();
        let request = render_http_request(
            &connector,
            operation,
            &serde_json::json!({"id": "abc", "payload": [1, 2, 3]}),
            None,
        )
        .expect("request should render");

        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "http://127.0.0.1/base/items/abc");
        assert_eq!(request.timeout_ms, 500);
        assert_eq!(request.max_response_bytes, 128);
        assert_eq!(
            request.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            request.body_json,
            Some(serde_json::json!({"payload": [1, 2, 3]}))
        );
    }

    #[test]
    fn connector_validation_rejects_zero_limits() {
        let mut connector = connector_fixture();
        connector.timeout_ms = 0;
        assert!(connector.validate().is_err());
        connector.timeout_ms = 1;
        connector.max_response_bytes = 0;
        assert!(connector.validate().is_err());
    }

    #[test]
    fn plain_http_enforces_response_cap() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            let mut request = [0_u8; 512];
            let _ = stream.read(&mut request);
            let body = "x".repeat(64);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should write");
        });
        let error = execute_plain_http_request(RenderedHttpRequest {
            method: "GET".to_string(),
            url: format!("http://127.0.0.1:{port}/large"),
            headers: BTreeMap::new(),
            body_text: None,
            body_json: None,
            timeout_ms: 500,
            max_response_bytes: 8,
        })
        .expect_err("large response should fail");
        server.join().expect("server should finish");
        match error {
            DaemonError::LocalTransport { message, .. } => {
                assert!(message.contains("max_response_bytes"), "{message}");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn registry_install_uses_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "arroba-connector-registry-test-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let source = root.join("source.yaml");
        fs::create_dir_all(&root).expect("root should create");
        fs::write(
            &source,
            serde_yaml::to_string(&connector_fixture()).unwrap(),
        )
        .expect("source should write");
        let registry = ArrobaConnectorRegistry::new(root.join("registry"));
        let (_connector, path) = registry
            .install_from_file(&source)
            .expect("connector should install");

        assert_eq!(
            fs::metadata(root.join("registry"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(&root).ok();
    }
}

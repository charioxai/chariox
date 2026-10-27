use std::collections::BTreeMap;
use std::process::Command;

use chariox_adapters::protocol::{
    ConnectorAdapterCredentialTarget, ConnectorAdapterPrepareResult, ConnectorAdapterRequest,
    UserCredentialInjectionConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use chariox_adapters::connector_adapter_util::{
    enforce_allowed_host, render_json_template, render_template_string, run_adapter,
};

#[derive(Debug, Serialize, Deserialize)]
struct GrpcConfig {
    address: String,
    method: String,
    #[serde(default)]
    plaintext: bool,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    import_paths: Vec<String>,
    #[serde(default)]
    protos: Vec<String>,
    #[serde(default)]
    authority: Option<String>,
}

fn main() {
    run_adapter(validate_request, prepare_request, call_request);
}

fn validate_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    if request.operations.is_empty() {
        return Err("gRPC connector must define at least one operation".to_string());
    }
    for operation in request.operations {
        validate_config(&parse_config(operation.config)?)?;
    }
    Ok(serde_json::json!({"validated": true, "requires": "grpcurl"}))
}

fn prepare_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    let config = parse_config(
        request
            .config
            .ok_or_else(|| "gRPC prepare is missing operation config".to_string())?,
    )?;
    validate_config(&config)?;
    let arguments = request.arguments.unwrap_or_else(|| serde_json::json!({}));
    let prepared = prepare_config(&config, &arguments)?;
    let (host, port) = split_address_host(&prepared.address)?;
    serde_json::to_value(ConnectorAdapterPrepareResult {
        credential_targets: vec![ConnectorAdapterCredentialTarget::Host { host, port }],
        prepared_config: serde_json::to_value(prepared).map_err(|error| error.to_string())?,
    })
    .map_err(|error| error.to_string())
}

fn call_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    let config = parse_config(
        request
            .config
            .ok_or_else(|| "gRPC call is missing operation config".to_string())?,
    )?;
    validate_config(&config)?;
    let arguments = request.arguments.unwrap_or_else(|| serde_json::json!({}));
    let config = prepare_config(&config, &arguments)?;
    let address = config.address.clone();
    let method = config.method.clone();
    let mut headers = config.headers.clone();
    if let Some(credential) = request.credential {
        let (host, port) = split_address_host(&address)?;
        let host_with_port = port
            .map(|port| format!("{host}:{port}"))
            .unwrap_or_else(|| host.clone());
        enforce_allowed_host(&credential, &host, &host_with_port)?;
        match credential.injection {
            UserCredentialInjectionConfig::Header { name, value } => {
                headers.insert(name, value.replace("${secret}", &credential.secret));
            }
            UserCredentialInjectionConfig::Basic { username } => {
                use base64::Engine;
                let value = base64::engine::general_purpose::STANDARD
                    .encode(format!("{username}:{}", credential.secret));
                headers.insert("authorization".to_string(), format!("Basic {value}"));
            }
            UserCredentialInjectionConfig::Query { .. } => {
                return Err("gRPC adapter cannot use query credentials".to_string())
            }
            UserCredentialInjectionConfig::Hmac { .. } => {
                return Err("gRPC adapter does not support hmac credentials yet".to_string())
            }
            UserCredentialInjectionConfig::Pty => {
                return Err("gRPC adapter cannot use terminal credentials".to_string())
            }
        }
    }

    let data = config.data;
    let mut command = Command::new("grpcurl");
    command.arg("-format").arg("json");
    command
        .arg("-max-time")
        .arg(format!("{}", request.timeout_ms.div_ceil(1000).max(1)));
    if config.plaintext {
        command.arg("-plaintext");
    }
    if let Some(authority) = config.authority {
        command.arg("-authority").arg(authority);
    }
    for import_path in config.import_paths {
        command.arg("-import-path").arg(import_path);
    }
    for proto in config.protos {
        command.arg("-proto").arg(proto);
    }
    for (name, value) in headers {
        command.arg("-H").arg(format!("{name}: {value}"));
    }
    command
        .arg("-d")
        .arg(serde_json::to_string(&data).map_err(|error| error.to_string())?)
        .arg(address)
        .arg(method);

    let output = command
        .output()
        .map_err(|error| format!("failed to launch grpcurl: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(format!("grpcurl failed: {}", stderr.trim()));
    }
    if stdout.len() as u64 > request.max_response_bytes {
        return Err(format!(
            "response exceeded {} bytes",
            request.max_response_bytes
        ));
    }
    let body_json = serde_json::from_str::<Value>(&stdout).ok();
    Ok(serde_json::json!({
        "status": 0,
        "body_json": body_json,
        "body_text": if body_json.is_some() { None } else { Some(stdout) },
        "stderr": if stderr.trim().is_empty() { None::<String> } else { Some(stderr) },
    }))
}

fn prepare_config(config: &GrpcConfig, arguments: &Value) -> Result<GrpcConfig, String> {
    Ok(GrpcConfig {
        address: render_template_string(&config.address, arguments)?,
        method: render_template_string(&config.method, arguments)?,
        plaintext: config.plaintext,
        headers: config
            .headers
            .iter()
            .map(|(name, value)| Ok((name.clone(), render_template_string(value, arguments)?)))
            .collect::<Result<BTreeMap<_, _>, String>>()?,
        data: render_json_template(&config.data, arguments)?,
        import_paths: config
            .import_paths
            .iter()
            .map(|value| render_template_string(value, arguments))
            .collect::<Result<Vec<_>, _>>()?,
        protos: config
            .protos
            .iter()
            .map(|value| render_template_string(value, arguments))
            .collect::<Result<Vec<_>, _>>()?,
        authority: config
            .authority
            .as_ref()
            .map(|value| render_template_string(value, arguments))
            .transpose()?,
    })
}

fn parse_config(value: Value) -> Result<GrpcConfig, String> {
    serde_json::from_value::<GrpcConfig>(value)
        .map_err(|error| format!("invalid gRPC config: {error}"))
}

fn validate_config(config: &GrpcConfig) -> Result<(), String> {
    if config.address.trim().is_empty() {
        return Err("gRPC address must not be empty".to_string());
    }
    if config.method.trim().is_empty() {
        return Err("gRPC method must not be empty".to_string());
    }
    Ok(())
}

fn split_address_host(address: &str) -> Result<(String, Option<u16>), String> {
    let without_scheme = address
        .strip_prefix("http://")
        .or_else(|| address.strip_prefix("https://"))
        .unwrap_or(address);
    let host_with_port = without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .to_string();
    let host = host_with_port
        .strip_prefix('[')
        .and_then(|value| value.split(']').next())
        .map(str::to_string)
        .unwrap_or_else(|| host_with_port.split(':').next().unwrap_or("").to_string());
    if host.is_empty() {
        return Err("gRPC address host must not be empty".to_string());
    }
    let port = host_with_port
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok());
    Ok((host, port))
}

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{self, BufRead, Read, Write};

use crate::protocol::{
    ConnectorAdapterCredential, ConnectorAdapterCredentialTarget, ConnectorAdapterRequest,
    ConnectorAdapterRequestType, ConnectorAdapterResponse, UserCredentialInjectionConfig,
};
use serde_json::Value;

pub fn run_adapter(
    validate: fn(ConnectorAdapterRequest) -> Result<Value, String>,
    prepare: fn(ConnectorAdapterRequest) -> Result<Value, String>,
    call: fn(ConnectorAdapterRequest) -> Result<Value, String>,
) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    serde_json::json!({"id":"","ok":false,"error":error.to_string()})
                );
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<ConnectorAdapterRequest>(&line) {
            Ok(request) => request,
            Err(error) => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    serde_json::json!({"id":"","ok":false,"error":format!("invalid request: {error}")})
                );
                let _ = stdout.flush();
                continue;
            }
        };
        let id = request.id.clone();
        let response = match request.request_type {
            ConnectorAdapterRequestType::Validate => validate(request),
            ConnectorAdapterRequestType::Prepare => prepare(request),
            ConnectorAdapterRequestType::Call => call(request),
            ConnectorAdapterRequestType::Shutdown => break,
        };
        let response = match response {
            Ok(result) => ConnectorAdapterResponse {
                id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => ConnectorAdapterResponse {
                id,
                ok: false,
                result: None,
                error: Some(error),
            },
        };
        let _ = writeln!(stdout, "{}", serde_json::to_string(&response).unwrap());
        let _ = stdout.flush();
    }
}

pub fn credential_host_target(url: &url::Url) -> Result<ConnectorAdapterCredentialTarget, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "credential target has no host".to_string())?
        .to_string();
    Ok(ConnectorAdapterCredentialTarget::Host {
        host,
        port: url.port(),
    })
}

pub fn render_json_template(value: &Value, arguments: &Value) -> Result<Value, String> {
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
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(Value::Object),
        other => Ok(other.clone()),
    }
}

pub fn render_template_string(template: &str, arguments: &Value) -> Result<String, String> {
    let mut rendered = template.to_string();
    while let Some(start) = rendered.find("{{") {
        let Some(end) = rendered[start + 2..]
            .find("}}")
            .map(|index| start + 2 + index)
        else {
            return Err(format!("unclosed template in `{template}`"));
        };
        let key = rendered[start + 2..end].trim();
        let value = argument_value(arguments, key)?;
        let replacement = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            Value::Null => String::new(),
            other => serde_json::to_string(other)
                .map_err(|error| format!("failed to render `{key}`: {error}"))?,
        };
        rendered.replace_range(start..end + 2, &replacement);
    }
    Ok(rendered)
}

pub fn exact_template_key(template: &str) -> Option<&str> {
    let trimmed = template.trim();
    trimmed
        .strip_prefix("{{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub fn argument_value<'a>(arguments: &'a Value, key: &str) -> Result<&'a Value, String> {
    arguments
        .get(key)
        .ok_or_else(|| format!("missing connector input field `{key}`"))
}

pub fn enforce_allowed_host(
    credential: &ConnectorAdapterCredential,
    host: &str,
    host_with_port: &str,
) -> Result<(), String> {
    if credential.allowed_hosts.is_empty()
        || credential
            .allowed_hosts
            .iter()
            .any(|allowed| allowed == host || allowed == host_with_port)
    {
        return Ok(());
    }
    Err(format!(
        "credential `{}` is not allowed for host `{host_with_port}`",
        credential.id
    ))
}

pub fn inject_http_credential(
    url: &mut url::Url,
    headers: &mut BTreeMap<String, String>,
    credential: ConnectorAdapterCredential,
) -> Result<(), String> {
    let host = url
        .host_str()
        .ok_or_else(|| "credential host policy target has no host".to_string())?;
    let host_with_port = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    enforce_allowed_host(&credential, host, &host_with_port)?;
    match credential.injection {
        UserCredentialInjectionConfig::Header { name, value } => {
            headers.insert(name, value.replace("${secret}", &credential.secret));
        }
        UserCredentialInjectionConfig::Query { name } => {
            url.query_pairs_mut().append_pair(&name, &credential.secret);
        }
        UserCredentialInjectionConfig::Basic { username } => {
            use base64::Engine;
            let value = base64::engine::general_purpose::STANDARD
                .encode(format!("{username}:{}", credential.secret));
            headers.insert("authorization".to_string(), format!("Basic {value}"));
        }
        UserCredentialInjectionConfig::Hmac { .. } => {
            return Err("adapter does not support hmac credential injection yet".to_string())
        }
        UserCredentialInjectionConfig::Pty => {
            return Err("adapter cannot use terminal credentials".to_string())
        }
    }
    Ok(())
}

pub fn response_body_limited(response: ureq::Response, max_bytes: u64) -> Result<Value, String> {
    let status = response.status();
    let mut reader = response.into_reader().take(max_bytes.saturating_add(1));
    let mut body = String::new();
    std::io::Read::read_to_string(&mut reader, &mut body).map_err(|error| error.to_string())?;
    if body.len() as u64 > max_bytes {
        return Err(format!("response exceeded {max_bytes} bytes"));
    }
    let body_json = serde_json::from_str::<Value>(&body).ok();
    Ok(serde_json::json!({
        "status": status,
        "body_json": body_json,
        "body_text": if body_json.is_some() { None } else { Some(body) },
    }))
}

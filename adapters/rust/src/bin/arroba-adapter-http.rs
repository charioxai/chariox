use std::collections::BTreeMap;

use arroba_adapters::connector_adapter_util::run_adapter;
use arroba_adapters::protocol::{
    ConnectorAdapterCredentialTarget, ConnectorAdapterPrepareResult, ConnectorAdapterRequest,
    UserCredentialInjectionConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
struct HttpConfig {
    base_url: String,
    method: String,
    path: String,
    #[serde(default)]
    query: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body_json: Option<Value>,
    #[serde(default)]
    body_text: Option<String>,
}

fn main() {
    run_adapter(validate_request, prepare_request, call_request);
}

fn validate_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    if request.operations.is_empty() {
        return Err("HTTP connector must define at least one operation".to_string());
    }
    for operation in request.operations {
        let config = parse_config(operation.config)?;
        validate_config(&config)?;
    }
    Ok(serde_json::json!({"validated": true}))
}

fn prepare_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    let config = parse_config(
        request
            .config
            .ok_or_else(|| "HTTP prepare is missing operation config".to_string())?,
    )?;
    validate_config(&config)?;
    let arguments = request.arguments.unwrap_or_else(|| serde_json::json!({}));
    let prepared = prepare_config(&config, &arguments)?;
    let target = http_config_url(&prepared)?;
    serde_json::to_value(ConnectorAdapterPrepareResult {
        credential_targets: vec![host_target(&target)?],
        prepared_config: serde_json::to_value(prepared).map_err(|error| error.to_string())?,
    })
    .map_err(|error| error.to_string())
}

fn call_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    let config = parse_config(
        request
            .config
            .ok_or_else(|| "HTTP call is missing operation config".to_string())?,
    )?;
    validate_config(&config)?;
    let arguments = request.arguments.unwrap_or_else(|| serde_json::json!({}));
    let prepared = prepare_config(&config, &arguments)?;
    let mut url = http_config_url(&prepared)?;
    let mut headers = prepared.headers;
    let body_json = prepared.body_json;
    let body_text = prepared.body_text;
    if let Some(credential) = request.credential {
        if !credential.allowed_hosts.is_empty() {
            let host = url
                .host_str()
                .ok_or_else(|| "credential host policy target has no host".to_string())?;
            let host_with_port = match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            };
            if !credential
                .allowed_hosts
                .iter()
                .any(|allowed| allowed == host || allowed == &host_with_port)
            {
                return Err(format!(
                    "credential `{}` is not allowed for host `{host_with_port}`",
                    credential.id
                ));
            }
        }
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
                return Err(
                    "HTTP adapter does not support hmac credential injection yet".to_string(),
                )
            }
            UserCredentialInjectionConfig::Pty => {
                return Err("HTTP adapter cannot use terminal credentials".to_string())
            }
        }
    }

    let agent = http_agent(request.timeout_ms);
    let method = prepared.method.trim().to_ascii_uppercase();
    let mut http_request = agent.request(&method, url.as_str());
    for (name, value) in headers {
        http_request = http_request.set(&name, &value);
    }
    let response = match (body_text, body_json) {
        (Some(text), None) => http_request.send_string(&text),
        (None, Some(json)) => http_request
            .send_string(&serde_json::to_string(&json).map_err(|error| error.to_string())?),
        (None, None) => http_request.call(),
        (Some(_), Some(_)) => {
            return Err("body_text and body_json are mutually exclusive".to_string())
        }
    }
    .map_err(|error| error.to_string())?;
    decode_response(response, request.max_response_bytes)
}

fn http_agent(timeout_ms: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .redirects(0)
        .build()
}

fn prepare_config(config: &HttpConfig, arguments: &Value) -> Result<HttpConfig, String> {
    let body_json = config
        .body_json
        .as_ref()
        .map(|value| render_json_template(value, arguments))
        .transpose()?;
    let body_text = config
        .body_text
        .as_ref()
        .map(|value| render_template_string(value, arguments))
        .transpose()?;
    let mut headers = config
        .headers
        .iter()
        .map(|(name, value)| Ok((name.clone(), render_template_string(value, arguments)?)))
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    if body_json.is_some()
        && !headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        headers.insert("content-type".to_string(), "application/json".to_string());
    }
    Ok(HttpConfig {
        base_url: render_template_string(&config.base_url, arguments)?,
        method: render_template_string(&config.method, arguments)?,
        path: render_template_string(&config.path, arguments)?,
        query: config
            .query
            .iter()
            .map(|(name, value)| Ok((name.clone(), render_template_string(value, arguments)?)))
            .collect::<Result<BTreeMap<_, _>, String>>()?,
        headers,
        body_json,
        body_text,
    })
}

fn http_config_url(config: &HttpConfig) -> Result<url::Url, String> {
    let mut base = config.base_url.clone();
    if !base.ends_with('/') {
        base.push('/');
    }
    let mut url = url::Url::parse(&base)
        .map_err(|error| format!("invalid base_url: {error}"))?
        .join(config.path.trim_start_matches('/'))
        .map_err(|error| format!("invalid request path: {error}"))?;
    if !config.query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (name, value) in &config.query {
            pairs.append_pair(name, value);
        }
    }
    Ok(url)
}

fn host_target(url: &url::Url) -> Result<ConnectorAdapterCredentialTarget, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "credential target has no host".to_string())?
        .to_string();
    Ok(ConnectorAdapterCredentialTarget::Host {
        host,
        port: url.port(),
    })
}

fn parse_config(value: Value) -> Result<HttpConfig, String> {
    serde_json::from_value::<HttpConfig>(value)
        .map_err(|error| format!("invalid HTTP config: {error}"))
}

fn validate_config(config: &HttpConfig) -> Result<(), String> {
    let base_url =
        url::Url::parse(&config.base_url).map_err(|error| format!("invalid base_url: {error}"))?;
    if !matches!(base_url.scheme(), "http" | "https") {
        return Err("HTTP config base_url must use http or https".to_string());
    }
    if !base_url.username().is_empty() || base_url.password().is_some() {
        return Err("HTTP config base_url must not contain credentials".to_string());
    }
    if config.path.trim().is_empty() {
        return Err("HTTP config path must not be empty".to_string());
    }
    match config.method.trim().to_ascii_uppercase().as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" => {}
        other => return Err(format!("unsupported HTTP method `{other}`")),
    }
    if config.body_json.is_some() && config.body_text.is_some() {
        return Err("body_text and body_json are mutually exclusive".to_string());
    }
    Ok(())
}

fn render_json_template(value: &Value, arguments: &Value) -> Result<Value, String> {
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

fn render_template_string(template: &str, arguments: &Value) -> Result<String, String> {
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

fn exact_template_key(template: &str) -> Option<&str> {
    let trimmed = template.trim();
    trimmed
        .strip_prefix("{{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn argument_value<'a>(arguments: &'a Value, key: &str) -> Result<&'a Value, String> {
    arguments
        .get(key)
        .ok_or_else(|| format!("missing connector input field `{key}`"))
}

fn decode_response(response: ureq::Response, max_response_bytes: u64) -> Result<Value, String> {
    use std::io::Read;
    let status = response.status();
    let mut body_text = String::new();
    let mut reader = response
        .into_reader()
        .take(max_response_bytes.saturating_add(1));
    reader
        .read_to_string(&mut body_text)
        .map_err(|error| format!("failed to read response body: {error}"))?;
    if body_text.len() as u64 > max_response_bytes {
        return Err(format!("response exceeded {max_response_bytes} bytes"));
    }
    let body_json = serde_json::from_str::<Value>(&body_text).ok();
    Ok(serde_json::json!({
        "status": status,
        "body_text": if body_json.is_none() { Some(body_text) } else { None::<String> },
        "body_json": body_json
    }))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn http_agent_does_not_follow_redirects_after_policy_validation() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind redirect fixture");
        let address = listener
            .local_addr()
            .expect("read redirect fixture address");
        let fixture = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept redirect request");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).expect("read redirect request");
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/latest/meta-data\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("write redirect response");
        });

        let response = http_agent(1_000)
            .get(&format!("http://{address}/allowed"))
            .call()
            .expect("redirect response must be returned without following it");
        assert_eq!(response.status(), 302);
        fixture.join().expect("redirect fixture thread");
    }

    #[test]
    fn http_config_rejects_non_http_and_embedded_credentials() {
        let config = |base_url: &str| HttpConfig {
            base_url: base_url.to_string(),
            method: "GET".to_string(),
            path: "/resource".to_string(),
            query: BTreeMap::new(),
            headers: BTreeMap::new(),
            body_json: None,
            body_text: None,
        };

        assert!(validate_config(&config("ftp://example.com")).is_err());
        assert!(validate_config(&config("https://user:secret@example.com")).is_err());
        assert!(validate_config(&config("https://api.example.com")).is_ok());
    }
}

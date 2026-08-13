use std::collections::BTreeMap;

use chariox_adapters::protocol::{ConnectorAdapterPrepareResult, ConnectorAdapterRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use chariox_adapters::connector_adapter_util::{
    credential_host_target, inject_http_credential, render_json_template, render_template_string,
    response_body_limited, run_adapter,
};

#[derive(Debug, Serialize, Deserialize)]
struct GraphqlConfig {
    endpoint: String,
    query: String,
    #[serde(default)]
    operation_name: Option<String>,
    #[serde(default)]
    variables: Value,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

fn main() {
    run_adapter(validate_request, prepare_request, call_request);
}

fn validate_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    if request.operations.is_empty() {
        return Err("GraphQL connector must define at least one operation".to_string());
    }
    for operation in request.operations {
        validate_config(&parse_config(operation.config)?)?;
    }
    Ok(serde_json::json!({"validated": true}))
}

fn prepare_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    let config = parse_config(
        request
            .config
            .ok_or_else(|| "GraphQL prepare is missing operation config".to_string())?,
    )?;
    validate_config(&config)?;
    let arguments = request.arguments.unwrap_or_else(|| serde_json::json!({}));
    let prepared = prepare_config(&config, &arguments)?;
    let url = url::Url::parse(&prepared.endpoint)
        .map_err(|error| format!("invalid endpoint: {error}"))?;
    serde_json::to_value(ConnectorAdapterPrepareResult {
        credential_targets: vec![credential_host_target(&url)?],
        prepared_config: serde_json::to_value(prepared).map_err(|error| error.to_string())?,
    })
    .map_err(|error| error.to_string())
}

fn call_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    let config = parse_config(
        request
            .config
            .ok_or_else(|| "GraphQL call is missing operation config".to_string())?,
    )?;
    validate_config(&config)?;
    let arguments = request.arguments.unwrap_or_else(|| serde_json::json!({}));
    let config = prepare_config(&config, &arguments)?;
    let mut url =
        url::Url::parse(&config.endpoint).map_err(|error| format!("invalid endpoint: {error}"))?;
    let mut headers = config.headers;
    headers
        .entry("content-type".to_string())
        .or_insert_with(|| "application/json".to_string());
    if let Some(credential) = request.credential {
        inject_http_credential(&mut url, &mut headers, credential)?;
    }
    let body = serde_json::json!({
        "query": config.query,
        "operationName": config.operation_name,
        "variables": config.variables,
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_millis(request.timeout_ms))
        .build();
    let mut http_request = agent.post(url.as_str());
    for (name, value) in headers {
        http_request = http_request.set(&name, &value);
    }
    let response = http_request
        .send_string(&serde_json::to_string(&body).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    response_body_limited(response, request.max_response_bytes)
}

fn prepare_config(config: &GraphqlConfig, arguments: &Value) -> Result<GraphqlConfig, String> {
    Ok(GraphqlConfig {
        endpoint: render_template_string(&config.endpoint, arguments)?,
        query: render_template_string(&config.query, arguments)?,
        operation_name: config
            .operation_name
            .as_ref()
            .map(|value| render_template_string(value, arguments))
            .transpose()?,
        variables: render_json_template(&config.variables, arguments)?,
        headers: config
            .headers
            .iter()
            .map(|(name, value)| Ok((name.clone(), render_template_string(value, arguments)?)))
            .collect::<Result<BTreeMap<_, _>, String>>()?,
    })
}

fn parse_config(value: Value) -> Result<GraphqlConfig, String> {
    serde_json::from_value::<GraphqlConfig>(value)
        .map_err(|error| format!("invalid GraphQL config: {error}"))
}

fn validate_config(config: &GraphqlConfig) -> Result<(), String> {
    url::Url::parse(&config.endpoint).map_err(|error| format!("invalid endpoint: {error}"))?;
    if config.query.trim().is_empty() {
        return Err("GraphQL query must not be empty".to_string());
    }
    Ok(())
}

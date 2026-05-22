use arroba_adapters::protocol::{
    ConnectorAdapterPrepareResult, ConnectorAdapterRequest, UserCredentialInjectionConfig,
};
use postgres::types::{ToSql, Type};
use postgres::{Client, NoTls, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use arroba_adapters::connector_adapter_util::{
    credential_host_target, enforce_allowed_host, render_json_template, render_template_string,
    run_adapter,
};

#[derive(Debug, Serialize, Deserialize)]
struct PostgresConfig {
    connection_url: String,
    query: String,
    #[serde(default)]
    params: Vec<Value>,
}

fn main() {
    run_adapter(validate_request, prepare_request, call_request);
}

fn validate_request(request: ConnectorAdapterRequest) -> Result<Value, String> {
    if request.operations.is_empty() {
        return Err("Postgres connector must define at least one operation".to_string());
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
            .ok_or_else(|| "Postgres prepare is missing operation config".to_string())?,
    )?;
    validate_config(&config)?;
    let arguments = request.arguments.unwrap_or_else(|| serde_json::json!({}));
    let prepared = prepare_config(&config, &arguments)?;
    let url = url::Url::parse(&prepared.connection_url)
        .map_err(|error| format!("invalid connection_url: {error}"))?;
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
            .ok_or_else(|| "Postgres call is missing operation config".to_string())?,
    )?;
    validate_config(&config)?;
    let arguments = request.arguments.unwrap_or_else(|| serde_json::json!({}));
    let config = prepare_config(&config, &arguments)?;
    let mut connection_url = config.connection_url;
    if let Some(credential) = request.credential {
        connection_url = inject_connection_credential(&connection_url, credential)?;
    }
    let query = config.query;
    let params = config.params;
    let boxed_params = params
        .iter()
        .map(postgres_param)
        .collect::<Result<Vec<_>, _>>()?;
    let refs = boxed_params
        .iter()
        .map(|value| value.as_ref() as &(dyn ToSql + Sync))
        .collect::<Vec<_>>();
    let mut client = Client::connect(&connection_url, NoTls)
        .map_err(|error| format!("connect failed: {error}"))?;
    let rows = client
        .query(&query, &refs)
        .map_err(|error| format!("query failed: {error}"))?;
    let row_values = rows
        .iter()
        .map(row_to_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({
        "rows": row_values,
        "row_count": row_values.len(),
    }))
}

fn prepare_config(config: &PostgresConfig, arguments: &Value) -> Result<PostgresConfig, String> {
    Ok(PostgresConfig {
        connection_url: render_template_string(&config.connection_url, arguments)?,
        query: render_template_string(&config.query, arguments)?,
        params: config
            .params
            .iter()
            .map(|value| render_json_template(value, arguments))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn parse_config(value: Value) -> Result<PostgresConfig, String> {
    serde_json::from_value::<PostgresConfig>(value)
        .map_err(|error| format!("invalid Postgres config: {error}"))
}

fn validate_config(config: &PostgresConfig) -> Result<(), String> {
    let url = url::Url::parse(&config.connection_url)
        .map_err(|error| format!("invalid connection_url: {error}"))?;
    match url.scheme() {
        "postgres" | "postgresql" => {}
        other => return Err(format!("unsupported Postgres URL scheme `{other}`")),
    }
    if config.query.trim().is_empty() {
        return Err("Postgres query must not be empty".to_string());
    }
    Ok(())
}

fn inject_connection_credential(
    connection_url: &str,
    credential: arroba_adapters::protocol::ConnectorAdapterCredential,
) -> Result<String, String> {
    let mut url = url::Url::parse(connection_url)
        .map_err(|error| format!("invalid connection_url: {error}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| "credential host policy target has no host".to_string())?
        .to_string();
    let host_with_port = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.clone(),
    };
    enforce_allowed_host(&credential, &host, &host_with_port)?;
    match credential.injection {
        UserCredentialInjectionConfig::Basic { username } => {
            url.set_username(&username)
                .map_err(|_| "failed to set Postgres username".to_string())?;
            url.set_password(Some(&credential.secret))
                .map_err(|_| "failed to set Postgres password".to_string())?;
        }
        UserCredentialInjectionConfig::Query { name } => {
            url.query_pairs_mut().append_pair(&name, &credential.secret);
        }
        UserCredentialInjectionConfig::Header { .. } => {
            return Err("Postgres adapter cannot use header credentials".to_string())
        }
        UserCredentialInjectionConfig::Hmac { .. } => {
            return Err("Postgres adapter does not support hmac credentials yet".to_string())
        }
        UserCredentialInjectionConfig::Pty => {
            return Err("Postgres adapter cannot use terminal credentials".to_string())
        }
    }
    Ok(url.to_string())
}

fn postgres_param(value: &Value) -> Result<Box<dyn ToSql + Sync>, String> {
    match value {
        Value::Null => Ok(Box::new(None::<String>)),
        Value::Bool(value) => Ok(Box::new(*value)),
        Value::Number(value) if value.is_i64() => {
            let number = value.as_i64().unwrap();
            if let Ok(number) = i32::try_from(number) {
                Ok(Box::new(number))
            } else {
                Ok(Box::new(number))
            }
        }
        Value::Number(value) if value.is_u64() => {
            let number: i64 = value
                .as_u64()
                .unwrap()
                .try_into()
                .map_err(|_| "u64 parameter exceeds Postgres i64 range".to_string())?;
            Ok(Box::new(number))
        }
        Value::Number(value) if value.is_f64() => Ok(Box::new(value.as_f64().unwrap())),
        Value::String(value) => Ok(Box::new(value.clone())),
        other => Ok(Box::new(other.clone())),
    }
}

fn row_to_json(row: &Row) -> Result<Value, String> {
    let mut object = serde_json::Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        object.insert(
            column.name().to_string(),
            cell_to_json(row, index, column.type_())?,
        );
    }
    Ok(Value::Object(object))
}

fn cell_to_json(row: &Row, index: usize, column_type: &Type) -> Result<Value, String> {
    if *column_type == Type::BOOL {
        return Ok(row
            .try_get::<_, Option<bool>>(index)
            .map_err(|error| error.to_string())?
            .map(Value::Bool)
            .unwrap_or(Value::Null));
    }
    if *column_type == Type::INT2 || *column_type == Type::INT4 {
        return Ok(row
            .try_get::<_, Option<i32>>(index)
            .map_err(|error| error.to_string())?
            .map(|value| serde_json::json!(value))
            .unwrap_or(Value::Null));
    }
    if *column_type == Type::INT8 {
        return Ok(row
            .try_get::<_, Option<i64>>(index)
            .map_err(|error| error.to_string())?
            .map(|value| serde_json::json!(value))
            .unwrap_or(Value::Null));
    }
    if *column_type == Type::FLOAT4 || *column_type == Type::FLOAT8 {
        return Ok(row
            .try_get::<_, Option<f64>>(index)
            .map_err(|error| error.to_string())?
            .map(|value| serde_json::json!(value))
            .unwrap_or(Value::Null));
    }
    if *column_type == Type::JSON || *column_type == Type::JSONB {
        return Ok(row
            .try_get::<_, Option<Value>>(index)
            .map_err(|error| error.to_string())?
            .unwrap_or(Value::Null));
    }
    Ok(row
        .try_get::<_, Option<String>>(index)
        .map_err(|error| error.to_string())?
        .map(Value::String)
        .unwrap_or(Value::Null))
}

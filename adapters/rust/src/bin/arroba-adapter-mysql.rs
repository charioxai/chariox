use arroba_adapters::protocol::{
    ConnectorAdapterPrepareResult, ConnectorAdapterRequest, UserCredentialInjectionConfig,
};
use mysql::prelude::Queryable;
use mysql::{Params, Pool, Row, Value as MySqlValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use arroba_adapters::connector_adapter_util::{
    credential_host_target, enforce_allowed_host, render_json_template, render_template_string,
    run_adapter,
};

#[derive(Debug, Serialize, Deserialize)]
struct MysqlConfig {
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
        return Err("MySQL connector must define at least one operation".to_string());
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
            .ok_or_else(|| "MySQL prepare is missing operation config".to_string())?,
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
            .ok_or_else(|| "MySQL call is missing operation config".to_string())?,
    )?;
    validate_config(&config)?;
    let arguments = request.arguments.unwrap_or_else(|| serde_json::json!({}));
    let config = prepare_config(&config, &arguments)?;
    let mut connection_url = config.connection_url;
    if let Some(credential) = request.credential {
        connection_url = inject_connection_credential(&connection_url, credential)?;
    }
    let query = config.query;
    let params = config
        .params
        .into_iter()
        .map(mysql_param)
        .collect::<Vec<_>>();
    let opts = mysql::Opts::from_url(&connection_url)
        .map_err(|error| format!("invalid MySQL connection_url: {error}"))?;
    let pool = Pool::new(opts).map_err(|error| format!("connect failed: {error}"))?;
    let mut conn = pool
        .get_conn()
        .map_err(|error| format!("connect failed: {error}"))?;
    let rows: Vec<Row> = conn
        .exec(query, Params::Positional(params))
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

fn prepare_config(config: &MysqlConfig, arguments: &Value) -> Result<MysqlConfig, String> {
    Ok(MysqlConfig {
        connection_url: render_template_string(&config.connection_url, arguments)?,
        query: render_template_string(&config.query, arguments)?,
        params: config
            .params
            .iter()
            .map(|value| render_json_template(value, arguments))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn parse_config(value: Value) -> Result<MysqlConfig, String> {
    serde_json::from_value::<MysqlConfig>(value)
        .map_err(|error| format!("invalid MySQL config: {error}"))
}

fn validate_config(config: &MysqlConfig) -> Result<(), String> {
    let url = url::Url::parse(&config.connection_url)
        .map_err(|error| format!("invalid connection_url: {error}"))?;
    if url.scheme() != "mysql" {
        return Err(format!("unsupported MySQL URL scheme `{}`", url.scheme()));
    }
    if config.query.trim().is_empty() {
        return Err("MySQL query must not be empty".to_string());
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
                .map_err(|_| "failed to set MySQL username".to_string())?;
            url.set_password(Some(&credential.secret))
                .map_err(|_| "failed to set MySQL password".to_string())?;
        }
        UserCredentialInjectionConfig::Query { name } => {
            url.query_pairs_mut().append_pair(&name, &credential.secret);
        }
        UserCredentialInjectionConfig::Header { .. } => {
            return Err("MySQL adapter cannot use header credentials".to_string())
        }
        UserCredentialInjectionConfig::Hmac { .. } => {
            return Err("MySQL adapter does not support hmac credentials yet".to_string())
        }
        UserCredentialInjectionConfig::Pty => {
            return Err("MySQL adapter cannot use terminal credentials".to_string())
        }
    }
    Ok(url.to_string())
}

fn mysql_param(value: Value) -> MySqlValue {
    match value {
        Value::Null => MySqlValue::NULL,
        Value::Bool(value) => MySqlValue::Int(if value { 1 } else { 0 }),
        Value::Number(value) if value.is_i64() => MySqlValue::Int(value.as_i64().unwrap()),
        Value::Number(value) if value.is_u64() => MySqlValue::UInt(value.as_u64().unwrap()),
        Value::Number(value) if value.is_f64() => MySqlValue::Double(value.as_f64().unwrap()),
        Value::String(value) => MySqlValue::Bytes(value.into_bytes()),
        other => MySqlValue::Bytes(other.to_string().into_bytes()),
    }
}

fn row_to_json(row: &Row) -> Result<Value, String> {
    let mut object = serde_json::Map::new();
    for (index, column) in row.columns_ref().iter().enumerate() {
        let value = row
            .as_ref(index)
            .ok_or_else(|| format!("missing MySQL column value at index {index}"))?;
        object.insert(column.name_str().to_string(), mysql_value_to_json(value));
    }
    Ok(Value::Object(object))
}

fn mysql_value_to_json(value: &MySqlValue) -> Value {
    match value {
        MySqlValue::NULL => Value::Null,
        MySqlValue::Bytes(bytes) => String::from_utf8(bytes.clone())
            .map(Value::String)
            .unwrap_or_else(|_| {
                use base64::Engine;
                serde_json::json!(base64::engine::general_purpose::STANDARD.encode(bytes))
            }),
        MySqlValue::Int(value) => serde_json::json!(value),
        MySqlValue::UInt(value) => serde_json::json!(value),
        MySqlValue::Float(value) => serde_json::json!(value),
        MySqlValue::Double(value) => serde_json::json!(value),
        MySqlValue::Date(year, month, day, hour, minute, second, micros) => Value::String(format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{micros:06}"
        )),
        MySqlValue::Time(is_negative, days, hours, minutes, seconds, micros) => {
            Value::String(format!(
                "{}{} {hours:02}:{minutes:02}:{seconds:02}.{micros:06}",
                if *is_negative { "-" } else { "" },
                days
            ))
        }
    }
}

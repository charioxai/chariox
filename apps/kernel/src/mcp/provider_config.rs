use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;

use super::{home_dir, CharioxMcpServerConfig, CharioxMcpTransportConfig};

pub(super) fn codex_mcp_to_chariox(
    name: &str,
    value: &toml::Value,
) -> Result<CharioxMcpServerConfig, String> {
    let table = value
        .as_table()
        .ok_or_else(|| "MCP entry must be a TOML table".to_string())?;
    let unsupported = unsupported_codex_mcp_fields(table);
    if !unsupported.is_empty() {
        return Err(format!(
            "unsupported Codex MCP fields: {}",
            unsupported.join(", ")
        ));
    }
    if table.contains_key("bearer_token") {
        return Err("inline bearer_token is not imported; use bearer_token_env_var".to_string());
    }

    let mut config = if let Some(command) = table.get("command") {
        if table.contains_key("url") {
            return Err("entry mixes stdio command and HTTP url transports".to_string());
        }
        for field in ["bearer_token_env_var", "http_headers", "env_http_headers"] {
            if table.contains_key(field) {
                return Err(format!("{field} is not supported for stdio MCPs"));
            }
        }
        let command = required_string(command, "command")?;
        let args = optional_string_array(table.get("args"), "args")?.unwrap_or_default();
        let mut config = CharioxMcpServerConfig::stdio(name, command, args);
        if let CharioxMcpTransportConfig::Stdio {
            env, env_vars, cwd, ..
        } = &mut config.transport
        {
            *env = optional_string_map(table.get("env"), "env")?.unwrap_or_default();
            *env_vars =
                optional_string_array(table.get("env_vars"), "env_vars")?.unwrap_or_default();
            *cwd = table
                .get("cwd")
                .map(|value| required_string(value, "cwd").map(PathBuf::from))
                .transpose()?;
        }
        config
    } else if let Some(url) = table.get("url") {
        for field in ["args", "env", "env_vars", "cwd"] {
            if table.contains_key(field) {
                return Err(format!("{field} is not supported for HTTP MCPs"));
            }
        }
        let url = required_string(url, "url")?;
        let mut config = CharioxMcpServerConfig::streamable_http(name, url);
        if let CharioxMcpTransportConfig::StreamableHttp {
            bearer_token_env_var,
            http_headers,
            env_http_headers,
            ..
        } = &mut config.transport
        {
            *bearer_token_env_var = table
                .get("bearer_token_env_var")
                .map(|value| required_string(value, "bearer_token_env_var"))
                .transpose()?;
            *http_headers =
                optional_string_map(table.get("http_headers"), "http_headers")?.unwrap_or_default();
            *env_http_headers =
                optional_string_map(table.get("env_http_headers"), "env_http_headers")?
                    .unwrap_or_default();
        }
        config
    } else {
        return Err("missing command or url transport".to_string());
    };

    config.enabled = optional_bool(table.get("enabled"), "enabled")?.unwrap_or(true);
    config.required = optional_bool(table.get("required"), "required")?.unwrap_or(false);
    config.startup_timeout_sec = optional_timeout_secs(table, "startup_timeout_sec")?
        .or(optional_legacy_timeout_ms(table, "startup_timeout_ms")?);
    config.tool_timeout_sec = optional_timeout_secs(table, "tool_timeout_sec")?;
    config.enabled_tools = optional_string_array(table.get("enabled_tools"), "enabled_tools")?;
    config.disabled_tools = optional_string_array(table.get("disabled_tools"), "disabled_tools")?;
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn unsupported_codex_mcp_fields(table: &toml::Table) -> Vec<String> {
    let supported = [
        "command",
        "args",
        "env",
        "env_vars",
        "cwd",
        "url",
        "bearer_token",
        "bearer_token_env_var",
        "http_headers",
        "env_http_headers",
        "startup_timeout_sec",
        "startup_timeout_ms",
        "tool_timeout_sec",
        "enabled",
        "required",
        "enabled_tools",
        "disabled_tools",
    ];
    table
        .keys()
        .filter(|field| !supported.contains(&field.as_str()))
        .cloned()
        .collect()
}

fn required_string(value: &toml::Value, field: &str) -> Result<String, String> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{field} must be a string"))
}

fn optional_bool(value: Option<&toml::Value>, field: &str) -> Result<Option<bool>, String> {
    value
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("{field} must be a boolean"))
        })
        .transpose()
}

fn optional_string_array(
    value: Option<&toml::Value>,
    field: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{field} must be an array of strings"))?;
    array
        .iter()
        .map(|value| required_string(value, field))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn optional_string_map(
    value: Option<&toml::Value>,
    field: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let table = value
        .as_table()
        .ok_or_else(|| format!("{field} must be a table of strings"))?;
    table
        .iter()
        .map(|(key, value)| required_string(value, field).map(|value| (key.clone(), value)))
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map(Some)
}

fn optional_timeout_secs(table: &toml::Table, field: &str) -> Result<Option<u64>, String> {
    let Some(value) = table.get(field) else {
        return Ok(None);
    };
    let secs = value
        .as_float()
        .or_else(|| value.as_integer().map(|integer| integer as f64))
        .ok_or_else(|| format!("{field} must be a number"))?;
    finite_timeout_secs(field, secs)
}

fn optional_legacy_timeout_ms(table: &toml::Table, field: &str) -> Result<Option<u64>, String> {
    let Some(value) = table.get(field) else {
        return Ok(None);
    };
    let millis = value
        .as_integer()
        .ok_or_else(|| format!("{field} must be an integer"))?;
    if millis < 0 {
        return Err(format!("{field} must be non-negative"));
    }
    finite_timeout_secs(field, millis as f64 / 1000.0)
}

fn finite_timeout_secs(field: &str, secs: f64) -> Result<Option<u64>, String> {
    if !secs.is_finite() || secs < 0.0 {
        return Err(format!("{field} must be a finite non-negative number"));
    }
    Ok(Some(secs.ceil() as u64))
}

pub(super) fn codex_home_dir() -> Result<PathBuf, DaemonError> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home));
    }
    home_dir()
        .map(|home| home.join(".codex"))
        .ok_or(DaemonError::InvalidConfig {
            field: "CODEX_HOME",
            message: "CODEX_HOME or HOME must be set to import Codex MCPs",
        })
}

pub(super) fn opencode_config_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(custom) = std::env::var_os("OPENCODE_CONFIG") {
        paths.push(PathBuf::from(custom));
    }
    if let Some(config_dir) = std::env::var_os("OPENCODE_CONFIG_DIR") {
        paths.extend(opencode_files_in_dir(Path::new(&config_dir)));
    }
    paths.extend([
        workspace.join("opencode.jsonc"),
        workspace.join("opencode.json"),
        workspace.join(".opencode").join("opencode.jsonc"),
        workspace.join(".opencode").join("opencode.json"),
    ]);
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        paths.extend(opencode_files_in_dir(
            &PathBuf::from(config_home).join("opencode"),
        ));
    } else if let Some(home) = home_dir() {
        paths.extend(opencode_files_in_dir(
            &home.join(".config").join("opencode"),
        ));
    }
    paths
}

pub(super) fn claude_mcp_config_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(custom) = std::env::var_os("CHARIOX_CLAUDE_CONFIG") {
        paths.push(PathBuf::from(custom));
    } else if let Some(home) = home_dir() {
        paths.push(home.join(".claude.json"));
    }
    paths.push(workspace.join(".mcp.json"));
    paths
}

fn opencode_files_in_dir(dir: &Path) -> Vec<PathBuf> {
    vec![
        dir.join("opencode.jsonc"),
        dir.join("opencode.json"),
        dir.join("config.json"),
    ]
}

pub(super) fn claude_mcp_server_sets<'a>(
    parsed: &'a serde_json::Value,
    config_path: &Path,
    workspace: &Path,
) -> Vec<(String, &'a serde_json::Map<String, serde_json::Value>)> {
    let mut sets = Vec::new();
    if let Some(servers) = parsed
        .get("mcpServers")
        .and_then(serde_json::Value::as_object)
    {
        sets.push((config_path.display().to_string(), servers));
    }
    if config_path.file_name().and_then(|name| name.to_str()) == Some(".claude.json") {
        let workspace_key = workspace.to_string_lossy();
        let canonical_workspace = workspace.canonicalize().ok();
        if let Some(projects) = parsed
            .get("projects")
            .and_then(serde_json::Value::as_object)
        {
            for (project_path, project) in projects {
                let direct_match = project_path == workspace_key.as_ref();
                let canonical_match = canonical_workspace
                    .as_ref()
                    .is_some_and(|canonical| Path::new(project_path) == canonical.as_path());
                if !direct_match && !canonical_match {
                    continue;
                }
                if let Some(servers) = project
                    .get("mcpServers")
                    .and_then(serde_json::Value::as_object)
                {
                    sets.push((
                        format!("{} projects[{project_path}]", config_path.display()),
                        servers,
                    ));
                }
            }
        }
    }
    sets
}

pub(super) fn opencode_mcp_to_chariox(
    name: &str,
    value: &serde_json::Value,
) -> Result<CharioxMcpServerConfig, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "MCP entry must be an object".to_string())?;
    let mcp_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing MCP type".to_string())?;
    match mcp_type {
        "local" => opencode_local_mcp_to_chariox(name, object),
        "remote" => opencode_remote_mcp_to_chariox(name, object),
        other => Err(format!("unsupported OpenCode MCP type `{other}`")),
    }
}

fn opencode_local_mcp_to_chariox(
    name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<CharioxMcpServerConfig, String> {
    let command_parts = object
        .get("command")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "local MCP command must be an array".to_string())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "local MCP command entries must be strings".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let Some((command, args)) = command_parts.split_first() else {
        return Err("local MCP command must not be empty".to_string());
    };
    let mut config = CharioxMcpServerConfig::stdio(name, command.clone(), args.to_vec());
    config.enabled = optional_json_bool(object.get("enabled"), "enabled")?.unwrap_or(true);
    config.tool_timeout_sec = optional_json_timeout_ms(object.get("timeout"), "timeout")?;
    if let CharioxMcpTransportConfig::Stdio { env, env_vars, .. } = &mut config.transport {
        let environment =
            optional_json_string_map(object.get("environment"), "environment")?.unwrap_or_default();
        for (key, value) in environment {
            if let Some(var_name) = env_reference(&value) {
                if var_name == key {
                    env_vars.push(key);
                } else {
                    return Err(format!(
                        "environment `{key}` references env var `{var_name}`, which cannot be represented in Chariox stdio env_vars"
                    ));
                }
            } else {
                env.insert(key, value);
            }
        }
    }
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn opencode_remote_mcp_to_chariox(
    name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<CharioxMcpServerConfig, String> {
    let url = object
        .get("url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "remote MCP url must be a string".to_string())?;
    if object
        .get("oauth")
        .is_some_and(|value| value != &serde_json::Value::Bool(false))
    {
        return Err("OpenCode OAuth MCP config is not imported yet".to_string());
    }
    let mut config = CharioxMcpServerConfig::streamable_http(name, url);
    config.enabled = optional_json_bool(object.get("enabled"), "enabled")?.unwrap_or(true);
    config.tool_timeout_sec = optional_json_timeout_ms(object.get("timeout"), "timeout")?;
    if let CharioxMcpTransportConfig::StreamableHttp {
        http_headers,
        env_http_headers,
        ..
    } = &mut config.transport
    {
        let headers =
            optional_json_string_map(object.get("headers"), "headers")?.unwrap_or_default();
        for (key, value) in headers {
            if let Some(var_name) = env_reference(&value) {
                env_http_headers.insert(key, var_name);
            } else {
                http_headers.insert(key, value);
            }
        }
    }
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

pub(super) fn claude_mcp_to_chariox(
    name: &str,
    value: &serde_json::Value,
) -> Result<CharioxMcpServerConfig, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "MCP entry must be an object".to_string())?;
    if object.contains_key("oauth")
        || object.contains_key("oauthScopes")
        || object.contains_key("oauthResource")
    {
        return Err("Claude OAuth MCP config is not imported yet".to_string());
    }
    let mcp_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| {
            if object.contains_key("command") {
                "stdio"
            } else if object.contains_key("url") {
                "http"
            } else {
                ""
            }
        });
    match mcp_type {
        "stdio" => claude_stdio_mcp_to_chariox(name, object),
        "http" | "streamable_http" => claude_http_mcp_to_chariox(name, object),
        "sse" => Err("Claude SSE MCP config is not imported yet".to_string()),
        "" => Err("missing Claude MCP type, command, or url".to_string()),
        other => Err(format!("unsupported Claude MCP type `{other}`")),
    }
}

fn claude_stdio_mcp_to_chariox(
    name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<CharioxMcpServerConfig, String> {
    if object.contains_key("url") {
        return Err("stdio MCP entry also contains url".to_string());
    }
    let command = object
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "stdio MCP command must be a string".to_string())?;
    let args = optional_json_string_array(object.get("args"), "args")?.unwrap_or_default();
    let mut config = CharioxMcpServerConfig::stdio(name, command, args);
    config.enabled = optional_json_bool(object.get("enabled"), "enabled")?.unwrap_or(true);
    config.required = optional_json_bool(object.get("required"), "required")?.unwrap_or(false);
    config.startup_timeout_sec =
        optional_json_timeout_secs(object.get("startup_timeout_sec"), "startup_timeout_sec")?;
    config.tool_timeout_sec =
        optional_json_timeout_secs(object.get("tool_timeout_sec"), "tool_timeout_sec")?;
    if let CharioxMcpTransportConfig::Stdio {
        env, env_vars, cwd, ..
    } = &mut config.transport
    {
        let environment = optional_json_string_map(object.get("env"), "env")?.unwrap_or_default();
        for (key, value) in environment {
            if let Some(var_name) = env_reference(&value) {
                if var_name == key {
                    env_vars.push(key);
                } else {
                    return Err(format!(
                        "env `{key}` references env var `{var_name}`, which cannot be represented in Chariox stdio env_vars"
                    ));
                }
            } else {
                env.insert(key, value);
            }
        }
        *cwd = object
            .get("cwd")
            .map(|value| {
                value
                    .as_str()
                    .map(PathBuf::from)
                    .ok_or_else(|| "cwd must be a string".to_string())
            })
            .transpose()?;
    }
    config.enabled_tools = optional_json_string_array(object.get("enabledTools"), "enabledTools")?;
    config.disabled_tools =
        optional_json_string_array(object.get("disabledTools"), "disabledTools")?;
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn claude_http_mcp_to_chariox(
    name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<CharioxMcpServerConfig, String> {
    if object.contains_key("command") {
        return Err("HTTP MCP entry also contains command".to_string());
    }
    let url = object
        .get("url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "HTTP MCP url must be a string".to_string())?;
    let mut config = CharioxMcpServerConfig::streamable_http(name, url);
    config.enabled = optional_json_bool(object.get("enabled"), "enabled")?.unwrap_or(true);
    config.required = optional_json_bool(object.get("required"), "required")?.unwrap_or(false);
    config.startup_timeout_sec =
        optional_json_timeout_secs(object.get("startup_timeout_sec"), "startup_timeout_sec")?;
    config.tool_timeout_sec =
        optional_json_timeout_secs(object.get("tool_timeout_sec"), "tool_timeout_sec")?;
    if let CharioxMcpTransportConfig::StreamableHttp {
        http_headers,
        env_http_headers,
        ..
    } = &mut config.transport
    {
        let headers =
            optional_json_string_map(object.get("headers"), "headers")?.unwrap_or_default();
        for (key, value) in headers {
            if let Some(var_name) = env_reference(&value) {
                env_http_headers.insert(key, var_name);
            } else if key.eq_ignore_ascii_case("authorization") {
                return Err(
                    "static Authorization headers are not imported; use an environment reference"
                        .to_string(),
                );
            } else {
                http_headers.insert(key, value);
            }
        }
    }
    config.enabled_tools = optional_json_string_array(object.get("enabledTools"), "enabledTools")?;
    config.disabled_tools =
        optional_json_string_array(object.get("disabledTools"), "disabledTools")?;
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn optional_json_bool(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<bool>, String> {
    value
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("{field} must be a boolean"))
        })
        .transpose()
}

fn optional_json_timeout_ms(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let millis = value
        .as_u64()
        .ok_or_else(|| format!("{field} must be a positive integer number of milliseconds"))?;
    if millis == 0 {
        return Err(format!("{field} must be positive"));
    }
    Ok(Some((millis + 999) / 1000))
}

fn optional_json_timeout_secs(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let secs = value
        .as_f64()
        .ok_or_else(|| format!("{field} must be a positive number of seconds"))?;
    if !secs.is_finite() || secs <= 0.0 {
        return Err(format!("{field} must be positive"));
    }
    Ok(Some(secs.ceil() as u64))
}

fn optional_json_string_array(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{field} must be an array of strings"))?;
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{field} entries must be strings"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn optional_json_string_map(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| format!("{field} must be an object of strings"))?;
    object
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_string()))
                .ok_or_else(|| format!("{field}.{key} must be a string"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map(Some)
}

fn env_reference(value: &str) -> Option<String> {
    value
        .strip_prefix("{env:")
        .and_then(|rest| rest.strip_suffix('}'))
        .filter(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .map(str::to_string)
}

pub(super) fn strip_jsonc_comments(input: &str) -> Result<String, String> {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }

        if ch == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut closed = false;
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                        }
                        if previous == '*' && next == '/' {
                            closed = true;
                            break;
                        }
                        previous = next;
                    }
                    if !closed {
                        return Err("unterminated block comment".to_string());
                    }
                    continue;
                }
                _ => {}
            }
        }

        output.push(ch);
    }

    Ok(output)
}

pub(super) fn remove_json_trailing_commas(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }

        if ch == ',' {
            let mut lookahead = chars.clone();
            while matches!(lookahead.peek(), Some(next) if next.is_whitespace()) {
                lookahead.next();
            }
            if matches!(lookahead.peek(), Some('}' | ']')) {
                continue;
            }
        }

        output.push(ch);
    }

    output
}

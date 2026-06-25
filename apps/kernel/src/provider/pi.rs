use std::collections::BTreeMap;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, OpenCodeProviderCatalog, OpenCodeProviderInfo,
    OpenCodeProviderModel, ProviderAuthStatus, ProviderLaunchResult,
};

const PI_ENV_OVERRIDE: &str = "ARROBA_PI_BIN";
const PI_PROVIDER_ID: &str = "pi";
const PI_MCP_SERVERS_ENV: &str = "ARROBA_PI_MCP_SERVERS";
const PI_RUNTIME_MCP_EXTENSION_FILE: &str = "arroba-pi-runtime-mcp-extension-v1.ts";

const PI_KNOWN_MODELS: &[(&str, &str)] = &[
    ("openai-codex/gpt-5.4", "ChatGPT GPT-5.4 via Pi"),
    ("openai-codex/gpt-5.4-mini", "ChatGPT GPT-5.4 Mini via Pi"),
    ("openai/gpt-5.4", "OpenAI GPT-5.4 via Pi"),
    ("openai/gpt-5.4-mini", "OpenAI GPT-5.4 Mini via Pi"),
    ("anthropic/claude-sonnet-4-6", "Claude Sonnet 4.6 via Pi"),
    ("github-copilot/gpt-5.4", "Copilot GPT-5.4 via Pi"),
];

pub fn resolve_pi_executable() -> Result<PathBuf, DaemonError> {
    let _guard = crate::env_lock::lock();
    resolve_pi_executable_unlocked()
}

fn resolve_pi_executable_unlocked() -> Result<PathBuf, DaemonError> {
    if let Some(path) = env::var_os(PI_ENV_OVERRIDE).map(PathBuf::from) {
        return resolve_candidate(path, true).ok_or_else(|| {
            DaemonError::ProviderExecutableNotFound {
                adapter_key: PI_PROVIDER_ID.to_string(),
                executable: env::var(PI_ENV_OVERRIDE)
                    .unwrap_or_else(|_| PI_PROVIDER_ID.to_string()),
            }
        });
    }

    resolve_candidate(PathBuf::from(PI_PROVIDER_ID), false).ok_or_else(|| {
        DaemonError::ProviderExecutableNotFound {
            adapter_key: PI_PROVIDER_ID.to_string(),
            executable: PI_PROVIDER_ID.to_string(),
        }
    })
}

pub fn plan_pi_launch(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    let _guard = crate::env_lock::lock();
    plan_pi_launch_unlocked(request)
}

fn plan_pi_launch_unlocked(
    request: Option<&LaunchProviderRequest>,
) -> Result<ProviderLaunchResult, DaemonError> {
    let executable = resolve_pi_executable_unlocked()?;
    let request = request.ok_or_else(|| DaemonError::LocalTransport {
        operation: "plan_pi_launch",
        message: "Pi provider launch requires a provider run request".to_string(),
    })?;
    let mut args = vec!["--mode".to_string(), "rpc".to_string()];
    let pi_model = pi_provider_model(&request.model);
    if let Some(backing_provider) = pi_model_backing_provider(pi_model) {
        ensure_pi_backing_provider_auth(backing_provider)?;
        args.extend(["--provider".to_string(), backing_provider.to_string()]);
    }
    if !pi_model.trim().is_empty() && pi_model != "default" {
        args.extend(["--model".to_string(), pi_model.to_string()]);
    }
    if let Some(variant) = request
        .variant
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.extend(["--thinking".to_string(), variant.to_string()]);
    }
    if let Some(session_id) = request
        .resume_state
        .as_ref()
        .and_then(|state| state.pi_session_id())
    {
        args.extend(["--session-id".to_string(), session_id.to_string()]);
    }
    args.extend([
        "--name".to_string(),
        format!(
            "arroba-{}-{}",
            request.session_id,
            request.agent_id.as_deref().unwrap_or("agent")
        ),
    ]);
    let mut launch_env = BTreeMap::new();
    if let Some(mcp_servers_json) = pi_runtime_mcp_servers_json(request)? {
        let extension_path = ensure_pi_runtime_mcp_extension()?;
        args.extend([
            "--extension".to_string(),
            extension_path.display().to_string(),
        ]);
        launch_env.insert(PI_MCP_SERVERS_ENV.to_string(), mcp_servers_json);
    }

    Ok(ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::Managed,
        process_label: "pi:rpc".to_string(),
        pty_target: None,
        pty_program: Some(executable.display().to_string()),
        pty_args: args,
        pty_env: launch_env,
        pty_env_remove: request.provider_env_remove.clone(),
        working_directory: request.working_directory.clone(),
        structured_endpoint: None,
    })
}

pub fn pi_provider_catalog() -> OpenCodeProviderCatalog {
    let mut models = BTreeMap::new();
    for (id, name) in PI_KNOWN_MODELS {
        models.insert((*id).to_string(), pi_model(id, name));
    }
    for (id, name) in pi_config_model_entries() {
        models
            .entry(id.clone())
            .or_insert_with(|| pi_model(&id, &name));
    }
    OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: PI_PROVIDER_ID.to_string(),
            name: "Pi".to_string(),
            remote_machine_aliases: Vec::new(),
            models,
        }],
        default: BTreeMap::from([(
            PI_PROVIDER_ID.to_string(),
            "openai-codex/gpt-5.4".to_string(),
        )]),
        connected: if resolve_pi_executable().is_ok() {
            vec![PI_PROVIDER_ID.to_string()]
        } else {
            Vec::new()
        },
    }
}

fn pi_model(id: &str, name: &str) -> OpenCodeProviderModel {
    OpenCodeProviderModel {
        id: id.to_string(),
        name: name.to_string(),
        status: "available".to_string(),
        limit: None,
        variants: BTreeMap::from([
            ("low".to_string(), serde_json::json!({})),
            ("medium".to_string(), serde_json::json!({})),
            ("high".to_string(), serde_json::json!({})),
            ("xhigh".to_string(), serde_json::json!({})),
        ]),
    }
}

pub fn pi_provider_auth_status(model: Option<&str>) -> Result<ProviderAuthStatus, DaemonError> {
    let detected_version = pi_version().ok();
    let backing_provider = model.and_then(parse_pi_backing_provider_hint);
    let auth = pi_backing_provider_auth_state(backing_provider);
    Ok(ProviderAuthStatus {
        provider: PI_PROVIDER_ID.to_string(),
        auth_state: if resolve_pi_executable().is_err() {
            "provider_cli_missing".to_string()
        } else if auth.configured {
            "authenticated".to_string()
        } else {
            "not_logged_in".to_string()
        },
        account_profile: auth.account_profile,
        login_hint: Some(
            "Run `pi`, use `/login`, and authenticate the selected Pi backing provider; or configure ~/.pi/agent/auth.json/API-key environment variables.".to_string(),
        ),
        detected_version,
    })
}

#[derive(Debug, Default)]
struct PiBackingAuth {
    configured: bool,
    account_profile: Option<String>,
}

fn pi_backing_provider_auth_state(backing_provider: Option<&str>) -> PiBackingAuth {
    let auth_json = pi_auth_json();
    let Some(provider) = backing_provider else {
        return PiBackingAuth::default();
    };
    if pi_auth_json_has_provider(auth_json.as_ref(), provider) || pi_env_has_provider_key(provider)
    {
        return PiBackingAuth {
            configured: true,
            account_profile: Some(provider.to_string()),
        };
    }
    PiBackingAuth::default()
}

fn ensure_pi_backing_provider_auth(backing_provider: &str) -> Result<(), DaemonError> {
    if pi_backing_provider_auth_state(Some(backing_provider)).configured {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "plan_pi_launch",
        message: format!(
            "Pi backing provider `{backing_provider}` is not configured; run `pi`, use `/login`, authenticate that backing provider, or set the matching API key before launching provider `pi` with this model"
        ),
    })
}

fn pi_auth_json() -> Option<Value> {
    if let Some(path) = env::var_os("PI_AUTH_FILE").map(PathBuf::from) {
        return read_pi_auth_json_file(&path);
    }
    let home = env::var_os("HOME").map(PathBuf::from)?;
    [home.join(".pi/agent/auth.json"), home.join(".pi/auth.json")]
        .into_iter()
        .find_map(|path| read_pi_auth_json_file(&path))
}

fn read_pi_auth_json_file(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn pi_auth_json_has_provider(value: Option<&Value>, provider: &str) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    let keys = pi_provider_auth_keys(provider);
    keys.iter().any(|key| {
        object.get(key.as_str()).is_some_and(|value| {
            !value.is_null()
                && match value.as_object() {
                    Some(object) => !object.is_empty(),
                    None => true,
                }
        })
    })
}

pub(crate) fn parse_pi_provider_request(provider: &str) -> Option<String> {
    provider
        .strip_prefix("pi/")
        .filter(|value| !value.is_empty())
        .and_then(|rest| {
            let model = pi_provider_model(rest);
            if model.contains('/') {
                pi_model_backing_provider(model).map(str::to_string)
            } else if !model.is_empty() {
                Some(model.to_string())
            } else {
                None
            }
        })
}

fn pi_env_has_provider_key(provider: &str) -> bool {
    pi_provider_env_keys(provider)
        .iter()
        .any(|name| env::var_os(name).is_some())
}

fn pi_provider_auth_keys(provider: &str) -> Vec<String> {
    match provider {
        "anthropic" | "claude" => vec!["anthropic".to_string(), "claude".to_string()],
        "openai" => vec!["openai".to_string()],
        "codex" | "openai-codex" => vec!["codex".to_string(), "openai-codex".to_string()],
        "github-copilot" | "copilot" | "autopilot" => {
            vec!["github-copilot".to_string(), "copilot".to_string()]
        }
        other => vec![other.to_string()],
    }
}

fn pi_provider_env_keys(provider: &str) -> Vec<&'static str> {
    match provider {
        "anthropic" | "claude" => vec!["ANTHROPIC_API_KEY"],
        "openai" => vec!["OPENAI_API_KEY"],
        "github-copilot" | "copilot" | "autopilot" => vec!["GITHUB_TOKEN", "COPILOT_TOKEN"],
        _ => Vec::new(),
    }
}

pub(crate) fn pi_model_backing_provider(model: &str) -> Option<&str> {
    let provider = model.split_once('/')?.0.trim();
    (!provider.is_empty()).then_some(provider)
}

fn parse_pi_backing_provider_hint(model: &str) -> Option<&str> {
    let model = pi_provider_model(model);
    model.split_once('/').map(|(provider, _)| provider).or_else(|| {
        match model {
            "openai" | "openai-codex" | "anthropic" | "claude" | "copilot"
            | "github-copilot" | "autopilot" => Some(model),
            _ => None,
        }
    })
}

fn pi_provider_model(model: &str) -> &str {
    model.strip_prefix("pi/").unwrap_or(model)
}

fn pi_config_model_entries() -> Vec<(String, String)> {
    let path = env::var_os("PI_MODELS_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME").map(|home| PathBuf::from(home).join(".pi/agent/models.json"))
        });
    let Some(path) = path else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    if let Some(providers) = value.get("providers").and_then(Value::as_object) {
        for (provider_id, provider) in providers {
            if let Some(models) = provider.get("models").and_then(Value::as_array) {
                for model in models {
                    let Some(id) = model.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let id = if id.contains('/') {
                        id.to_string()
                    } else {
                        format!("{provider_id}/{id}")
                    };
                    let name = model
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    entries.push((id, name));
                }
            }
        }
    }
    entries
}

fn pi_runtime_mcp_servers_json(
    request: &LaunchProviderRequest,
) -> Result<Option<String>, DaemonError> {
    let Some(binding) = request.runtime_mcp_binding.as_ref() else {
        return Ok(None);
    };
    let mut servers = Vec::new();
    servers.push(serde_json::json!({
        "name": "arroba",
        "toolPrefix": "",
        "url": binding.server_url,
        "headers": {
            "Authorization": format!("Bearer {}", binding.auth_token),
        },
    }));
    let provider_mcp_servers = super::mcp_proxy::provider_facing_mcp_proxy_configs(
        &request.mcp_servers,
        Some(binding.server_url.as_str()),
        Some(binding.auth_token.as_str()),
    )?;
    for server in provider_mcp_servers {
        if let Some(config) = pi_mcp_server_extension_config(&server) {
            servers.push(config);
        }
    }
    serde_json::to_string(&servers)
        .map(Some)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "pi_mcp_extension_config",
            message: error.to_string(),
        })
}

fn pi_mcp_server_extension_config(server: &ArrobaMcpServerConfig) -> Option<Value> {
    let ArrobaMcpTransportConfig::StreamableHttp {
        url,
        bearer_token_env_var: _,
        bearer_token_credential: _,
        http_headers,
        credential_http_headers: _,
        env_http_headers,
    } = &server.transport
    else {
        return None;
    };
    let mut headers = http_headers.clone();
    for (header, env_var) in env_http_headers {
        if let Ok(value) = env::var(env_var) {
            headers.insert(header.clone(), value);
        }
    }
    Some(serde_json::json!({
        "name": server.name,
        "toolPrefix": format!("arroba_mcp_{}_", server.name),
        "url": url,
        "headers": headers,
    }))
}

fn ensure_pi_runtime_mcp_extension() -> Result<PathBuf, DaemonError> {
    let path = env::temp_dir().join(PI_RUNTIME_MCP_EXTENSION_FILE);
    let desired = pi_runtime_mcp_extension_source();
    if fs::read_to_string(&path).ok().as_deref() == Some(desired) {
        return Ok(path);
    }
    fs::write(&path, desired).map_err(|error| DaemonError::LocalTransport {
        operation: "pi_mcp_extension_write",
        message: format!("failed to write Pi runtime MCP extension: {error}"),
    })?;
    Ok(path)
}

fn pi_runtime_mcp_extension_source() -> &'static str {
    r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

type McpServer = {
  name: string;
  toolPrefix?: string;
  url: string;
  headers?: Record<string, string>;
};

type McpTool = {
  name: string;
  description?: string;
  inputSchema?: unknown;
};

let nextId = 1;

async function callMcp(server: McpServer, method: string, params?: unknown): Promise<any> {
  const response = await fetch(server.url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(server.headers ?? {}),
    },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: nextId++,
      method,
      params: params ?? {},
    }),
  });
  if (!response.ok) {
    throw new Error(`MCP ${server.name} ${method} failed with HTTP ${response.status}: ${await response.text()}`);
  }
  const payload = await response.json();
  if (payload.error) {
    throw new Error(payload.error.message ?? JSON.stringify(payload.error));
  }
  return payload.result;
}

function toolName(server: McpServer, tool: McpTool): string {
  return `${server.toolPrefix ?? ""}${tool.name}`;
}

function toolSchema(tool: McpTool): any {
  return tool.inputSchema ?? { type: "object", additionalProperties: true };
}

function resultText(result: any): string {
  if (Array.isArray(result?.content)) {
    const text = result.content
      .filter((item: any) => item?.type === "text" && typeof item.text === "string")
      .map((item: any) => item.text)
      .join("\n");
    if (text) return text;
  }
  if (result?.structuredContent !== undefined) {
    return JSON.stringify(result.structuredContent);
  }
  return JSON.stringify(result);
}

export default async function(pi: ExtensionAPI) {
  const raw = process.env.ARROBA_PI_MCP_SERVERS;
  if (!raw) return;
  const servers = JSON.parse(raw) as McpServer[];
  for (const server of servers) {
    try {
      await callMcp(server, "initialize", {
        protocolVersion: "2025-03-26",
        capabilities: {},
        clientInfo: {
          name: "arroba-pi-runtime-mcp",
          version: "v1",
        },
      });
    } catch {
      // Some MCP endpoints allow tools/list without an initialize round trip.
    }
    const listed = await callMcp(server, "tools/list");
    for (const tool of (listed.tools ?? []) as McpTool[]) {
      const exposedName = toolName(server, tool);
      pi.registerTool({
        name: exposedName,
        label: exposedName,
        description: server.toolPrefix
          ? `${tool.description ?? tool.name} (Arroba MCP ${server.name}:${tool.name})`
          : (tool.description ?? tool.name),
        parameters: toolSchema(tool),
        async execute(_toolCallId, params) {
          const result = await callMcp(server, "tools/call", {
            name: tool.name,
            arguments: params ?? {},
          });
          return {
            content: [{ type: "text", text: resultText(result) }],
            details: result?.structuredContent ?? result,
            isError: Boolean(result?.isError),
          };
        },
      });
    }
  }
}
"#
}

fn pi_version() -> Result<String, DaemonError> {
    let executable = resolve_pi_executable()?;
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "pi_version",
            message: error.to_string(),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Ok(stderr);
    }
    Err(DaemonError::LocalTransport {
        operation: "pi_version",
        message: "pi returned no version text".to_string(),
    })
}

fn resolve_candidate(candidate: PathBuf, treat_as_literal_path: bool) -> Option<PathBuf> {
    if treat_as_literal_path || candidate.components().count() > 1 {
        return is_executable_file(&candidate).then_some(candidate);
    }

    if candidate.is_absolute() && is_executable_file(&candidate) {
        return Some(candidate);
    }

    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|directory| directory.join(&candidate))
        .find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use crate::mcp::ArrobaMcpServerConfig;
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, RuntimeMcpBinding};

    use super::{
        parse_pi_provider_request, pi_model_backing_provider, plan_pi_launch,
        pi_provider_auth_status, resolve_pi_executable, PI_MCP_SERVERS_ENV,
    };

    fn write_executable_fixture(path: &std::path::Path) {
        fs::write(
            path,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo \"pi 0.0.0\"\n  exit 0\nfi\nsleep 60\n",
        )
        .expect("fixture should exist");
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(path).expect("fixture metadata").permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(path, permissions).expect("fixture permissions");
        }
    }

    #[test]
    fn resolves_pi_override_path_for_tests() {
        let _guard = crate::env_lock::lock();
        let path =
            std::env::temp_dir().join(format!("arroba-pi-resolve-test-{}", std::process::id()));
        write_executable_fixture(&path);
        std::env::set_var("ARROBA_PI_BIN", &path);

        let resolved = resolve_pi_executable().expect("override path should resolve");

        std::env::remove_var("ARROBA_PI_BIN");
        let _ = fs::remove_file(&path);
        assert_eq!(resolved, path);
    }

    #[test]
    fn rejects_non_executable_pi_override_path() {
        let _guard = crate::env_lock::lock();
        let path = std::env::temp_dir().join(format!(
            "arroba-pi-resolve-non-executable-test-{}",
            std::process::id()
        ));
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fixture should exist");
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(&path).expect("fixture metadata").permissions();
            permissions.set_mode(0o600);
            fs::set_permissions(&path, permissions).expect("fixture permissions");
        }
        std::env::set_var("ARROBA_PI_BIN", &path);

        let error = resolve_pi_executable().expect_err("non-executable path should not resolve");

        std::env::remove_var("ARROBA_PI_BIN");
        let _ = fs::remove_file(&path);
        assert!(error.to_string().contains("pi"));
    }

    #[test]
    fn plans_pi_rpc_launch_with_backing_provider_model() {
        let _guard = crate::env_lock::lock();
        let path =
            std::env::temp_dir().join(format!("arroba-pi-launch-test-{}", std::process::id()));
        let auth_path =
            std::env::temp_dir().join(format!("arroba-pi-launch-auth-test-{}", std::process::id()));
        write_executable_fixture(&path);
        fs::write(&auth_path, r#"{"anthropic":{"type":"oauth"}}"#)
            .expect("auth fixture should exist");
        std::env::set_var("ARROBA_PI_BIN", &path);
        std::env::set_var("PI_AUTH_FILE", &auth_path);
        let request = LaunchProviderRequest::new(
            "session-1",
            "pi",
            "pi",
            "default",
            "pi/anthropic/claude-sonnet-4-6",
        )
        .with_agent_id("agent-1");

        let launch = plan_pi_launch(Some(&request)).expect("launch should plan");

        std::env::remove_var("ARROBA_PI_BIN");
        std::env::remove_var("PI_AUTH_FILE");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&auth_path);
        assert_eq!(launch.endpoint_mode, AgentEndpointMode::Managed);
        assert_eq!(launch.pty_args[0], "--mode");
        assert!(launch.pty_args.contains(&"rpc".to_string()));
        assert!(launch.pty_args.contains(&"--provider".to_string()));
        assert!(launch.pty_args.contains(&"anthropic".to_string()));
        assert!(launch.pty_args.contains(&"--model".to_string()));
        assert!(launch
            .pty_args
            .contains(&"anthropic/claude-sonnet-4-6".to_string()));
    }

    #[test]
    fn plans_pi_rpc_launch_with_openai_codex_auth() {
        let _guard = crate::env_lock::lock();
        let path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-openai-codex-test-{}",
            std::process::id()
        ));
        let auth_path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-openai-codex-auth-test-{}",
            std::process::id()
        ));
        write_executable_fixture(&path);
        fs::write(&auth_path, r#"{"openai-codex":{"type":"oauth"}}"#)
            .expect("auth fixture should exist");
        std::env::set_var("ARROBA_PI_BIN", &path);
        std::env::set_var("PI_AUTH_FILE", &auth_path);
        let request = LaunchProviderRequest::new(
            "session-1",
            "pi",
            "pi",
            "default",
            "pi/openai-codex/gpt-5.4",
        );

        let launch = plan_pi_launch(Some(&request))
            .expect("openai-codex auth should satisfy openai-codex backing");

        std::env::remove_var("ARROBA_PI_BIN");
        std::env::remove_var("PI_AUTH_FILE");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&auth_path);
        assert!(launch.pty_args.contains(&"--provider".to_string()));
        assert!(launch.pty_args.contains(&"openai-codex".to_string()));
        assert!(launch
            .pty_args
            .contains(&"openai-codex/gpt-5.4".to_string()));
    }

    #[test]
    fn openai_codex_auth_does_not_satisfy_plain_openai_model() {
        let _guard = crate::env_lock::lock();
        let path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-openai-auth-mismatch-test-{}",
            std::process::id()
        ));
        let auth_path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-openai-auth-mismatch-auth-test-{}",
            std::process::id()
        ));
        write_executable_fixture(&path);
        fs::write(&auth_path, r#"{"openai-codex":{"type":"oauth"}}"#)
            .expect("auth fixture should exist");
        std::env::set_var("ARROBA_PI_BIN", &path);
        std::env::set_var("PI_AUTH_FILE", &auth_path);
        let request =
            LaunchProviderRequest::new("session-1", "pi", "pi", "default", "pi/openai/gpt-5.4");

        let error = plan_pi_launch(Some(&request))
            .expect_err("openai-codex auth should not satisfy openai backing");

        std::env::remove_var("ARROBA_PI_BIN");
        std::env::remove_var("PI_AUTH_FILE");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&auth_path);
        assert!(error.to_string().contains("Pi backing provider `openai`"));
    }

    #[test]
    fn plans_pi_rpc_launch_with_thinking_and_resume_session() {
        let _guard = crate::env_lock::lock();
        let path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-resume-test-{}",
            std::process::id()
        ));
        let auth_path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-resume-auth-test-{}",
            std::process::id()
        ));
        write_executable_fixture(&path);
        fs::write(&auth_path, r#"{"openai-codex":{"type":"oauth"}}"#)
            .expect("auth fixture should exist");
        std::env::set_var("ARROBA_PI_BIN", &path);
        std::env::set_var("PI_AUTH_FILE", &auth_path);
        let request = LaunchProviderRequest::new(
            "session-1",
            "pi",
            "pi",
            "default",
            "pi/openai-codex/gpt-5.4",
        )
        .with_variant(Some("high".to_string()))
        .with_resume_state(crate::provider::ProviderResumeState::from_pi_session_id(
            "pi-session-1".to_string(),
        ));

        let launch = plan_pi_launch(Some(&request)).expect("launch should plan");

        std::env::remove_var("ARROBA_PI_BIN");
        std::env::remove_var("PI_AUTH_FILE");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&auth_path);
        assert!(launch.pty_args.contains(&"--thinking".to_string()));
        assert!(launch.pty_args.contains(&"high".to_string()));
        assert!(launch.pty_args.contains(&"--session-id".to_string()));
        assert!(launch.pty_args.contains(&"pi-session-1".to_string()));
    }

    #[test]
    fn plans_pi_runtime_mcp_extension_when_runtime_binding_is_available() {
        let _guard = crate::env_lock::lock();
        let path =
            std::env::temp_dir().join(format!("arroba-pi-launch-mcp-test-{}", std::process::id()));
        let auth_path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-mcp-auth-test-{}",
            std::process::id()
        ));
        write_executable_fixture(&path);
        fs::write(&auth_path, r#"{"openai-codex":{"type":"oauth"}}"#)
            .expect("auth fixture should exist");
        std::env::set_var("ARROBA_PI_BIN", &path);
        std::env::set_var("PI_AUTH_FILE", &auth_path);
        let request = LaunchProviderRequest::new(
            "session-1",
            "pi",
            "pi",
            "default",
            "pi/openai-codex/gpt-5.4",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "token-123",
        ))
        .with_mcp_servers(vec![ArrobaMcpServerConfig::stdio(
            "filesystem",
            "fs-mcp",
            Vec::new(),
        )]);

        let launch = plan_pi_launch(Some(&request)).expect("launch should plan");

        std::env::remove_var("ARROBA_PI_BIN");
        std::env::remove_var("PI_AUTH_FILE");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&auth_path);
        assert!(launch.pty_args.contains(&"--extension".to_string()));
        let servers = launch
            .pty_env
            .get(PI_MCP_SERVERS_ENV)
            .expect("Pi MCP extension env should be set");
        let servers: serde_json::Value =
            serde_json::from_str(servers).expect("servers should be valid JSON");
        assert_eq!(
            servers
                .pointer("/0/name")
                .and_then(serde_json::Value::as_str),
            Some("arroba")
        );
        assert_eq!(
            servers
                .pointer("/0/headers/Authorization")
                .and_then(serde_json::Value::as_str),
            Some("Bearer token-123")
        );
        assert_eq!(
            servers
                .pointer("/1/name")
                .and_then(serde_json::Value::as_str),
            Some("filesystem")
        );
        assert_eq!(
            servers
                .pointer("/1/toolPrefix")
                .and_then(serde_json::Value::as_str),
            Some("arroba_mcp_filesystem_")
        );
    }

    #[test]
    fn parses_pi_model_backing_provider() {
        assert_eq!(pi_model_backing_provider("openai/gpt-5.4"), Some("openai"));
        assert_eq!(pi_model_backing_provider("gpt-5.4"), None);
    }

    #[test]
    fn reads_legacy_pi_auth_json_location() {
        let _guard = crate::env_lock::lock();
        let path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-legacy-auth-test-{}",
            std::process::id()
        ));
        let home = std::env::temp_dir().join(format!(
            "arroba-pi-launch-legacy-auth-home-test-{}",
            std::process::id()
        ));
        let legacy_auth_path = home.join(".pi/auth.json");
        write_executable_fixture(&path);
        fs::create_dir_all(legacy_auth_path.parent().expect("legacy auth parent"))
            .expect("legacy auth parent should exist");
        fs::write(&legacy_auth_path, r#"{"openai-codex":{"type":"oauth"}}"#)
            .expect("auth fixture should exist");
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("ARROBA_PI_BIN", &path);
        std::env::set_var("HOME", &home);
        std::env::remove_var("PI_AUTH_FILE");
        let request = LaunchProviderRequest::new(
            "session-1",
            "pi",
            "pi",
            "default",
            "pi/openai-codex/gpt-5.4",
        );

        plan_pi_launch(Some(&request)).expect("legacy Pi auth path should satisfy launch");

        std::env::remove_var("ARROBA_PI_BIN");
        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn pi_auth_status_requires_backing_provider_to_report_authenticated() {
        let _guard = crate::env_lock::lock();
        let path = std::env::temp_dir().join(format!(
            "arroba-pi-status-auth-legacy-openai-{}",
            std::process::id()
        ));
        let auth_path = std::env::temp_dir().join(format!(
            "arroba-pi-status-auth-file-{}",
            std::process::id()
        ));
        write_executable_fixture(&path);
        fs::write(&auth_path, r#"{"openai":{"type":"oauth","accountId":"acct"}}"#)
            .expect("auth fixture should exist");
        std::env::set_var("ARROBA_PI_BIN", &path);
        std::env::set_var("PI_AUTH_FILE", &auth_path);

        let openai_status = pi_provider_auth_status(Some("pi/openai/gpt-5.4"))
            .expect("openai-backed Pi model should be authenticated");
        let bare_status = pi_provider_auth_status(None)
            .expect("bare Pi auth status should be safe without a selected backing provider");
        let anthropic_status = pi_provider_auth_status(Some("pi/anthropic/claude-sonnet-4-6"))
            .expect("non-matching backing provider should be unauthenticated");

        std::env::remove_var("ARROBA_PI_BIN");
        std::env::remove_var("PI_AUTH_FILE");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&auth_path);

        assert_eq!(openai_status.auth_state, "authenticated");
        assert_eq!(openai_status.account_profile.as_deref(), Some("openai"));
        assert_eq!(bare_status.auth_state, "not_logged_in");
        assert_eq!(bare_status.account_profile, None);
        assert_eq!(anthropic_status.auth_state, "not_logged_in");
        assert_eq!(anthropic_status.account_profile, None);
    }

    #[test]
    fn parse_pi_provider_request_extracts_backing_provider() {
        assert_eq!(
            parse_pi_provider_request("pi/openai/gpt-5.4").as_deref(),
            Some("openai")
        );
        assert_eq!(
            parse_pi_provider_request("pi/anthropic/claude-sonnet-4-6").as_deref(),
            Some("anthropic")
        );
        assert_eq!(parse_pi_provider_request("pi/openai").as_deref(), Some("openai"));
        assert_eq!(parse_pi_provider_request("pi/"), None);
        assert_eq!(parse_pi_provider_request("pi"), None);
    }

    #[test]
    fn openai_api_key_does_not_satisfy_openai_codex_model() {
        let _guard = crate::env_lock::lock();
        let path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-openai-codex-env-mismatch-test-{}",
            std::process::id()
        ));
        let auth_path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-openai-codex-env-mismatch-auth-test-{}",
            std::process::id()
        ));
        write_executable_fixture(&path);
        fs::write(&auth_path, r#"{}"#).expect("auth fixture should exist");
        let previous_openai_api_key = std::env::var_os("OPENAI_API_KEY");
        std::env::set_var("ARROBA_PI_BIN", &path);
        std::env::set_var("PI_AUTH_FILE", &auth_path);
        std::env::set_var("OPENAI_API_KEY", "sk-test");
        let request = LaunchProviderRequest::new(
            "session-1",
            "pi",
            "pi",
            "default",
            "pi/openai-codex/gpt-5.4",
        );

        let error = plan_pi_launch(Some(&request))
            .expect_err("OpenAI API key should not satisfy openai-codex backing");

        std::env::remove_var("ARROBA_PI_BIN");
        std::env::remove_var("PI_AUTH_FILE");
        match previous_openai_api_key {
            Some(value) => std::env::set_var("OPENAI_API_KEY", value),
            None => std::env::remove_var("OPENAI_API_KEY"),
        }
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&auth_path);
        assert!(error
            .to_string()
            .contains("Pi backing provider `openai-codex`"));
    }

    #[test]
    fn pi_launch_fails_without_selected_backing_provider_auth() {
        let _guard = crate::env_lock::lock();
        let path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-missing-auth-test-{}",
            std::process::id()
        ));
        let auth_path = std::env::temp_dir().join(format!(
            "arroba-pi-launch-missing-auth-json-test-{}",
            std::process::id()
        ));
        write_executable_fixture(&path);
        fs::write(&auth_path, r#"{"anthropic":{"type":"oauth"}}"#)
            .expect("auth fixture should exist");
        std::env::set_var("ARROBA_PI_BIN", &path);
        std::env::set_var("PI_AUTH_FILE", &auth_path);
        let request =
            LaunchProviderRequest::new("session-1", "pi", "pi", "default", "pi/openai/gpt-5.4");

        let error = plan_pi_launch(Some(&request)).expect_err("missing openai auth should fail");

        std::env::remove_var("ARROBA_PI_BIN");
        std::env::remove_var("PI_AUTH_FILE");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&auth_path);
        assert!(error
            .to_string()
            .contains("Pi backing provider `openai` is not configured"));
    }
}

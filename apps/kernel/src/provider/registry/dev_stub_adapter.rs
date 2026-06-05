use std::collections::BTreeMap;

use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};

use super::AgentEndpointAdapter;

#[derive(Debug, Default)]
pub(super) struct DevStubAdapter;

impl DevStubAdapter {
    pub(super) const KEY: &'static str = "dev-stub";
}

pub(super) static DEV_STUB_ADAPTER: DevStubAdapter = DevStubAdapter;

impl AgentEndpointAdapter for DevStubAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        let pty_args = dev_stub_pty_args(request.model.as_str());
        let pty_env = dev_stub_pty_env(request);
        let pty_target = if is_dev_stub_workflow_drill_model(request.model.as_str()) {
            format!(
                "stub-pty:{}:{}",
                request.session_id,
                request
                    .agent_id
                    .as_deref()
                    .unwrap_or(request.model.as_str())
            )
        } else {
            format!("stub-pty:{}", request.session_id)
        };
        Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: format!(
                "dev-stub:{}:{}:{}",
                request.provider, request.account_profile, request.model
            ),
            pty_target: Some(pty_target),
            pty_program: Some("/bin/sh".to_string()),
            pty_args,
            pty_env,
            pty_env_remove: request.provider_env_remove.clone(),
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        })
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(super) struct ManagedDevStubAdapter;

#[cfg(test)]
impl ManagedDevStubAdapter {
    pub(super) const KEY: &'static str = "managed-dev-stub";
}

#[cfg(test)]
pub(super) static MANAGED_DEV_STUB_ADAPTER: ManagedDevStubAdapter = ManagedDevStubAdapter;

#[cfg(test)]
impl AgentEndpointAdapter for ManagedDevStubAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn supports_workspace_live_sync_write_enforcement(&self) -> bool {
        true
    }

    fn supports_turn_scoped_execution_config(&self) -> bool {
        true
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        let mut launch = DEV_STUB_ADAPTER.connect(request)?;
        launch.process_label = format!(
            "managed-dev-stub:{}:{}:{}",
            request.provider, request.account_profile, request.model
        );
        Ok(launch)
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

fn dev_stub_pty_args(model: &str) -> Vec<String> {
    let script = match model {
        "workflow-drill-node-1" => Some(dev_stub_workflow_output_script(
            "workflow drill node 1",
            1842,
        )),
        "workflow-drill-node-2" => Some(dev_stub_workflow_output_script(
            "workflow drill node 2",
            1843,
        )),
        "workflow-single-turn-node" => Some(dev_stub_workflow_read_through_script(
            "workflow single turn node",
            1842,
        )),
        "workflow-intermediate-node" => Some(dev_stub_workflow_intermediate_script(
            "workflow intermediate node",
            1841,
            1842,
        )),
        "semantic-url-renderer-stub" => Some(dev_stub_semantic_renderer_script()),
        "slow-first-output-drill" => Some(dev_stub_slow_first_output_script()),
        "large-output-drill" => Some(dev_stub_large_output_script()),
        _ => None,
    }
    .unwrap_or_else(|| "cat".to_string());
    vec!["-lc".to_string(), script]
}

fn is_dev_stub_workflow_drill_model(model: &str) -> bool {
    matches!(
        model,
        "workflow-drill-node-1"
            | "workflow-drill-node-2"
            | "workflow-single-turn-node"
            | "workflow-intermediate-node"
            | "semantic-url-renderer-stub"
            | "slow-first-output-drill"
            | "large-output-drill"
    )
}

fn dev_stub_pty_env(request: &LaunchProviderRequest) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if request.model == "workflow-intermediate-node" {
        if let Some(binding) = request.runtime_mcp_binding.as_ref() {
            env.insert(
                "ARROBA_DEV_STUB_RUNTIME_MCP_URL".to_string(),
                binding.server_url.clone(),
            );
            env.insert(
                "ARROBA_DEV_STUB_RUNTIME_MCP_TOKEN".to_string(),
                binding.auth_token.clone(),
            );
        }
    }
    env
}

fn dev_stub_workflow_output_script(summary: &str, value: i64) -> String {
    let payload =
        format!(r#"{{"summary":"{summary}","output":{{"message":{{"value":{value}}}}}}}"#);
    format!("sleep 1; printf '%s\\n%s\\n%s\\n' '```json' '{payload}' '```'; sleep 300")
}

fn dev_stub_workflow_read_through_script(summary: &str, value: i64) -> String {
    let payload =
        format!(r#"{{"summary":"{summary}","output":{{"message":{{"value":{value}}}}}}}"#);
    format!(
        "stty -echo 2>/dev/null || true; printed=0; while IFS= read -r _line; do if [ \"$printed\" -eq 0 ]; then printf '%s\\n%s\\n%s\\n' '```json' '{payload}' '```'; printed=1; fi; done"
    )
}

fn dev_stub_workflow_intermediate_script(
    summary: &str,
    intermediate: i64,
    final_value: i64,
) -> String {
    let payload =
        format!(r#"{{"summary":"{summary}","output":{{"message":{{"value":{final_value}}}}}}}"#);
    let intermediate_output = serde_json::json!({ "value": intermediate }).to_string();
    let js = format!(
        r#"const http = require("node:http");
const runtimeUrl = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_URL;
const runtimeToken = process.env.ARROBA_DEV_STUB_RUNTIME_MCP_TOKEN;
const finalPayload = {payload_json};
const intermediateOutput = {intermediate_json};
let printed = false;
let promptBuffer = "";
let fallbackTimer = null;

function callRuntimeTool(name, args) {{
  return new Promise((resolve, reject) => {{
    if (!runtimeUrl || !runtimeToken) return reject(new Error("runtime MCP binding missing"));
    const parsed = new URL(runtimeUrl);
    const body = JSON.stringify({{
      jsonrpc: "2.0",
      id: `dev-stub-${{Date.now()}}`,
      method: "tools/call",
      params: {{ name, arguments: args }},
    }});
    const req = http.request({{
      hostname: parsed.hostname,
      port: parsed.port,
      path: parsed.pathname || "/mcp",
      method: "POST",
      headers: {{
        authorization: `Bearer ${{runtimeToken}}`,
        "content-type": "application/json",
        "content-length": Buffer.byteLength(body),
      }},
    }}, (res) => {{
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => {{
        const text = Buffer.concat(chunks).toString("utf8");
        if (res.statusCode !== 200) return reject(new Error(`runtime MCP HTTP ${{res.statusCode}}: ${{text}}`));
        const response = JSON.parse(text);
        if (response.error) return reject(new Error(JSON.stringify(response.error)));
        if (response.result && response.result.isError) return reject(new Error(JSON.stringify(response.result)));
        resolve(response.result);
      }});
    }});
    req.on("error", reject);
    req.end(body);
  }});
}}

async function emitOnce() {{
  if (printed) return;
  const tokenMatch = promptBuffer.match(/workflow-ack:[A-Za-z0-9_-]+/);
  const deliveryToken = tokenMatch ? tokenMatch[0] : undefined;
  if (!deliveryToken && !fallbackTimer) {{
    fallbackTimer = setTimeout(() => {{
      emitOnce().catch((error) => {{
        process.stdout.write("dev-stub intermediate error: " + error.message + "\n");
      }});
    }}, 250);
    return;
  }}
  printed = true;
  if (deliveryToken) {{
    await callRuntimeTool("ack_workflow_turn", {{
      delivery_token: deliveryToken,
    }});
  }}
  await callRuntimeTool("validate_and_submit_intermediate_workflow_run_output", {{
    workflow_output_json: JSON.stringify(intermediateOutput),
    ...(deliveryToken ? {{ delivery_token: deliveryToken }} : {{}}),
  }});
  process.stdout.write("```json\n" + JSON.stringify(finalPayload) + "\n```\n");
}}

process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {{
  promptBuffer += chunk;
  emitOnce().catch((error) => {{
    process.stdout.write("dev-stub intermediate error: " + error.message + "\n");
  }});
}});
process.stdin.resume();
"#,
        payload_json = payload,
        intermediate_json = intermediate_output,
    );
    format!(
        "stty -echo 2>/dev/null || true; node -e {}",
        shell_single_quote(&js)
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn dev_stub_semantic_renderer_script() -> String {
    let response = serde_json::json!({
        "kind": "http_response",
        "status": 200,
        "headers": { "content-type": "text/html; charset=utf-8" },
        "body": "<!doctype html><html><head><title>About Arroba Foods - Neon</title><style>body{background:#000;color:#39ff14;font-family:system-ui,sans-serif}a{color:#39ff14}main{border:1px solid #39ff14;padding:2rem;box-shadow:0 0 24px #39ff14}</style></head><body data-arroba-render=\"ARROBA_RENDER_NEON_GREEN\"><main><h1>About Arroba Foods</h1><p>We build practical grocery tools for neighborhood stores.</p><a href=\"/contact\">Contact us</a></main></body></html>"
    });
    let payload = serde_json::json!({
        "summary": "semantic renderer stub",
        "output": { "message": response }
    });
    format!(
        "while true; do sleep 2; printf '%s\\n%s\\n%s\\n' '```json' '{}' '```'; done",
        payload
    )
}

fn dev_stub_slow_first_output_script() -> String {
    let delay_ms = std::env::var("ARROBA_DEV_STUB_FIRST_OUTPUT_DELAY_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(35_000)
        .min(300_000);
    let delay_seconds = delay_ms as f64 / 1000.0;
    format!(
        "stty -echo 2>/dev/null || true; while IFS= read -r _line; do sleep {delay_seconds}; printf 'slow-first-output-drill complete\\n'; done"
    )
}

fn dev_stub_large_output_script() -> String {
    let line_count = std::env::var("ARROBA_DEV_STUB_LARGE_OUTPUT_LINES")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(1_000)
        .min(100_000);
    let line_bytes = std::env::var("ARROBA_DEV_STUB_LARGE_OUTPUT_LINE_BYTES")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(128)
        .clamp(32, 4096);
    let payload_bytes = line_bytes.saturating_sub(31);
    format!(
        "stty -echo 2>/dev/null || true; while IFS= read -r _line; do i=1; while [ \"$i\" -le {line_count} ]; do printf 'large-output-drill %06d ' \"$i\"; printf '%*s' {payload_bytes} '' | tr ' ' x; printf '\\n'; i=$((i + 1)); done; done"
    )
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(super) struct FailingPtyAdapter;

#[cfg(test)]
impl FailingPtyAdapter {
    pub(super) const KEY: &'static str = "dev-invalid-pty";
}

#[cfg(test)]
pub(super) static FAILING_PTY_ADAPTER: FailingPtyAdapter = FailingPtyAdapter;

#[cfg(test)]
impl AgentEndpointAdapter for FailingPtyAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: format!(
                "dev-invalid-pty:{}:{}:{}",
                request.provider, request.account_profile, request.model
            ),
            pty_target: Some(format!("invalid-pty:{}", request.session_id)),
            pty_program: Some("/definitely/not/a/real/provider".to_string()),
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: request.provider_env_remove.clone(),
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        })
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

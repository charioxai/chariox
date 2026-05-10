use super::{
    apply_workspace_write_fence, plan_codex_launch, plan_opencode_launch, AgentEndpointMode,
    LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use crate::error::DaemonError;

pub trait AgentEndpointAdapter: Send + Sync {
    fn key(&self) -> &'static str;
    fn connect(&self, request: &LaunchProviderRequest)
        -> Result<ProviderLaunchResult, DaemonError>;
    fn supports_managed_io_write_enforcement(&self) -> bool {
        false
    }
    fn supports_turn_scoped_execution_config(&self) -> bool {
        false
    }
    fn park(&self, run: &RuntimeProviderRun);
    fn resume(&self, run: &RuntimeProviderRun);
    fn terminate(&self, run: &RuntimeProviderRun);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn new() -> Self {
        Self
    }

    pub fn registered_adapter_count(&self) -> usize {
        #[cfg(test)]
        {
            5
        }

        #[cfg(not(test))]
        {
            3
        }
    }

    pub fn resolve(&self, key: &str) -> Option<&'static dyn AgentEndpointAdapter> {
        match key {
            DevStubAdapter::KEY => Some(&DEV_STUB_ADAPTER),
            CodexAdapter::KEY => Some(&CODEX_ADAPTER),
            OpenCodeAdapter::KEY => Some(&OPENCODE_ADAPTER),
            #[cfg(test)]
            ManagedDevStubAdapter::KEY => Some(&MANAGED_DEV_STUB_ADAPTER),
            #[cfg(test)]
            FailingPtyAdapter::KEY => Some(&FAILING_PTY_ADAPTER),
            _ => None,
        }
    }

    pub fn registered_adapter_keys(&self) -> Vec<String> {
        let keys = vec![
            DevStubAdapter::KEY.to_string(),
            CodexAdapter::KEY.to_string(),
            OpenCodeAdapter::KEY.to_string(),
        ];
        #[cfg(not(test))]
        return keys;
        #[cfg(test)]
        {
            let mut keys = keys;
            keys.push(ManagedDevStubAdapter::KEY.to_string());
            keys.push(FailingPtyAdapter::KEY.to_string());
            keys
        }
    }
}

#[derive(Debug, Default)]
struct DevStubAdapter;

impl DevStubAdapter {
    const KEY: &'static str = "dev-stub";
}

static DEV_STUB_ADAPTER: DevStubAdapter = DevStubAdapter;

impl AgentEndpointAdapter for DevStubAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        let pty_args = dev_stub_pty_args(request.model.as_str());
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
            pty_env: std::collections::BTreeMap::new(),
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
struct ManagedDevStubAdapter;

#[cfg(test)]
impl ManagedDevStubAdapter {
    const KEY: &'static str = "managed-dev-stub";
}

#[cfg(test)]
static MANAGED_DEV_STUB_ADAPTER: ManagedDevStubAdapter = ManagedDevStubAdapter;

#[cfg(test)]
impl AgentEndpointAdapter for ManagedDevStubAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn supports_managed_io_write_enforcement(&self) -> bool {
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
            | "semantic-url-renderer-stub"
            | "slow-first-output-drill"
            | "large-output-drill"
    )
}

fn dev_stub_workflow_output_script(summary: &str, value: i64) -> String {
    let payload =
        format!(r#"{{"summary":"{summary}","output":{{"message":{{"value":{value}}}}}}}"#);
    format!("sleep 1; printf '%s\\n%s\\n%s\\n' '```json' '{payload}' '```'; sleep 300")
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

#[derive(Debug, Default)]
struct OpenCodeAdapter;

impl OpenCodeAdapter {
    const KEY: &'static str = "opencode";
}

static OPENCODE_ADAPTER: OpenCodeAdapter = OpenCodeAdapter;

impl AgentEndpointAdapter for OpenCodeAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn supports_managed_io_write_enforcement(&self) -> bool {
        true
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        let mut launch = plan_opencode_launch(Some(request))?;
        launch.process_label = format!("opencode:{}:{}", request.provider, request.model);
        launch.pty_target = None;
        launch.working_directory = request.working_directory.clone();
        apply_workspace_write_fence(launch, request)
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

#[derive(Debug, Default)]
struct CodexAdapter;

impl CodexAdapter {
    const KEY: &'static str = "codex";
}

static CODEX_ADAPTER: CodexAdapter = CodexAdapter;

impl AgentEndpointAdapter for CodexAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn supports_managed_io_write_enforcement(&self) -> bool {
        true
    }

    fn supports_turn_scoped_execution_config(&self) -> bool {
        true
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        let mut launch = plan_codex_launch(Some(request))?;
        launch.process_label = format!("codex:{}:{}", request.provider, request.model);
        launch.pty_target = None;
        launch.working_directory = request.working_directory.clone();
        apply_workspace_write_fence(launch, request)
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

#[cfg(test)]
#[derive(Debug, Default)]
struct FailingPtyAdapter;

#[cfg(test)]
impl FailingPtyAdapter {
    const KEY: &'static str = "dev-invalid-pty";
}

#[cfg(test)]
static FAILING_PTY_ADAPTER: FailingPtyAdapter = FailingPtyAdapter;

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
            pty_env: std::collections::BTreeMap::new(),
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
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{LaunchProviderRequest, ProviderRegistry};

    #[test]
    fn opencode_adapter_resolves_override_and_uses_working_directory() {
        let _guard = crate::env_lock::lock();
        let executable = std::env::temp_dir().join(format!(
            "arroba-opencode-adapter-test-{}",
            std::process::id()
        ));
        fs::write(&executable, "#!/bin/sh\nsleep 60\n").expect("fixture executable should exist");
        std::env::set_var("ARROBA_OPENCODE_BIN", &executable);
        std::env::set_var("ARROBA_OPENCODE_PORT", "43112");

        let request = LaunchProviderRequest::new(
            "session-1",
            "opencode",
            "opencode",
            "default",
            "anthropic/claude-sonnet-4",
        )
        .with_working_directory(PathBuf::from("/tmp"));
        let launch_result = ProviderRegistry::new()
            .resolve("opencode")
            .expect("opencode adapter should exist")
            .connect(&request)
            .expect("opencode launch should resolve");

        std::env::remove_var("ARROBA_OPENCODE_BIN");
        std::env::remove_var("ARROBA_OPENCODE_PORT");
        let _ = fs::remove_file(&executable);

        let expected_program = executable.to_string_lossy().to_string();
        assert_eq!(
            launch_result.pty_program.as_deref(),
            Some(expected_program.as_str())
        );
        assert_eq!(launch_result.working_directory, Some(PathBuf::from("/tmp")));
        assert_eq!(launch_result.pty_args[0], "serve");
        assert_eq!(launch_result.pty_args[1], "--hostname");
        assert_eq!(launch_result.pty_args[2], "127.0.0.1");
        assert_eq!(launch_result.pty_args[3], "--port");
        let port = launch_result.pty_args[4]
            .parse::<u16>()
            .expect("port argument should be numeric");
        let endpoint = format!("http://127.0.0.1:{port}");
        assert_eq!(
            launch_result.structured_endpoint.as_deref(),
            Some(endpoint.as_str())
        );
    }
}

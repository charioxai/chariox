use std::net::TcpListener;
use std::path::PathBuf;

use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

const DIRECTORY: &str = "CHARIOX_LOADLAB_TRACE_DIR";

pub(super) fn plan(request: &LaunchProviderRequest) -> Result<ProviderLaunchResult, DaemonError> {
    let root = std::env::var(DIRECTORY).map_err(|_| DaemonError::InvalidConfig {
        field: DIRECTORY,
        message: "set the absolute directory containing generated load lab traces",
    })?;
    if !PathBuf::from(&root).is_absolute() {
        return Err(DaemonError::InvalidConfig {
            field: DIRECTORY,
            message: "use an absolute directory containing generated load lab traces",
        });
    }
    let mut env = super::dev_stub_pty_env(request);
    env.insert(DIRECTORY.into(), root);
    env.insert(
        "CHARIOX_LOADLAB_TRACE_AGENT".into(),
        request
            .dev_trace_agent_alias
            .as_deref()
            .unwrap_or("orch")
            .into(),
    );
    let port = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "reserve load lab port",
            message: error.to_string(),
        })?
        .port();
    let script = concat!(
        "const { parseTrace } = (() => { const module = { exports: {} };\n",
        include_str!("trace_format.cjs"),
        "\nreturn module.exports; })();\n",
        include_str!("trace_runtime.cjs"),
        "\n",
        include_str!("trace_server.cjs"),
    );
    Ok(ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::Managed,
        process_label: "dev-stub:trace".into(),
        pty_target: Some(format!(
            "stub-pty:trace:{}:{}",
            request.session_id,
            request.agent_id.as_deref().unwrap_or("session")
        )),
        pty_program: Some("node".into()),
        pty_args: vec!["-e".into(), script.into(), port.to_string()],
        pty_env: env,
        pty_env_remove: request.provider_env_remove.clone(),
        working_directory: request.working_directory.clone(),
        structured_endpoint: Some(format!("http://127.0.0.1:{port}")),
    })
}

use crate::error::DaemonError;
use crate::runtime::projection::ProviderRunProjectionStore;

pub(crate) async fn dispatch_authenticated_mcp_proxy_call(
    provider_run_projection: &ProviderRunProjectionStore,
    auth_token: &str,
    name: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    crate::mcp::validate_registry_name(name, "mcp name")?;
    let run = unambiguous_mcp_proxy_provider_run(provider_run_projection, auth_token, name)?;
    let backing = run
        .mcp_servers()
        .iter()
        .find(|server| server.name == name && server.enabled)
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "mcp.proxy.grant",
            message: format!("MCP `{name}` is not granted to provider run `{}`", run.id()),
        })?;
    tokio::task::spawn_blocking(move || {
        crate::provider::dispatch_provider_mcp_proxy_request(&backing, payload)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.proxy.dispatch",
        message: error.to_string(),
    })?
}

pub(crate) fn unambiguous_mcp_proxy_provider_run(
    provider_run_projection: &ProviderRunProjectionStore,
    auth_token: &str,
    name: &str,
) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
    match provider_run_projection
        .active_runs_by_runtime_mcp_auth_token(auth_token)
        .as_slice()
    {
        [run] => Ok(run.clone()),
        [] => Err(DaemonError::LocalTransport {
            operation: "mcp.proxy.auth",
            message: "invalid runtime MCP auth token".to_string(),
        }),
        runs => {
            let mut run_ids = runs
                .iter()
                .map(|run| run.id().to_string())
                .collect::<Vec<_>>();
            run_ids.sort();
            Err(DaemonError::LocalTransport {
                operation: "mcp.proxy.auth",
                message: format!(
                    "runtime MCP auth token is bound to multiple active provider runs ({}) while proxying MCP `{name}`. MCP proxy calls require one authoritative provider run; run /kernel health and /provider processes, then stop duplicate provider runs before retrying.",
                    run_ids.join(",")
                ),
            })
        }
    }
}

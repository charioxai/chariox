//! In-place native harness cache refresh. Provider/run/terminal identity stays fixed.
use super::*;

impl KernelRuntimeState {
    pub(super) async fn refresh_native_runtime_catalog(
        &self,
        run: crate::provider::RuntimeProviderRun,
    ) -> Result<ProviderReloadOutcome, DaemonError> {
        let changes = self.owned.provider_run_projection.catalog_changes().clone();
        let Some(lane) = self
            .owned
            .provider_store
            .run_operation_lanes()
            .try_acquire(run.id())
        else {
            return Ok(ProviderReloadOutcome::Deferred);
        };
        // Contention is not a catalog change. Acquire the run lane first so
        // retries cannot invalidate an otherwise healthy provider's discovery.
        let Some(watch) = changes.begin_refresh(run.id()) else {
            return Ok(ProviderReloadOutcome::Deferred);
        };
        let expected_hash = run.remote_extension_manifest().manifest_hash();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let run_id = run.id().to_owned();
        let provider_store = self.owned.provider_store.clone();
        let expected_run = run.clone();
        // Both the cache reservation and operation lane stay in blocking
        // ownership until actual I/O finishes, even if this caller is cancelled.
        let work = tokio::task::spawn_blocking(move || {
            if std::time::Instant::now() >= deadline || watch.is_closed() {
                return Err(refresh_error("native MCP refresh expired before execution"));
            }
            let current = provider_store.get_run(expected_run.id())?;
            if current.state() == crate::provider::ProviderRunState::Ended
                || current.structured_endpoint() != expected_run.structured_endpoint()
                || current.runtime_mcp_auth_token() != expected_run.runtime_mcp_auth_token()
            {
                return Err(refresh_error(
                    "native provider changed before catalog refresh",
                ));
            }
            request_provider_refresh(&expected_run, deadline)?;
            Ok::<_, DaemonError>((watch, lane))
        });
        let (mut watch, _lane) = tokio::time::timeout_at(deadline.into(), work)
            .await
            .map_err(|_| refresh_error("native MCP refresh request timed out"))?
            .map_err(|_| refresh_error("native MCP refresh task stopped"))??;
        tokio::time::timeout_at(deadline.into(), watch.wait_until_observed())
            .await.map_err(|_| refresh_error("provider did not fetch the changed runtime catalog; check the provider version and MCP connection"))?
            .map_err(|_| refresh_error("native provider ended before catalog refresh"))?;
        let Some(observed) = watch
            .with_observed_current(|| {
                self.owned
                    .provider_store
                    .observe_runtime_tool_catalog(&run_id, &expected_hash)
            })
            .transpose()?
            .flatten()
        else {
            return Err(refresh_error(
                "native catalog changed while refresh was in progress",
            ));
        };
        self.owned.provider_run_projection.update(observed);
        Ok(ProviderReloadOutcome::ToolsRefreshed)
    }
}

fn request_provider_refresh(
    run: &crate::provider::RuntimeProviderRun,
    deadline: std::time::Instant,
) -> Result<(), DaemonError> {
    match run.adapter_key() {
        "codex" => {
            let endpoint = run
                .structured_endpoint()
                .ok_or_else(|| refresh_error("native Codex has no owned app-server endpoint"))?;
            crate::provider::CodexClient::new(run.id(), endpoint)?
                .reload_mcp_servers_until(deadline)
        }
        // These official harnesses consume the standard tools/list_changed
        // notification on their existing runtime MCP connection. Their actual
        // tools/list, not this branch, acknowledges freshness.
        "claude" | "opencode" => Ok(()),
        _ => Err(refresh_error(
            "native provider does not support runtime catalog refresh",
        )),
    }
}
fn refresh_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "native runtime MCP refresh",
        message: message.into(),
    }
}

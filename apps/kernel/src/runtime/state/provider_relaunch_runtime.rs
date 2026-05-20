//! Provider relaunch task orchestration.

use super::*;

impl KernelRuntimeState {
    pub(super) fn spawn_provider_relaunch(
        &self,
        launch_request: crate::provider::LaunchProviderRequest,
        runtime_init_delay_ms: u64,
        terminated_run_id: Option<String>,
        launch_delay_ms: u64,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            if let Some(terminated_run_id) = terminated_run_id {
                let (_, process_key) = state
                    .with_app_side_effect(|app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(&terminated_run_id)
                    })
                    .await
                    .unwrap_or((false, None));
                state
                    .owned
                    .remove_provider_process_tracking_for_run(&terminated_run_id, process_key);
            }
            if launch_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(launch_delay_ms)).await;
            }
            let started = match state.owned.start_provider_launch(launch_request) {
                Ok(started) => started,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "provider policy relaunch failed",
                        serde_json::json!({ "error": error.to_string() }),
                    );
                    return;
                }
            };
            if runtime_init_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(runtime_init_delay_ms)).await;
            }
            let spawn_result = state
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app)
                        .spawn_for_launch(&started.run)
                })
                .await;
            if let Err(error) = spawn_result {
                state.fail_provider_launch(&started, &error).await;
                return;
            }
            let run = started.run.clone();
            let binding = tokio::task::spawn_blocking(move || {
                crate::provider::ProviderProcessService::initialize_runtime_binding(&run)
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize provider runtime",
                message: error.to_string(),
            });
            match binding {
                Ok(Ok(binding)) => {
                    state.finish_provider_launch(&started, binding).await;
                }
                Ok(Err(error)) | Err(error) => {
                    state.fail_provider_launch(&started, &error).await;
                }
            }
        });
    }
}

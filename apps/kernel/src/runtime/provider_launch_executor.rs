use tokio::time::{sleep, Duration};

use crate::app::StartedProviderLaunch;
use crate::error::DaemonError;
use crate::local::{LaunchProviderRunRequest, LocalDaemonResponse};
use crate::provider::ProviderProcessService;
use crate::runtime::state::KernelRuntimeState;

#[derive(Clone)]
pub(crate) struct ProviderLaunchCommandExecutor {
    store: ProviderLaunchStore,
}

#[derive(Clone)]
pub(crate) struct ProviderLaunchStore {
    state: KernelRuntimeState,
}

impl ProviderLaunchCommandExecutor {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self {
            store: ProviderLaunchStore::new(state),
        }
    }

    pub(crate) async fn execute(
        &self,
        request: LaunchProviderRunRequest,
        caller_user_id: String,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (started, runtime_init_delay_ms) =
            self.store.start_launch(request, caller_user_id).await?;
        let accepted = started.run.clone();
        let store = self.store.clone();
        tokio::spawn(async move {
            if runtime_init_delay_ms > 0 {
                sleep(Duration::from_millis(runtime_init_delay_ms)).await;
            }
            let run = started.run.clone();
            let binding = tokio::task::spawn_blocking(move || {
                ProviderProcessService::initialize_runtime_binding(&run)
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize provider runtime",
                message: error.to_string(),
            });

            match binding {
                Ok(Ok(binding)) => {
                    store.finish_launch(&started, binding).await;
                }
                Ok(Err(error)) | Err(error) => {
                    store.fail_launch(&started, &error).await;
                }
            }
        });
        Ok(LocalDaemonResponse::ProviderRunLaunchAccepted {
            provider_run: accepted,
        })
    }
}

impl ProviderLaunchStore {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self { state }
    }

    async fn start_launch(
        &self,
        request: LaunchProviderRunRequest,
        caller_user_id: String,
    ) -> Result<(StartedProviderLaunch, u64), DaemonError> {
        self.state
            .start_provider_launch(request, caller_user_id)
            .await
    }

    async fn finish_launch(
        &self,
        started: &StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) {
        self.state.finish_provider_launch(started, binding).await;
    }

    async fn fail_launch(&self, started: &StartedProviderLaunch, error: &DaemonError) {
        self.state.fail_provider_launch(started, error).await;
    }
}

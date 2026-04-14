use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::app::{DaemonApp, StartedProviderLaunch};
use crate::error::DaemonError;
use crate::local::provider_requests::launch_provider_request_from_local;
use crate::local::{LaunchProviderRunRequest, LocalDaemonResponse};
use crate::provider::ProviderProcessService;

#[derive(Clone)]
pub(crate) struct ProviderLaunchCommandExecutor {
    store: ProviderLaunchStore,
}

#[derive(Clone)]
pub(crate) struct ProviderLaunchStore {
    app: Arc<Mutex<DaemonApp>>,
}

impl ProviderLaunchCommandExecutor {
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self {
            store: ProviderLaunchStore::new(app),
        }
    }

    pub(crate) async fn execute(
        &self,
        request: LaunchProviderRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (started, runtime_init_delay_ms) = self.store.start_launch(request).await?;
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
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self { app }
    }

    async fn start_launch(
        &self,
        request: LaunchProviderRunRequest,
    ) -> Result<(StartedProviderLaunch, u64), DaemonError> {
        let mut app = self.app.lock().await;
        let launch_request = launch_provider_request_from_local(&app, request);
        Ok((
            app.start_provider_launch(launch_request)?,
            app.config().provider_runtime_init_delay_ms,
        ))
    }

    async fn finish_launch(
        &self,
        started: &StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) {
        let mut app = self.app.lock().await;
        if let Err(error) = app.finish_provider_launch(started, binding) {
            app.fail_provider_launch(started, &error);
        }
    }

    async fn fail_launch(&self, started: &StartedProviderLaunch, error: &DaemonError) {
        let mut app = self.app.lock().await;
        app.fail_provider_launch(started, error);
    }
}

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::provider_requests::launch_provider_request_from_local;
use crate::local::{LaunchProviderRunRequest, LocalDaemonResponse};

#[derive(Clone)]
pub(crate) struct ProviderLaunchCommandExecutor {
    app: Arc<Mutex<DaemonApp>>,
}

impl ProviderLaunchCommandExecutor {
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self { app }
    }

    pub(crate) async fn execute(
        &self,
        request: LaunchProviderRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (started, runtime_init_delay_ms) = {
            let mut app = self.app.lock().await;
            let launch_request = launch_provider_request_from_local(&app, request);
            (
                app.start_provider_launch(launch_request)?,
                app.config().provider_runtime_init_delay_ms,
            )
        };
        let accepted = started.run.clone();
        let app = Arc::clone(&self.app);
        tokio::spawn(async move {
            if runtime_init_delay_ms > 0 {
                sleep(Duration::from_millis(runtime_init_delay_ms)).await;
            }
            let run = started.run.clone();
            let binding = tokio::task::spawn_blocking(move || {
                DaemonApp::initialize_provider_runtime_binding(&run)
            })
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "initialize provider runtime",
                message: error.to_string(),
            });

            match binding {
                Ok(Ok(binding)) => {
                    let mut app = app.lock().await;
                    if let Err(error) = app.finish_provider_launch(&started, binding) {
                        app.fail_provider_launch(&started, &error);
                    }
                }
                Ok(Err(error)) | Err(error) => {
                    let mut app = app.lock().await;
                    app.fail_provider_launch(&started, &error);
                }
            }
        });
        Ok(LocalDaemonResponse::ProviderRunLaunchAccepted {
            provider_run: accepted,
        })
    }
}

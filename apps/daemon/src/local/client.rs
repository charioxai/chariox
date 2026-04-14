use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::runtime::{Builder, Runtime};
use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::command::{KernelCommand, KernelCommandSource};
use crate::kernel::router::{CommandRouter, INTERACTIVE_COMMAND_QUEUE_LIMIT};
use crate::session::unix_epoch_ms;

use super::{LocalDaemonRequest, LocalDaemonResponse};

/// In-process local request client for tests and smoke harnesses.
///
/// This keeps local request callers on the same `CommandRouter` boundary as IPC
/// and kernel transport without exposing `DaemonApp::handle_local_request`.
pub struct LocalDaemonClient {
    router: CommandRouter,
    runtime: Runtime,
    command_sequence: AtomicU64,
}

impl LocalDaemonClient {
    pub fn new(app: DaemonApp) -> Result<Self, DaemonError> {
        let provider_runtime_lanes = app.provider_run_operation_lanes();
        let app = Arc::new(Mutex::new(app));
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "create local daemon client runtime",
                message: error.to_string(),
            })?;
        let router = runtime.block_on(async {
            CommandRouter::with_interactive_capacity_and_provider_lanes(
                Arc::clone(&app),
                INTERACTIVE_COMMAND_QUEUE_LIMIT,
                provider_runtime_lanes,
            )
        });

        Ok(Self {
            router,
            runtime,
            command_sequence: AtomicU64::new(1),
        })
    }

    pub fn send(&self, request: LocalDaemonRequest) -> Result<LocalDaemonResponse, DaemonError> {
        let sequence = self.command_sequence.fetch_add(1, Ordering::Relaxed);
        let command_id = format!("local-client-{}-{sequence}", unix_epoch_ms());
        let command = KernelCommand::from_local_request_with_source(
            command_id,
            KernelCommandSource::LocalIpc,
            None,
            None,
            &request,
        );

        self.runtime
            .block_on(async { self.router.dispatch(command, request).await })
    }
}

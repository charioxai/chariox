use std::net::TcpListener;
use std::sync::Arc;

use crate::app::{provider_runtime, DaemonApp};
use crate::error::DaemonError;

impl DaemonApp {
    pub fn startup_message(&self) -> String {
        format!(
            "chariox daemon {} ready on machine {} ({})",
            self.config.daemon_id,
            self.config.host_machine_id,
            self.config.kernel_websocket_url()
        )
    }

    pub fn shutdown_cleanup(&mut self) -> Result<(), DaemonError> {
        let session_ids = self
            .sessions
            .list_sessions()
            .into_iter()
            .map(|session| session.id().to_string())
            .collect::<Vec<_>>();
        let mut first_error = None;

        for session_id in session_ids {
            if let Err(error) = self.shutdown_cleanup_session_runtime(&session_id) {
                crate::logging::error_with_fields(
                    "daemon.shutdown",
                    "failed to clean session runtime during daemon shutdown",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        crate::provider::shutdown_opencode_account_endpoints();
        crate::provider::shutdown_codex_account_endpoints();

        if let Some(error) = first_error {
            return Err(error);
        }

        Ok(())
    }

    fn shutdown_cleanup_session_runtime(&mut self, session_id: &str) -> Result<(), DaemonError> {
        let removed_attachments = self.attachments.remove_session_attachments(session_id);
        for attachment in &removed_attachments {
            match self
                .sessions
                .write()
                .remove_attachment_from_session(session_id, attachment.id())
            {
                Ok(_) | Err(DaemonError::AttachmentNotInSession { .. }) => {}
                Err(error) => return Err(error),
            }
        }

        let terminated_runs = self
            .providers
            .terminate_session_runs_provider_only(session_id)?;
        let terminated_run_ids = terminated_runs
            .runs()
            .iter()
            .map(|outcome| outcome.run().id().to_string())
            .collect::<Vec<_>>();
        for outcome in terminated_runs.into_runs() {
            let run = outcome.into_run();
            if self
                .sessions
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(run.id())
            {
                self.sessions.set_active_provider_run(session_id, None)?;
            }
            self.update_provider_run_projection(run.clone());
            provider_runtime::ProviderProcessTracker::new(self).remove_run(run.id())?;
        }

        for run in self.providers.list_runs() {
            if run.session_id() == session_id {
                crate::transport::flow_control::clear_prompt_activity(self, run.id());
            }
        }
        self.prompt_owner_remove_session(session_id);

        let mut session = self.sessions.get_session(session_id)?;
        let reconciliation = session.interrupt_runtime_for_shutdown();
        let agents = self.agents.get_session_agents(session_id);
        session.set_agents(agents);
        self.sessions.restore_session(session.clone());
        self.update_session_projection(session.clone());

        crate::logging::info_with_fields(
            "daemon.shutdown",
            "session runtime cleaned for daemon shutdown",
            serde_json::json!({
                "session_id": session_id,
                "session_status": session.status(),
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "cleared_active_provider_run": reconciliation.cleared_active_provider_run,
                "removed_orphaned_workflow_prompt_count": reconciliation.removed_orphaned_workflow_prompt_count,
                "interrupted_prompt_count": reconciliation.interrupted_prompt_count,
                "stopped_workflow_run_count": reconciliation.stopped_workflow_run_count,
            }),
        );

        Ok(())
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        self.run_with_listener(None).await
    }

    pub async fn run_on_listener(self, listener: TcpListener) -> Result<(), DaemonError> {
        self.run_with_listener(Some(listener)).await
    }

    async fn run_with_listener(self, listener: Option<TcpListener>) -> Result<(), DaemonError> {
        let empty_context_completion =
            crate::managed_context::empty::EmptyManagedContextCompletion::prepare(
                self.config(),
                self.managed_kernel_registration().as_ref(),
            )?;
        let managed_activity_reporter =
            crate::runtime::managed_kernel_activity::ManagedKernelActivityReporter::from_runtime(
                self.config(),
                self.managed_kernel_registration().as_ref(),
            )?;
        let legacy_workflow_history = self.legacy_workflow_history_store();
        let history_migration_store = self.durable_state_store();
        let history_migration_owner = self.config.daemon_id.clone();
        let app = Arc::new(tokio::sync::Mutex::new(self));
        let router = std::sync::Arc::new(
            crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
                Arc::clone(&app),
                crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
            ),
        );
        let runtime_state = router.runtime_state();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let managed_activity_task = managed_activity_reporter
            .map(|reporter| tokio::spawn(reporter.run(runtime_state.clone(), shutdown_rx.clone())));
        let history_migration_task = (!legacy_workflow_history.is_empty()).then(|| {
            tokio::spawn(run_legacy_workflow_history_migration(
                history_migration_store,
                history_migration_owner,
                legacy_workflow_history,
                shutdown_rx.clone(),
            ))
        });
        let relay_state = {
            let app = app.lock().await;
            app.relay_client_state()
        };
        let relay_task = tokio::spawn(
            crate::transport::relay_client::run_daemon_relay_connector_with_router(
                Arc::clone(&router),
                relay_state.clone(),
                shutdown_rx.clone(),
            ),
        );
        let empty_context_completion_task = empty_context_completion.map(|completion| {
            tokio::spawn(completion.run(relay_state.clone(), shutdown_rx.clone()))
        });
        let event_delivery_config = {
            let app = app.lock().await;
            let config_projection = app.config_projection_store();
            crate::transport::event_delivery_client::EventDeliveryClientConfig {
                url: app.config().event_delivery_url.clone(),
                token: app.config().event_delivery_token.clone(),
                kernel_id: app.config().daemon_id.clone(),
                environment_id: app.config().event_delivery_environment_id.clone(),
                generator_management_targets: app
                    .config()
                    .event_generator_management_targets
                    .clone(),
                config_projection,
            }
        };
        let event_delivery_task = tokio::spawn(
            crate::transport::event_delivery_client::run_event_delivery_connector(
                runtime_state.clone(),
                event_delivery_config,
                shutdown_rx.clone(),
            ),
        );
        let external_provider_session_discovery_task = tokio::spawn(
            crate::runtime::external_provider_session_control::run_external_provider_session_discovery_poller(
                Arc::clone(&app),
                runtime_state.clone(),
                shutdown_rx.clone(),
            ),
        );
        let event_connection_reconciliation_router = Arc::clone(&router);
        let event_connection_reconciliation_task = tokio::spawn(async move {
            event_connection_reconciliation_router
                .run_event_connection_authorization_reconciler(shutdown_rx.clone())
                .await;
        });
        // Attached external provider transcript observation intentionally does not run.
        // Discovery may list/import external sessions, but provider transcript history
        // must not mutate kernel-owned active prompt state.
        let _ = runtime_state;

        let shutdown = async {
            wait_for_shutdown_signal().await;
            let _ = shutdown_tx.send(true);
        };
        let result = match listener {
            Some(listener) => {
                crate::runtime_transport::run_kernel_websocket_server_with_router_on_listener(
                    router, listener, shutdown,
                )
                .await
            }
            None => {
                crate::runtime_transport::run_kernel_websocket_server_with_router(router, shutdown)
                    .await
            }
        };

        let _ = shutdown_tx.send(true);
        let _ = relay_task.await;
        if let Some(task) = empty_context_completion_task {
            let _ = task.await;
        }
        let _ = event_delivery_task.await;
        let _ = external_provider_session_discovery_task.await;
        let _ = event_connection_reconciliation_task.await;
        if let Some(task) = managed_activity_task {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => crate::logging::error_with_fields(
                    "managed_kernel.activity",
                    "managed kernel activity reporter stopped",
                    serde_json::json!({ "error": error.to_string() }),
                ),
                Err(error) => crate::logging::error_with_fields(
                    "managed_kernel.activity",
                    "managed kernel activity reporter task failed",
                    serde_json::json!({ "error": error.to_string() }),
                ),
            }
        }
        if let Some(task) = history_migration_task {
            let _ = task.await;
        }
        result
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(signal) => Some(signal),
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.shutdown",
                "failed to register SIGTERM handler",
                serde_json::json!({ "error": error.to_string() }),
            );
            None
        }
    };
    let mut hangup = match signal(SignalKind::hangup()) {
        Ok(signal) => Some(signal),
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.shutdown",
                "failed to register SIGHUP handler",
                serde_json::json!({ "error": error.to_string() }),
            );
            None
        }
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = wait_for_unix_signal(&mut terminate) => {}
        _ = wait_for_unix_signal(&mut hangup) => {}
    }
}

#[cfg(unix)]
async fn wait_for_unix_signal(signal: &mut Option<tokio::signal::unix::Signal>) {
    match signal {
        Some(signal) => {
            let _ = signal.recv().await;
        }
        None => std::future::pending().await,
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn run_legacy_workflow_history_migration(
    store: crate::durable_state::DurableKernelStateStore,
    owner_id: String,
    workflow_runs: crate::app::LegacyWorkflowHistoryStore,
    shutdown: tokio::sync::watch::Receiver<bool>,
) {
    const MIGRATION_CHUNK_SIZE: usize = 256;
    let run_count = workflow_runs.len();
    crate::logging::info_with_fields(
        "durable_state.migration",
        "started background workflow history migration",
        serde_json::json!({
            "workflow_run_count": run_count,
            "chunk_size": MIGRATION_CHUNK_SIZE,
        }),
    );
    let mut completed_run_count = 0usize;
    loop {
        if *shutdown.borrow() {
            return;
        }
        let chunk = workflow_runs.next_chunk(MIGRATION_CHUNK_SIZE);
        if chunk.is_empty() {
            break;
        }
        let store = store.clone();
        let owner_id = owner_id.clone();
        let migration_chunk = chunk.clone();
        let completed = chunk.len() == workflow_runs.len();
        let result = tokio::task::spawn_blocking(move || {
            store.migrate_legacy_workflow_history_chunk(&owner_id, &migration_chunk, completed)
        })
        .await;
        match result {
            Ok(Ok(())) => {
                workflow_runs.remove_committed(&chunk);
                completed_run_count += chunk.len();
            }
            Ok(Err(error)) => {
                crate::logging::warn_with_fields(
                    "durable_state.migration",
                    "background workflow history migration failed",
                    serde_json::json!({
                        "error": error.to_string(),
                        "completed_run_count": completed_run_count,
                        "workflow_run_count": run_count,
                    }),
                );
                return;
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "durable_state.migration",
                    "background workflow history migration worker failed",
                    serde_json::json!({"error": error.to_string()}),
                );
                return;
            }
        }
        tokio::task::yield_now().await;
    }
    crate::logging::info_with_fields(
        "durable_state.migration",
        "completed background workflow history migration",
        serde_json::json!({"workflow_run_count": run_count}),
    );
    match tokio::task::spawn_blocking(move || store.reclaim_unused_pages_incrementally(512)).await {
        Ok(Ok(outcome)) => crate::logging::info_with_fields(
            "durable_state.maintenance",
            "completed bounded background incremental reclaim",
            serde_json::json!({
                "supported": outcome.supported,
                "free_pages_before": outcome.free_pages_before,
                "free_pages_after": outcome.free_pages_after,
            }),
        ),
        Ok(Err(error)) => crate::logging::warn_with_fields(
            "durable_state.maintenance",
            "bounded background incremental reclaim failed",
            serde_json::json!({"error": error.to_string()}),
        ),
        Err(error) => crate::logging::warn_with_fields(
            "durable_state.maintenance",
            "bounded background incremental reclaim worker failed",
            serde_json::json!({"error": error.to_string()}),
        ),
    }
}

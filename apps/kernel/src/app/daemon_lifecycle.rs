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
                "interrupted_prompt_count": reconciliation.interrupted_prompt_count,
                "stopped_workflow_run_count": reconciliation.stopped_workflow_run_count,
            }),
        );

        Ok(())
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        let app = Arc::new(tokio::sync::Mutex::new(self));
        let router = std::sync::Arc::new(
            crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
                Arc::clone(&app),
                crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
            ),
        );
        let runtime_state = router.runtime_state();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let relay_state = {
            let app = app.lock().await;
            app.relay_client_state()
        };
        let relay_task = tokio::spawn(
            crate::transport::relay_client::run_daemon_relay_connector_with_router(
                Arc::clone(&router),
                relay_state,
                shutdown_rx.clone(),
            ),
        );
        let event_delivery_config = {
            let app = app.lock().await;
            crate::transport::event_delivery_client::EventDeliveryClientConfig {
                url: app.config().event_delivery_url.clone(),
                token: app.config().event_delivery_token.clone(),
                kernel_id: app.config().daemon_id.clone(),
                environment_id: app.config().event_delivery_environment_id.clone(),
                generator_management_targets: app
                    .config()
                    .event_generator_management_targets
                    .clone(),
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

        let result =
            crate::runtime_transport::run_kernel_websocket_server_with_router(router, async {
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown_tx.send(true);
            })
            .await;

        let _ = shutdown_tx.send(true);
        let _ = relay_task.await;
        let _ = event_delivery_task.await;
        let _ = external_provider_session_discovery_task.await;
        let _ = event_connection_reconciliation_task.await;
        result
    }
}

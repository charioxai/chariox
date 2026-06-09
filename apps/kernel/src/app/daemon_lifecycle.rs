use std::sync::Arc;

use crate::app::{DaemonApp, provider_runtime};
use crate::error::DaemonError;

impl DaemonApp {
    pub fn startup_message(&self) -> String {
        format!(
            "arroba daemon {} ready on machine {} ({})",
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
        let reconciliation = session.reconcile_after_kernel_restart();
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
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let relay_state = {
            let app = app.lock().await;
            app.relay_client_state()
        };
        let relay_task = tokio::spawn(crate::transport::relay_client::run_daemon_relay_connector(
            Arc::clone(&app),
            relay_state,
            shutdown_rx,
        ));
        let external_provider_discovery_task = tokio::spawn(
            crate::runtime::external_provider_session_control::run_external_provider_session_discovery_poller(
                Arc::clone(&app),
                shutdown_tx.subscribe(),
            ),
        );

        let result =
            crate::runtime_transport::run_kernel_websocket_server(Arc::clone(&app), async {
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown_tx.send(true);
            })
            .await;

        let _ = shutdown_tx.send(true);
        let _ = relay_task.await;
        let _ = external_provider_discovery_task.await;
        result
    }
}

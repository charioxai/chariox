use base64::Engine;

use super::*;

impl KernelRuntimeState {
    pub(crate) async fn resize_terminal(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        if let Some(provider_run_id) = self.owned.resize_terminal(session_id)? {
            self.with_app_side_effect(|app| app.pty_mut().resize(&provider_run_id, cols, rows))
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn send_terminal_input(
        &self,
        session_id: &str,
        attachment_id: &str,
        provider_run_id: Option<&str>,
        data_base64: &str,
    ) -> Result<usize, DaemonError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data_base64)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "send terminal input",
                message: format!("data_base64 is not valid base64: {error}"),
            })?;
        let byte_count = bytes.len();
        let session_id = session_id.to_string();
        let attachment_id = attachment_id.to_string();
        let provider_run_id = match provider_run_id {
            Some(provider_run_id) => provider_run_id.to_string(),
            None => self
                .owned
                .session_store
                .get_session(&session_id)?
                .active_provider_run_id()
                .ok_or_else(|| DaemonError::NoActiveProviderRun {
                    session_id: session_id.clone(),
                })?
                .to_string(),
        };
        if self.owned.provider_store.get_run(&provider_run_id).is_err() {
            if let Some(projected_run) = self
                .owned
                .provider_run_projection
                .get(&provider_run_id)
                .filter(|run| run.session_id() == session_id)
            {
                if let Some(agent_id) = projected_run.agent_instance_id() {
                    let agent = self.owned.agent_store.get_agent(agent_id)?;
                    if let Some(remote_execution) = agent.remote_execution().cloned() {
                        self.owned
                            .ensure_attachment_in_session(&session_id, &attachment_id)?;
                        let mut relay_config = self.owned.config_projection.snapshot();
                        if let (Some(relay_url), Some(relay_token)) = (
                            remote_execution.relay_url.clone(),
                            remote_execution.relay_token.clone(),
                        ) {
                            relay_config.apply_remote_relay_override(relay_url, relay_token);
                        }
                        let response =
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                &relay_config,
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::SendLeasedNativeProviderInput {
                                    leased_agent_id: remote_execution.leased_agent_id,
                                    provider_run_id: provider_run_id.clone(),
                                    attachment_id: attachment_id.clone(),
                                    data_base64: data_base64.to_string(),
                                },
                            )
                            .await?;
                        return match response {
                            RelayPeerResponse::LeasedNativeProviderInputSent { byte_count } => {
                                Ok(byte_count)
                            }
                            other => Err(DaemonError::LocalTransport {
                                operation: "send leased native provider input",
                                message: format!(
                                    "unexpected remote native provider input response: {other:?}"
                                ),
                            }),
                        };
                    }
                }
            }
        }
        self.with_app_side_effect(move |app| {
            app.send_terminal_input(&session_id, &attachment_id, Some(&provider_run_id), &bytes)
        })
        .await?;
        Ok(byte_count)
    }

    pub(crate) async fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(), DaemonError> {
        let _ = self
            .owned
            .ensure_attachment_in_session(session_id, attachment_id)?;
        Ok(())
    }

    pub(crate) async fn drain_notice_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<crate::terminal::RuntimeNoticeRecord> {
        self.owned
            .terminal_stream
            .drain_notice_records(session_id, attachment_id)
    }

    pub(crate) fn append_native_provider_output(
        &self,
        request: crate::local::AppendNativeProviderOutputRequest,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        self.owned
            .ensure_attachment_in_session(&request.session_id, &request.attachment_id)?;
        let provider_run = self
            .owned
            .provider_store
            .get_run(&request.provider_run_id)?;
        if provider_run.session_id() != request.session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: request.session_id,
                provider_run_id: request.provider_run_id,
            });
        }
        let recipient_attachment_ids = self
            .owned
            .attachment_store
            .list_session_attachment_ids(&request.session_id);
        let record = self.owned.fan_out_terminal_output(
            &request.session_id,
            &request.provider_run_id,
            request.kind,
            request.merge_key,
            recipient_attachment_ids,
            request.text.as_bytes(),
        );
        Ok(vec![record])
    }
}

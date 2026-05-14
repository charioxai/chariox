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
        let provider_run_id = provider_run_id.map(str::to_string);
        self.with_app_side_effect(move |app| {
            app.send_terminal_input(
                &session_id,
                &attachment_id,
                provider_run_id.as_deref(),
                &bytes,
            )
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
}

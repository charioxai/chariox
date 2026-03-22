use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInputRecord {
    pub session_id: String,
    pub provider_run_id: String,
    pub source_attachment_id: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutputKind {
    ProviderOutput,
    PromptEcho,
    ProviderReasoning,
    ProviderTool,
    ProviderError,
    ProviderStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOutputRecord {
    pub session_id: String,
    pub provider_run_id: String,
    pub kind: TerminalOutputKind,
    pub recipient_attachment_ids: Vec<String>,
    pub pending_recipient_attachment_ids: Vec<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeNoticeRecord {
    pub session_id: String,
    pub provider_run_id: Option<String>,
    pub recipient_attachment_ids: Vec<String>,
    pub pending_recipient_attachment_ids: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct TerminalStreamService {
    input_records: Vec<TerminalInputRecord>,
    output_records: Vec<TerminalOutputRecord>,
    notice_records: Vec<RuntimeNoticeRecord>,
}

impl TerminalStreamService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_input(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        bytes: &[u8],
    ) {
        self.input_records.push(TerminalInputRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            source_attachment_id: source_attachment_id.to_string(),
            bytes: bytes.to_vec(),
        });
    }

    pub fn fan_out_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        kind: TerminalOutputKind,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = TerminalOutputRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            kind,
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            bytes: bytes.to_vec(),
        };

        self.output_records.push(record.clone());
        record
    }

    pub fn record_notice(
        &mut self,
        session_id: &str,
        provider_run_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> RuntimeNoticeRecord {
        let record = RuntimeNoticeRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.map(str::to_string),
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            message: message.into(),
        };

        self.notice_records.push(record.clone());
        record
    }

    pub fn input_records(&self) -> &[TerminalInputRecord] {
        &self.input_records
    }

    pub fn output_records(&self) -> &[TerminalOutputRecord] {
        &self.output_records
    }

    pub fn drain_output_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<TerminalOutputRecord> {
        let mut drained = Vec::new();
        for record in &mut self.output_records {
            if record.session_id == session_id
                && record
                    .pending_recipient_attachment_ids
                    .iter()
                    .any(|id| id == attachment_id)
            {
                drained.push(record.clone());
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
            }
        }

        self.output_records
            .retain(|record| !record.pending_recipient_attachment_ids.is_empty());
        drained
    }

    pub fn notice_records(&self) -> &[RuntimeNoticeRecord] {
        &self.notice_records
    }

    pub fn drain_notice_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<RuntimeNoticeRecord> {
        let mut drained = Vec::new();
        for record in &mut self.notice_records {
            if record.session_id == session_id
                && (record.pending_recipient_attachment_ids.is_empty()
                    || record
                        .pending_recipient_attachment_ids
                        .iter()
                        .any(|id| id == attachment_id))
            {
                drained.push(record.clone());
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
            }
        }

        self.notice_records.retain(|record| {
            if record.recipient_attachment_ids.is_empty() {
                false
            } else {
                !record.pending_recipient_attachment_ids.is_empty()
            }
        });
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::{TerminalOutputKind, TerminalStreamService};

    #[test]
    fn records_terminal_input_and_fans_out_output() {
        let mut terminal = TerminalStreamService::new();

        terminal.record_input("session-1", "provider-run-1", "attachment-1", b"ls\n");
        let output = terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            TerminalOutputKind::ProviderOutput,
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            b"listing\n",
        );
        let notice = terminal.record_notice(
            "session-1",
            Some("provider-run-1"),
            vec!["attachment-2".to_string()],
            "provider switch failed; resumed previous run",
        );

        assert_eq!(terminal.input_records().len(), 1);
        assert_eq!(terminal.output_records().len(), 1);
        assert_eq!(terminal.notice_records().len(), 1);
        assert_eq!(output.kind, TerminalOutputKind::ProviderOutput);
        assert_eq!(output.recipient_attachment_ids.len(), 2);
        assert_eq!(output.pending_recipient_attachment_ids.len(), 2);
        assert_eq!(notice.provider_run_id.as_deref(), Some("provider-run-1"));
        assert_eq!(notice.recipient_attachment_ids.len(), 1);
        assert_eq!(notice.pending_recipient_attachment_ids.len(), 1);
    }

    #[test]
    fn output_polling_is_per_recipient() {
        let mut terminal = TerminalStreamService::new();
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            TerminalOutputKind::PromptEcho,
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            b"hello\n",
        );

        let first = terminal.drain_output_records("session-1", "attachment-1");
        assert_eq!(first.len(), 1);
        assert_eq!(terminal.output_records().len(), 1);

        let second = terminal.drain_output_records("session-1", "attachment-2");
        assert_eq!(second.len(), 1);
        assert!(terminal.output_records().is_empty());
    }

    #[test]
    fn notice_polling_is_per_recipient() {
        let mut terminal = TerminalStreamService::new();
        terminal.record_notice(
            "session-1",
            None,
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            "queued prompt",
        );

        let first = terminal.drain_notice_records("session-1", "attachment-1");
        assert_eq!(first.len(), 1);
        assert_eq!(terminal.notice_records().len(), 1);

        let second = terminal.drain_notice_records("session-1", "attachment-2");
        assert_eq!(second.len(), 1);
        assert!(terminal.notice_records().is_empty());
    }
}

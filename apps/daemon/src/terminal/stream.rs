use serde::{Deserialize, Serialize};

const DEFAULT_PENDING_OUTPUT_RECORD_LIMIT_PER_ATTACHMENT: usize = 4096;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub kind: TerminalOutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
    pub recipient_attachment_ids: Vec<String>,
    pub pending_recipient_attachment_ids: Vec<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeNoticeRecord {
    pub session_id: String,
    pub provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub recipient_attachment_ids: Vec<String>,
    pub pending_recipient_attachment_ids: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantMessageCompletionRecord {
    pub session_id: String,
    pub provider_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub recipient_attachment_ids: Vec<String>,
    pub pending_recipient_attachment_ids: Vec<String>,
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TerminalStreamHealthSnapshot {
    pub pending_output_records: usize,
    pub pending_notice_records: usize,
    pub pending_completion_records: usize,
    pub pending_output_record_limit_per_attachment: usize,
    pub trimmed_pending_output_recipients: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TerminalStreamService {
    input_records: Vec<TerminalInputRecord>,
    output_records: Vec<TerminalOutputRecord>,
    notice_records: Vec<RuntimeNoticeRecord>,
    completion_records: Vec<AssistantMessageCompletionRecord>,
    pending_output_record_limit_per_attachment: usize,
    trimmed_pending_output_recipients: u64,
}

impl TerminalStreamService {
    pub fn new() -> Self {
        Self {
            pending_output_record_limit_per_attachment:
                DEFAULT_PENDING_OUTPUT_RECORD_LIMIT_PER_ATTACHMENT,
            ..Self::default()
        }
    }

    #[cfg(test)]
    fn with_pending_output_record_limit_per_attachment(limit: usize) -> Self {
        Self {
            pending_output_record_limit_per_attachment: limit,
            ..Self::new()
        }
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

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = TerminalOutputRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            kind,
            merge_key,
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            bytes: bytes.to_vec(),
        };

        self.output_records.push(record.clone());
        self.enforce_pending_output_record_limits();
        record
    }

    pub fn record_notice(
        &mut self,
        session_id: &str,
        provider_run_id: Option<&str>,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> RuntimeNoticeRecord {
        let record = RuntimeNoticeRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.map(str::to_string),
            agent_id: agent_id.map(str::to_string),
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

    pub fn health_snapshot(&self) -> TerminalStreamHealthSnapshot {
        TerminalStreamHealthSnapshot {
            pending_output_records: self.output_records.len(),
            pending_notice_records: self.notice_records.len(),
            pending_completion_records: self.completion_records.len(),
            pending_output_record_limit_per_attachment: self
                .pending_output_record_limit_per_attachment,
            trimmed_pending_output_recipients: self.trimmed_pending_output_recipients,
        }
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

    fn enforce_pending_output_record_limits(&mut self) {
        if self.pending_output_record_limit_per_attachment == 0 {
            let trimmed = self
                .output_records
                .iter()
                .map(|record| record.pending_recipient_attachment_ids.len() as u64)
                .sum::<u64>();
            self.trimmed_pending_output_recipients = self
                .trimmed_pending_output_recipients
                .saturating_add(trimmed);
            self.output_records.clear();
            return;
        }

        let mut pending_counts = std::collections::BTreeMap::<String, usize>::new();
        let mut trimmed = 0_u64;
        for record in self.output_records.iter_mut().rev() {
            record
                .pending_recipient_attachment_ids
                .retain(|attachment_id| {
                    let count = pending_counts.entry(attachment_id.clone()).or_default();
                    *count += 1;
                    let keep = *count <= self.pending_output_record_limit_per_attachment;
                    if !keep {
                        trimmed += 1;
                    }
                    keep
                });
        }
        self.trimmed_pending_output_recipients = self
            .trimmed_pending_output_recipients
            .saturating_add(trimmed);
        self.output_records
            .retain(|record| !record.pending_recipient_attachment_ids.is_empty());
    }

    pub fn notice_records(&self) -> &[RuntimeNoticeRecord] {
        &self.notice_records
    }

    pub fn record_assistant_message_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) -> AssistantMessageCompletionRecord {
        let record = AssistantMessageCompletionRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            message_id: message_id.to_string(),
            completed_at_ms,
        };

        self.completion_records.push(record.clone());
        record
    }

    pub fn drain_completion_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<AssistantMessageCompletionRecord> {
        let mut drained = Vec::new();
        for record in &mut self.completion_records {
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

        self.completion_records
            .retain(|record| !record.pending_recipient_attachment_ids.is_empty());
        drained
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
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            Some("part-1".to_string()),
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            b"listing\n",
        );
        let notice = terminal.record_notice(
            "session-1",
            Some("provider-run-1"),
            Some("agent-1"),
            vec!["attachment-2".to_string()],
            "provider switch failed; resumed previous run",
        );

        assert_eq!(terminal.input_records().len(), 1);
        assert_eq!(terminal.output_records().len(), 1);
        assert_eq!(terminal.notice_records().len(), 1);
        assert_eq!(output.kind, TerminalOutputKind::ProviderOutput);
        assert_eq!(output.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(output.merge_key.as_deref(), Some("part-1"));
        assert_eq!(output.recipient_attachment_ids.len(), 2);
        assert_eq!(output.pending_recipient_attachment_ids.len(), 2);
        assert_eq!(notice.provider_run_id.as_deref(), Some("provider-run-1"));
        assert_eq!(notice.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(notice.recipient_attachment_ids.len(), 1);
        assert_eq!(notice.pending_recipient_attachment_ids.len(), 1);
    }

    #[test]
    fn output_polling_is_per_recipient() {
        let mut terminal = TerminalStreamService::new();
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::PromptEcho,
            None,
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
    fn output_backlog_is_bounded_per_slow_recipient() {
        let mut terminal =
            TerminalStreamService::with_pending_output_record_limit_per_attachment(2);
        for index in 0..4 {
            terminal.fan_out_output(
                "session-1",
                "provider-run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                None,
                vec!["slow-attachment".to_string(), "fast-attachment".to_string()],
                format!("chunk-{index}").as_bytes(),
            );
        }

        let slow_records = terminal.drain_output_records("session-1", "slow-attachment");
        assert_eq!(slow_records.len(), 2);
        assert_eq!(slow_records[0].bytes, b"chunk-2");
        assert_eq!(slow_records[1].bytes, b"chunk-3");

        let fast_records = terminal.drain_output_records("session-1", "fast-attachment");
        assert_eq!(fast_records.len(), 2);
        assert_eq!(fast_records[0].bytes, b"chunk-2");
        assert_eq!(fast_records[1].bytes, b"chunk-3");
        assert!(terminal.output_records().is_empty());
    }

    #[test]
    fn health_reports_output_backlog_pressure() {
        let mut terminal =
            TerminalStreamService::with_pending_output_record_limit_per_attachment(2);
        for index in 0..4 {
            terminal.fan_out_output(
                "session-1",
                "provider-run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                None,
                vec!["slow-attachment".to_string()],
                format!("chunk-{index}").as_bytes(),
            );
        }
        terminal.record_notice(
            "session-1",
            None,
            None,
            vec!["attachment-1".to_string()],
            "n",
        );
        terminal.record_assistant_message_completion(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            vec!["attachment-1".to_string()],
            "message-1",
            42,
        );

        let health = terminal.health_snapshot();
        assert_eq!(health.pending_output_records, 2);
        assert_eq!(health.pending_notice_records, 1);
        assert_eq!(health.pending_completion_records, 1);
        assert_eq!(health.pending_output_record_limit_per_attachment, 2);
        assert_eq!(health.trimmed_pending_output_recipients, 2);
    }

    #[test]
    fn notice_polling_is_per_recipient() {
        let mut terminal = TerminalStreamService::new();
        terminal.record_notice(
            "session-1",
            None,
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

    #[test]
    fn completion_polling_is_per_recipient() {
        let mut terminal = TerminalStreamService::new();
        terminal.record_assistant_message_completion(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            "message-1",
            42,
        );

        let first = terminal.drain_completion_records("session-1", "attachment-1");
        assert_eq!(first.len(), 1);

        let second = terminal.drain_completion_records("session-1", "attachment-2");
        assert_eq!(second.len(), 1);

        let none_left = terminal.drain_completion_records("session-1", "attachment-2");
        assert!(none_left.is_empty());
    }
}

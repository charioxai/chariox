use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

const DEFAULT_PENDING_OUTPUT_RECORD_LIMIT_PER_ATTACHMENT: usize = 4096;
const DEFAULT_OUTPUT_COALESCE_BYTE_LIMIT: usize = 16 * 1024;
const DEFAULT_OUTPUT_DRAIN_JSON_LIMIT: usize = 128 * 1024;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_attachment_id: Option<String>,
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
pub struct TerminalStreamHealthStore {
    snapshot: Arc<StdMutex<TerminalStreamHealthSnapshot>>,
}

impl TerminalStreamHealthStore {
    pub fn snapshot(&self) -> TerminalStreamHealthSnapshot {
        self.snapshot
            .lock()
            .expect("terminal stream health lock should not be poisoned")
            .clone()
    }

    fn update(&self, snapshot: TerminalStreamHealthSnapshot) {
        *self
            .snapshot
            .lock()
            .expect("terminal stream health lock should not be poisoned") = snapshot;
    }
}

#[derive(Debug, Clone, Default)]
pub struct TerminalStreamStore {
    inner: Arc<StdMutex<TerminalStreamService>>,
    changes: Arc<TerminalStreamChangeSignal>,
}

#[derive(Debug, Default)]
struct TerminalStreamChangeSignal {
    sequence: AtomicU64,
    notify: Notify,
}

impl TerminalStreamStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(StdMutex::new(TerminalStreamService::new())),
            changes: Arc::new(TerminalStreamChangeSignal::default()),
        }
    }

    pub fn change_sequence(&self) -> u64 {
        self.changes.sequence.load(Ordering::Acquire)
    }

    pub async fn wait_for_change_after(&self, sequence: u64) {
        if self.change_sequence() != sequence {
            return;
        }
        let notified = self.changes.notify.notified();
        if self.change_sequence() != sequence {
            return;
        }
        notified.await;
    }

    pub fn health_store(&self) -> TerminalStreamHealthStore {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .health_store()
    }

    pub fn record_input(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        bytes: &[u8],
    ) {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .record_input(session_id, provider_run_id, source_attachment_id, bytes);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = self
            .inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .fan_out_output(
                session_id,
                provider_run_id,
                agent_id,
                kind,
                merge_key,
                recipient_attachment_ids,
                bytes,
            );
        self.record_change();
        record
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_prompt_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        prompt_id: &str,
        source_attachment_id: &str,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = self
            .inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .fan_out_prompt_output(
                session_id,
                provider_run_id,
                agent_id,
                prompt_id,
                source_attachment_id,
                recipient_attachment_ids,
                bytes,
            );
        self.record_change();
        record
    }

    pub fn record_notice(
        &self,
        session_id: &str,
        provider_run_id: Option<&str>,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> RuntimeNoticeRecord {
        let record = self
            .inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .record_notice(
                session_id,
                provider_run_id,
                agent_id,
                recipient_attachment_ids,
                message,
            );
        self.record_change();
        record
    }

    pub fn input_records(&self) -> Vec<TerminalInputRecord> {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .input_records()
            .to_vec()
    }

    pub fn output_records(&self) -> Vec<TerminalOutputRecord> {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .output_records()
            .to_vec()
    }

    pub fn notice_records(&self) -> Vec<RuntimeNoticeRecord> {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .notice_records()
            .to_vec()
    }

    pub fn health_snapshot(&self) -> TerminalStreamHealthSnapshot {
        self.health_store().snapshot()
    }

    pub fn drain_output_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<TerminalOutputRecord> {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .drain_output_records(session_id, attachment_id)
    }

    pub fn record_assistant_message_completion(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) -> AssistantMessageCompletionRecord {
        let record = self
            .inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .record_assistant_message_completion(
                session_id,
                provider_run_id,
                agent_id,
                recipient_attachment_ids,
                message_id,
                completed_at_ms,
            );
        self.record_change();
        record
    }

    pub fn drain_completion_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<AssistantMessageCompletionRecord> {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .drain_completion_records(session_id, attachment_id)
    }

    pub fn drain_notice_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<RuntimeNoticeRecord> {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .drain_notice_records(session_id, attachment_id)
    }

    pub fn remove_session(&self, session_id: &str) {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .remove_session(session_id);
        self.record_change();
    }

    pub fn remove_attachment(&self, session_id: &str, attachment_id: &str) {
        let changed = self
            .inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .remove_attachment(session_id, attachment_id);
        if changed {
            self.record_change();
        }
    }

    pub fn notify_terminal_projection_change(&self) {
        self.record_change();
    }

    fn record_change(&self) {
        self.changes.sequence.fetch_add(1, Ordering::AcqRel);
        self.changes.notify.notify_waiters();
    }
}

#[derive(Debug, Clone, Default)]
pub struct TerminalStreamService {
    input_records: Vec<TerminalInputRecord>,
    output_records: Vec<TerminalOutputRecord>,
    notice_records: Vec<RuntimeNoticeRecord>,
    completion_records: Vec<AssistantMessageCompletionRecord>,
    pending_output_record_limit_per_attachment: usize,
    output_coalesce_byte_limit: usize,
    output_drain_json_limit: usize,
    trimmed_pending_output_recipients: u64,
    health_store: TerminalStreamHealthStore,
}

impl TerminalStreamService {
    pub fn new() -> Self {
        let service = Self {
            pending_output_record_limit_per_attachment:
                DEFAULT_PENDING_OUTPUT_RECORD_LIMIT_PER_ATTACHMENT,
            output_coalesce_byte_limit: DEFAULT_OUTPUT_COALESCE_BYTE_LIMIT,
            output_drain_json_limit: DEFAULT_OUTPUT_DRAIN_JSON_LIMIT,
            ..Self::default()
        };
        service.refresh_health();
        service
    }

    #[cfg(test)]
    fn with_pending_output_record_limit_per_attachment(limit: usize) -> Self {
        let service = Self {
            pending_output_record_limit_per_attachment: limit,
            ..Self::new()
        };
        service.refresh_health();
        service
    }

    #[cfg(test)]
    fn with_output_coalesce_byte_limit(limit: usize) -> Self {
        let service = Self {
            output_coalesce_byte_limit: limit,
            ..Self::new()
        };
        service.refresh_health();
        service
    }

    #[cfg(test)]
    fn with_output_drain_json_limit(limit: usize) -> Self {
        let service = Self {
            output_drain_json_limit: limit,
            ..Self::new()
        };
        service.refresh_health();
        service
    }

    pub fn health_store(&self) -> TerminalStreamHealthStore {
        self.health_store.clone()
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
            prompt_id: None,
            source_attachment_id: None,
            kind,
            merge_key,
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            bytes: bytes.to_vec(),
        };

        if !self.try_coalesce_output_record(&record) {
            self.output_records.push(record.clone());
        }
        self.enforce_pending_output_record_limits();
        self.refresh_health();
        record
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_prompt_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        prompt_id: &str,
        source_attachment_id: &str,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = TerminalOutputRecord {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.map(str::to_string),
            prompt_id: Some(prompt_id.to_string()),
            source_attachment_id: Some(source_attachment_id.to_string()),
            kind: TerminalOutputKind::PromptEcho,
            merge_key: None,
            pending_recipient_attachment_ids: recipient_attachment_ids.clone(),
            recipient_attachment_ids,
            bytes: bytes.to_vec(),
        };

        if !self.try_coalesce_output_record(&record) {
            self.output_records.push(record.clone());
        }
        self.enforce_pending_output_record_limits();
        self.refresh_health();
        record
    }

    fn try_coalesce_output_record(&mut self, record: &TerminalOutputRecord) -> bool {
        if self.output_coalesce_byte_limit == 0 || !is_coalescible_output_kind(&record.kind) {
            return false;
        }
        let Some(previous) = self.output_records.last_mut() else {
            return false;
        };
        if !is_coalescible_output_kind(&previous.kind)
            || previous.session_id != record.session_id
            || previous.provider_run_id != record.provider_run_id
            || previous.agent_id != record.agent_id
            || previous.kind != record.kind
            || previous.merge_key != record.merge_key
            || previous.recipient_attachment_ids != record.recipient_attachment_ids
            || previous.pending_recipient_attachment_ids != record.pending_recipient_attachment_ids
        {
            return false;
        }
        let Some(coalesced_len) = previous.bytes.len().checked_add(record.bytes.len()) else {
            return false;
        };
        if coalesced_len > self.output_coalesce_byte_limit {
            return false;
        }
        previous.bytes.extend_from_slice(&record.bytes);
        true
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
        self.refresh_health();
        record
    }

    pub fn input_records(&self) -> &[TerminalInputRecord] {
        &self.input_records
    }

    pub fn output_records(&self) -> &[TerminalOutputRecord] {
        &self.output_records
    }

    pub fn health_snapshot(&self) -> TerminalStreamHealthSnapshot {
        self.health_store.snapshot()
    }

    fn current_health_snapshot(&self) -> TerminalStreamHealthSnapshot {
        TerminalStreamHealthSnapshot {
            pending_output_records: self.output_records.len(),
            pending_notice_records: self.notice_records.len(),
            pending_completion_records: self.completion_records.len(),
            pending_output_record_limit_per_attachment: self
                .pending_output_record_limit_per_attachment,
            trimmed_pending_output_recipients: self.trimmed_pending_output_recipients,
        }
    }

    fn refresh_health(&self) {
        self.health_store.update(self.current_health_snapshot());
    }

    pub fn drain_output_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<TerminalOutputRecord> {
        let mut drained = Vec::new();
        let mut drained_json_bytes: usize = 2;
        for record in &mut self.output_records {
            if record.session_id == session_id
                && record
                    .pending_recipient_attachment_ids
                    .iter()
                    .any(|id| id == attachment_id)
            {
                let scoped = scoped_output_record(record, attachment_id);
                let scoped_json_bytes = terminal_output_record_json_bytes(&scoped);
                let candidate_json_bytes = if drained.is_empty() {
                    2_usize.saturating_add(scoped_json_bytes)
                } else {
                    drained_json_bytes
                        .saturating_add(1)
                        .saturating_add(scoped_json_bytes)
                };
                if !drained.is_empty() && candidate_json_bytes > self.output_drain_json_limit {
                    break;
                }
                drained_json_bytes = candidate_json_bytes;
                drained.push(scoped);
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
            }
        }

        self.output_records
            .retain(|record| !record.pending_recipient_attachment_ids.is_empty());
        self.refresh_health();
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

        if self.output_records.len() <= self.pending_output_record_limit_per_attachment {
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
        self.refresh_health();
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
                drained.push(scoped_completion_record(record, attachment_id));
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
            }
        }

        self.completion_records
            .retain(|record| !record.pending_recipient_attachment_ids.is_empty());
        self.refresh_health();
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
                drained.push(scoped_notice_record(record, attachment_id));
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
        self.refresh_health();
        drained
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.input_records
            .retain(|record| record.session_id != session_id);
        self.output_records
            .retain(|record| record.session_id != session_id);
        self.notice_records
            .retain(|record| record.session_id != session_id);
        self.completion_records
            .retain(|record| record.session_id != session_id);
        self.refresh_health();
    }

    pub fn remove_attachment(&mut self, session_id: &str, attachment_id: &str) -> bool {
        let mut changed = false;
        for record in &mut self.output_records {
            if record.session_id == session_id {
                let previous_len = record.pending_recipient_attachment_ids.len();
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
                changed |= record.pending_recipient_attachment_ids.len() != previous_len;
            }
        }
        for record in &mut self.notice_records {
            if record.session_id == session_id {
                let previous_len = record.pending_recipient_attachment_ids.len();
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
                changed |= record.pending_recipient_attachment_ids.len() != previous_len;
            }
        }
        for record in &mut self.completion_records {
            if record.session_id == session_id {
                let previous_len = record.pending_recipient_attachment_ids.len();
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
                changed |= record.pending_recipient_attachment_ids.len() != previous_len;
            }
        }
        self.output_records
            .retain(|record| !record.pending_recipient_attachment_ids.is_empty());
        self.notice_records.retain(|record| {
            if record.recipient_attachment_ids.is_empty() {
                false
            } else {
                !record.pending_recipient_attachment_ids.is_empty()
            }
        });
        self.completion_records
            .retain(|record| !record.pending_recipient_attachment_ids.is_empty());
        if changed {
            self.refresh_health();
        }
        changed
    }
}

fn is_coalescible_output_kind(kind: &TerminalOutputKind) -> bool {
    matches!(
        kind,
        TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
    )
}

fn scoped_output_record(
    record: &TerminalOutputRecord,
    attachment_id: &str,
) -> TerminalOutputRecord {
    let mut scoped = record.clone();
    scoped.recipient_attachment_ids = vec![attachment_id.to_string()];
    scoped.pending_recipient_attachment_ids = vec![attachment_id.to_string()];
    scoped
}

fn scoped_notice_record(record: &RuntimeNoticeRecord, attachment_id: &str) -> RuntimeNoticeRecord {
    let mut scoped = record.clone();
    if !scoped.recipient_attachment_ids.is_empty() {
        scoped.recipient_attachment_ids = vec![attachment_id.to_string()];
    }
    scoped.pending_recipient_attachment_ids = vec![attachment_id.to_string()];
    scoped
}

fn scoped_completion_record(
    record: &AssistantMessageCompletionRecord,
    attachment_id: &str,
) -> AssistantMessageCompletionRecord {
    let mut scoped = record.clone();
    scoped.recipient_attachment_ids = vec![attachment_id.to_string()];
    scoped.pending_recipient_attachment_ids = vec![attachment_id.to_string()];
    scoped
}

fn terminal_output_record_json_bytes(record: &TerminalOutputRecord) -> usize {
    serde_json::to_vec(record)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::{TerminalOutputKind, TerminalStreamService, TerminalStreamStore};

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
        assert_eq!(first[0].recipient_attachment_ids, vec!["attachment-1"]);
        assert_eq!(
            first[0].pending_recipient_attachment_ids,
            vec!["attachment-1"]
        );
        assert_eq!(terminal.output_records().len(), 1);

        let second = terminal.drain_output_records("session-1", "attachment-2");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].recipient_attachment_ids, vec!["attachment-2"]);
        assert_eq!(
            second[0].pending_recipient_attachment_ids,
            vec!["attachment-2"]
        );
        assert!(terminal.output_records().is_empty());
    }

    #[test]
    fn prompt_output_records_carry_prompt_identity() {
        let mut terminal = TerminalStreamService::new();
        let output = terminal.fan_out_prompt_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            "prompt-42",
            "attachment-1",
            vec!["attachment-2".to_string()],
            b"hello\n",
        );

        assert_eq!(output.kind, TerminalOutputKind::PromptEcho);
        assert_eq!(output.prompt_id.as_deref(), Some("prompt-42"));
        assert_eq!(output.source_attachment_id.as_deref(), Some("attachment-1"));
        let drained = terminal.drain_output_records("session-1", "attachment-2");
        assert_eq!(drained[0].prompt_id.as_deref(), Some("prompt-42"));
        assert_eq!(
            drained[0].source_attachment_id.as_deref(),
            Some("attachment-1")
        );
    }

    #[test]
    fn output_polling_keeps_large_drains_batched() {
        let mut terminal = TerminalStreamService::with_output_drain_json_limit(256);
        for index in 0..4 {
            terminal.fan_out_output(
                "session-1",
                "provider-run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                Some(format!("chunk-{index}")),
                vec!["attachment-1".to_string()],
                b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
            );
        }

        let first = terminal.drain_output_records("session-1", "attachment-1");
        assert!(!first.is_empty());
        assert!(first.len() < 4);
        assert_eq!(first[0].merge_key.as_deref(), Some("chunk-0"));
        assert!(!terminal.output_records().is_empty());

        let second = terminal.drain_output_records("session-1", "attachment-1");
        assert!(!second.is_empty());
        let expected_next_chunk = format!("chunk-{}", first.len());
        assert_eq!(
            second[0].merge_key.as_deref(),
            Some(expected_next_chunk.as_str())
        );
        assert!(terminal.output_records().len() < 4);
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
                Some(format!("chunk-{index}")),
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
                Some(format!("chunk-{index}")),
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
    fn adjacent_provider_output_records_coalesce() {
        let mut terminal = TerminalStreamService::new();
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            b"hello ",
        );
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            b"world",
        );

        assert_eq!(terminal.output_records().len(), 1);
        assert_eq!(terminal.output_records()[0].bytes, b"hello world");

        let first = terminal.drain_output_records("session-1", "attachment-1");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].bytes, b"hello world");
    }

    #[test]
    fn output_coalescing_preserves_recipient_progress() {
        let mut terminal = TerminalStreamService::new();
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["slow-attachment".to_string(), "fast-attachment".to_string()],
            b"first",
        );
        let fast_first = terminal.drain_output_records("session-1", "fast-attachment");
        assert_eq!(fast_first.len(), 1);
        assert_eq!(fast_first[0].bytes, b"first");

        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["slow-attachment".to_string(), "fast-attachment".to_string()],
            b"second",
        );

        let fast_second = terminal.drain_output_records("session-1", "fast-attachment");
        assert_eq!(fast_second.len(), 1);
        assert_eq!(fast_second[0].bytes, b"second");

        let slow_records = terminal.drain_output_records("session-1", "slow-attachment");
        assert_eq!(slow_records.len(), 2);
        assert_eq!(slow_records[0].bytes, b"first");
        assert_eq!(slow_records[1].bytes, b"second");
    }

    #[test]
    fn output_coalescing_respects_byte_limit() {
        let mut terminal = TerminalStreamService::with_output_coalesce_byte_limit(5);
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string()],
            b"1234",
        );
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string()],
            b"56",
        );

        assert_eq!(terminal.output_records().len(), 2);
        assert_eq!(terminal.output_records()[0].bytes, b"1234");
        assert_eq!(terminal.output_records()[1].bytes, b"56");
    }

    #[test]
    fn cloned_health_store_tracks_terminal_stream_mutations() {
        let mut terminal =
            TerminalStreamService::with_pending_output_record_limit_per_attachment(2);
        let health_store = terminal.health_store();

        assert_eq!(health_store.snapshot().pending_output_records, 0);

        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string()],
            b"chunk-1",
        );
        assert_eq!(health_store.snapshot().pending_output_records, 1);

        terminal.record_notice(
            "session-1",
            None,
            None,
            vec!["attachment-1".to_string()],
            "notice",
        );
        assert_eq!(health_store.snapshot().pending_notice_records, 1);

        terminal.record_assistant_message_completion(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            vec!["attachment-1".to_string()],
            "message-1",
            42,
        );
        assert_eq!(health_store.snapshot().pending_completion_records, 1);

        let output = terminal.drain_output_records("session-1", "attachment-1");
        assert_eq!(output.len(), 1);
        assert_eq!(health_store.snapshot().pending_output_records, 0);

        let notices = terminal.drain_notice_records("session-1", "attachment-1");
        assert_eq!(notices.len(), 1);
        assert_eq!(health_store.snapshot().pending_notice_records, 0);

        let completions = terminal.drain_completion_records("session-1", "attachment-1");
        assert_eq!(completions.len(), 1);
        assert_eq!(health_store.snapshot().pending_completion_records, 0);
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
        assert_eq!(first[0].recipient_attachment_ids, vec!["attachment-1"]);
        assert_eq!(terminal.notice_records().len(), 1);

        let second = terminal.drain_notice_records("session-1", "attachment-2");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].recipient_attachment_ids, vec!["attachment-2"]);
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
        assert_eq!(first[0].recipient_attachment_ids, vec!["attachment-1"]);

        let second = terminal.drain_completion_records("session-1", "attachment-2");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].recipient_attachment_ids, vec!["attachment-2"]);

        let none_left = terminal.drain_completion_records("session-1", "attachment-2");
        assert!(none_left.is_empty());
    }

    #[test]
    fn removing_attachment_prunes_pending_terminal_records() {
        let mut terminal = TerminalStreamService::new();
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            None,
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            b"output",
        );
        terminal.record_notice(
            "session-1",
            None,
            None,
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            "notice",
        );
        terminal.record_assistant_message_completion(
            "session-1",
            "provider-run-1",
            None,
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            "message-1",
            1,
        );

        assert!(terminal.remove_attachment("session-1", "attachment-1"));

        assert_eq!(
            terminal.output_records()[0].pending_recipient_attachment_ids,
            vec!["attachment-2".to_string()],
        );
        assert_eq!(
            terminal.notice_records()[0].pending_recipient_attachment_ids,
            vec!["attachment-2".to_string()],
        );
        assert_eq!(
            terminal.completion_records[0].pending_recipient_attachment_ids,
            vec!["attachment-2".to_string()],
        );

        assert!(terminal.remove_attachment("session-1", "attachment-2"));
        assert!(terminal.output_records().is_empty());
        assert!(terminal.notice_records().is_empty());
        assert!(terminal.completion_records.is_empty());
    }

    #[test]
    fn removes_all_records_for_session() {
        let mut terminal = TerminalStreamService::new();
        terminal.record_input("session-1", "provider-run-1", "attachment-1", b"one");
        terminal.record_input("session-2", "provider-run-2", "attachment-2", b"two");
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            None,
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string()],
            b"output",
        );
        terminal.record_notice(
            "session-1",
            None,
            None,
            vec!["attachment-1".to_string()],
            "notice",
        );
        terminal.record_assistant_message_completion(
            "session-1",
            "provider-run-1",
            None,
            vec!["attachment-1".to_string()],
            "message-1",
            1,
        );

        terminal.remove_session("session-1");

        assert_eq!(terminal.input_records().len(), 1);
        assert_eq!(terminal.input_records()[0].session_id, "session-2");
        assert!(terminal.output_records().is_empty());
        assert!(terminal.notice_records().is_empty());
        assert_eq!(terminal.health_snapshot().pending_output_records, 0);
        assert_eq!(terminal.health_snapshot().pending_notice_records, 0);
        assert_eq!(terminal.health_snapshot().pending_completion_records, 0);
    }

    #[tokio::test]
    async fn terminal_stream_store_notifies_waiters_on_output() {
        let terminal = TerminalStreamStore::new();
        let sequence = terminal.change_sequence();
        let waiter = {
            let terminal = terminal.clone();
            tokio::spawn(async move {
                terminal.wait_for_change_after(sequence).await;
            })
        };

        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            None,
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string()],
            b"output",
        );

        tokio::time::timeout(std::time::Duration::from_millis(100), waiter)
            .await
            .expect("terminal stream waiter should wake")
            .expect("terminal stream waiter task should complete");
        assert!(terminal.change_sequence() > sequence);
    }

    #[tokio::test]
    async fn terminal_stream_store_wait_returns_when_sequence_already_changed() {
        let terminal = TerminalStreamStore::new();
        let sequence = terminal.change_sequence();
        terminal.record_notice("session-1", None, None, Vec::new(), "notice");

        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            terminal.wait_for_change_after(sequence),
        )
        .await
        .expect("changed terminal sequence should not block");
    }
}

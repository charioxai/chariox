use std::collections::{BTreeMap, BTreeSet, VecDeque};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOutputAppend {
    pub session_id: String,
    pub provider_run_id: String,
    pub agent_id: Option<String>,
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub recipient_attachment_ids: Vec<String>,
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
    attachment_changes: Arc<StdMutex<BTreeMap<(String, String), Arc<TerminalStreamChangeSignal>>>>,
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
            attachment_changes: Arc::new(StdMutex::new(BTreeMap::new())),
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

    pub fn attachment_change_sequence(&self, session_id: &str, attachment_id: &str) -> u64 {
        self.attachment_signal(session_id, attachment_id)
            .sequence
            .load(Ordering::Acquire)
    }

    pub async fn wait_for_attachment_change_after(
        &self,
        session_id: &str,
        attachment_id: &str,
        sequence: u64,
    ) {
        let signal = self.attachment_signal(session_id, attachment_id);
        if signal.sequence.load(Ordering::Acquire) != sequence {
            return;
        }
        let notified = signal.notify.notified();
        if signal.sequence.load(Ordering::Acquire) != sequence {
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
        self.record_change_for_record(&record);
        record
    }

    pub fn fan_out_outputs(&self, outputs: Vec<TerminalOutputAppend>) -> Vec<TerminalOutputRecord> {
        if outputs.is_empty() {
            return Vec::new();
        }
        let records = self
            .inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .fan_out_outputs(outputs);
        if !records.is_empty() {
            self.record_change_for_records(&records);
        }
        records
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
        self.record_change_for_record(&record);
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
        self.record_change_for_notice(&record);
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
    }

    pub fn notice_records(&self) -> Vec<RuntimeNoticeRecord> {
        self.inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .notice_records()
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
        self.record_change_for_completion(&record);
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
        self.attachment_changes
            .lock()
            .expect("terminal attachment change lock should not be poisoned")
            .retain(|(signal_session_id, _), _| signal_session_id != session_id);
        self.record_change();
    }

    pub fn remove_attachment(&self, session_id: &str, attachment_id: &str) {
        let changed = self
            .inner
            .lock()
            .expect("terminal stream lock should not be poisoned")
            .remove_attachment(session_id, attachment_id);
        if changed {
            self.attachment_changes
                .lock()
                .expect("terminal attachment change lock should not be poisoned")
                .remove(&(session_id.to_string(), attachment_id.to_string()));
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

    fn record_change_for_record(&self, record: &TerminalOutputRecord) {
        self.record_change_for_attachment_ids(&record.session_id, &record.recipient_attachment_ids);
    }

    fn record_change_for_records(&self, records: &[TerminalOutputRecord]) {
        let mut keys = BTreeSet::new();
        for record in records {
            for attachment_id in &record.recipient_attachment_ids {
                keys.insert((record.session_id.clone(), attachment_id.clone()));
            }
        }
        self.record_change_for_keys(keys);
    }

    fn record_change_for_notice(&self, record: &RuntimeNoticeRecord) {
        self.record_change_for_attachment_ids(&record.session_id, &record.recipient_attachment_ids);
    }

    fn record_change_for_completion(&self, record: &AssistantMessageCompletionRecord) {
        self.record_change_for_attachment_ids(&record.session_id, &record.recipient_attachment_ids);
    }

    fn record_change_for_attachment_ids(&self, session_id: &str, attachment_ids: &[String]) {
        let keys = attachment_ids
            .iter()
            .map(|attachment_id| (session_id.to_string(), attachment_id.clone()))
            .collect::<BTreeSet<_>>();
        self.record_change_for_keys(keys);
    }

    fn record_change_for_keys(&self, keys: BTreeSet<(String, String)>) {
        if keys.is_empty() {
            self.record_change();
            return;
        }
        for (session_id, attachment_id) in keys {
            let signal = self.attachment_signal(&session_id, &attachment_id);
            signal.sequence.fetch_add(1, Ordering::AcqRel);
            signal.notify.notify_waiters();
        }
    }

    fn attachment_signal(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Arc<TerminalStreamChangeSignal> {
        self.attachment_changes
            .lock()
            .expect("terminal attachment change lock should not be poisoned")
            .entry((session_id.to_string(), attachment_id.to_string()))
            .or_default()
            .clone()
    }
}

#[derive(Debug, Clone, Default)]
pub struct TerminalStreamService {
    input_records: Vec<TerminalInputRecord>,
    output_records: BTreeMap<u64, TerminalOutputRecord>,
    pending_output_by_attachment: BTreeMap<(String, String), VecDeque<u64>>,
    next_output_record_id: u64,
    last_output_record_id: Option<u64>,
    notice_records: BTreeMap<u64, RuntimeNoticeRecord>,
    pending_notice_by_attachment: BTreeMap<(String, String), VecDeque<u64>>,
    pending_broadcast_notice_by_session: BTreeMap<String, VecDeque<u64>>,
    next_notice_record_id: u64,
    completion_records: BTreeMap<u64, AssistantMessageCompletionRecord>,
    pending_completion_by_attachment: BTreeMap<(String, String), VecDeque<u64>>,
    next_completion_record_id: u64,
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
            self.push_output_record(record.clone());
        }
        self.enforce_pending_output_record_limits();
        self.refresh_health();
        record
    }

    pub fn fan_out_outputs(
        &mut self,
        outputs: Vec<TerminalOutputAppend>,
    ) -> Vec<TerminalOutputRecord> {
        if outputs.is_empty() {
            return Vec::new();
        }
        let mut records = Vec::with_capacity(outputs.len());
        for output in outputs {
            let record = TerminalOutputRecord {
                session_id: output.session_id,
                provider_run_id: output.provider_run_id,
                agent_id: output.agent_id,
                prompt_id: None,
                source_attachment_id: None,
                kind: output.kind,
                merge_key: output.merge_key,
                pending_recipient_attachment_ids: output.recipient_attachment_ids.clone(),
                recipient_attachment_ids: output.recipient_attachment_ids,
                bytes: output.bytes,
            };

            if !self.try_coalesce_output_record(&record) {
                self.push_output_record(record.clone());
            }
            records.push(record);
        }
        self.enforce_pending_output_record_limits();
        self.refresh_health();
        records
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
            self.push_output_record(record.clone());
        }
        self.enforce_pending_output_record_limits();
        self.refresh_health();
        record
    }

    fn push_output_record(&mut self, record: TerminalOutputRecord) -> u64 {
        let record_id = self.next_output_record_id;
        self.next_output_record_id = self.next_output_record_id.saturating_add(1);
        for attachment_id in &record.pending_recipient_attachment_ids {
            self.pending_output_by_attachment
                .entry((record.session_id.clone(), attachment_id.clone()))
                .or_default()
                .push_back(record_id);
        }
        self.output_records.insert(record_id, record);
        self.last_output_record_id = Some(record_id);
        record_id
    }

    fn try_coalesce_output_record(&mut self, record: &TerminalOutputRecord) -> bool {
        if self.output_coalesce_byte_limit == 0 || !is_coalescible_output_kind(&record.kind) {
            return false;
        }
        let Some(previous_id) = self.last_output_record_id else {
            return false;
        };
        let Some(previous) = self.output_records.get_mut(&previous_id) else {
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

        self.push_notice_record(record.clone());
        self.refresh_health();
        record
    }

    fn push_notice_record(&mut self, record: RuntimeNoticeRecord) -> u64 {
        let record_id = self.next_notice_record_id;
        self.next_notice_record_id = self.next_notice_record_id.saturating_add(1);
        if record.recipient_attachment_ids.is_empty()
            && record.pending_recipient_attachment_ids.is_empty()
        {
            self.pending_broadcast_notice_by_session
                .entry(record.session_id.clone())
                .or_default()
                .push_back(record_id);
        } else {
            for attachment_id in &record.pending_recipient_attachment_ids {
                self.pending_notice_by_attachment
                    .entry((record.session_id.clone(), attachment_id.clone()))
                    .or_default()
                    .push_back(record_id);
            }
        }
        self.notice_records.insert(record_id, record);
        record_id
    }

    pub fn input_records(&self) -> &[TerminalInputRecord] {
        &self.input_records
    }

    pub fn output_records(&self) -> Vec<TerminalOutputRecord> {
        self.output_records.values().cloned().collect()
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
        let key = (session_id.to_string(), attachment_id.to_string());
        loop {
            let Some(record_id) = self
                .pending_output_by_attachment
                .get(&key)
                .and_then(|queue| queue.front().copied())
            else {
                break;
            };
            let Some(record) = self.output_records.get(&record_id) else {
                self.pop_pending_output_queue_front(&key);
                continue;
            };
            let scoped_json_bytes = terminal_output_record_scoped_json_bytes(record, attachment_id);
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
            let scoped = scoped_output_record(record, attachment_id);
            self.pop_pending_output_queue_front(&key);
            drained_json_bytes = candidate_json_bytes;
            drained.push(scoped);
            if let Some(record) = self.output_records.get_mut(&record_id) {
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
            }
            self.remove_output_record_if_drained(record_id);
        }

        self.refresh_health();
        drained
    }

    fn pop_pending_output_queue_front(&mut self, key: &(String, String)) {
        let empty = match self.pending_output_by_attachment.get_mut(key) {
            Some(queue) => {
                queue.pop_front();
                queue.is_empty()
            }
            None => false,
        };
        if empty {
            self.pending_output_by_attachment.remove(key);
        }
    }

    fn remove_output_record_if_drained(&mut self, record_id: u64) {
        let should_remove = self
            .output_records
            .get(&record_id)
            .is_some_and(|record| record.pending_recipient_attachment_ids.is_empty());
        if should_remove {
            self.output_records.remove(&record_id);
            if self.last_output_record_id == Some(record_id) {
                self.last_output_record_id = self.output_records.keys().next_back().copied();
            }
        }
    }

    fn enforce_pending_output_record_limits(&mut self) {
        if self.pending_output_record_limit_per_attachment == 0 {
            let trimmed = self
                .pending_output_by_attachment
                .values()
                .map(|queue| queue.len() as u64)
                .sum::<u64>();
            self.trimmed_pending_output_recipients = self
                .trimmed_pending_output_recipients
                .saturating_add(trimmed);
            self.output_records.clear();
            self.pending_output_by_attachment.clear();
            self.last_output_record_id = None;
            return;
        }

        let mut trimmed = 0_u64;
        let keys = self
            .pending_output_by_attachment
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            loop {
                let should_trim =
                    self.pending_output_by_attachment
                        .get(&key)
                        .is_some_and(|queue| {
                            queue.len() > self.pending_output_record_limit_per_attachment
                        });
                if !should_trim {
                    break;
                }
                let Some(record_id) = self
                    .pending_output_by_attachment
                    .get_mut(&key)
                    .and_then(|queue| queue.pop_front())
                else {
                    break;
                };
                trimmed = trimmed.saturating_add(1);
                if let Some(record) = self.output_records.get_mut(&record_id) {
                    record
                        .pending_recipient_attachment_ids
                        .retain(|id| id != &key.1);
                }
                self.remove_output_record_if_drained(record_id);
            }
        }
        self.pending_output_by_attachment
            .retain(|_, queue| !queue.is_empty());
        self.trimmed_pending_output_recipients = self
            .trimmed_pending_output_recipients
            .saturating_add(trimmed);
    }

    pub fn notice_records(&self) -> Vec<RuntimeNoticeRecord> {
        self.notice_records.values().cloned().collect()
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

        self.push_completion_record(record.clone());
        self.refresh_health();
        record
    }

    fn push_completion_record(&mut self, record: AssistantMessageCompletionRecord) -> u64 {
        let record_id = self.next_completion_record_id;
        self.next_completion_record_id = self.next_completion_record_id.saturating_add(1);
        for attachment_id in &record.pending_recipient_attachment_ids {
            self.pending_completion_by_attachment
                .entry((record.session_id.clone(), attachment_id.clone()))
                .or_default()
                .push_back(record_id);
        }
        self.completion_records.insert(record_id, record);
        record_id
    }

    pub fn drain_completion_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<AssistantMessageCompletionRecord> {
        let mut drained = Vec::new();
        let key = (session_id.to_string(), attachment_id.to_string());
        let completion_ids = self
            .pending_completion_by_attachment
            .remove(&key)
            .unwrap_or_default();
        for record_id in completion_ids {
            let should_drain = self
                .completion_records
                .get(&record_id)
                .is_some_and(|record| {
                    record.session_id == session_id
                        && record
                            .pending_recipient_attachment_ids
                            .iter()
                            .any(|id| id == attachment_id)
                });
            if !should_drain {
                continue;
            }
            if let Some(record) = self.completion_records.get(&record_id) {
                drained.push(scoped_completion_record(record, attachment_id));
            }
            if let Some(record) = self.completion_records.get_mut(&record_id) {
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
            }
            self.remove_completion_record_if_drained(record_id);
        }

        self.refresh_health();
        drained
    }

    fn remove_completion_record_if_drained(&mut self, record_id: u64) {
        let should_remove = self
            .completion_records
            .get(&record_id)
            .is_some_and(|record| record.pending_recipient_attachment_ids.is_empty());
        if should_remove {
            self.completion_records.remove(&record_id);
        }
    }

    pub fn drain_notice_records(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<RuntimeNoticeRecord> {
        let mut drained = Vec::new();
        let key = (session_id.to_string(), attachment_id.to_string());
        let mut notice_ids = self
            .pending_notice_by_attachment
            .get(&key)
            .map(|queue| queue.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        notice_ids.extend(
            self.pending_broadcast_notice_by_session
                .remove(session_id)
                .unwrap_or_default(),
        );
        notice_ids.sort_unstable();
        notice_ids.dedup();

        for record_id in notice_ids {
            let should_drain = self.notice_records.get(&record_id).is_some_and(|record| {
                record.session_id == session_id
                    && (record.pending_recipient_attachment_ids.is_empty()
                        || record
                            .pending_recipient_attachment_ids
                            .iter()
                            .any(|id| id == attachment_id))
            });
            if !should_drain {
                self.remove_pending_notice_queue_id(&key, record_id);
                continue;
            }
            if let Some(record) = self.notice_records.get(&record_id) {
                drained.push(scoped_notice_record(record, attachment_id));
            }
            self.remove_pending_notice_queue_id(&key, record_id);
            if let Some(record) = self.notice_records.get_mut(&record_id) {
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
            }
            self.remove_notice_record_if_drained(record_id);
        }
        self.refresh_health();
        drained
    }

    fn remove_pending_notice_queue_id(&mut self, key: &(String, String), record_id: u64) {
        let empty = match self.pending_notice_by_attachment.get_mut(key) {
            Some(queue) => {
                queue.retain(|queued_id| *queued_id != record_id);
                queue.is_empty()
            }
            None => false,
        };
        if empty {
            self.pending_notice_by_attachment.remove(key);
        }
    }

    fn remove_notice_record_if_drained(&mut self, record_id: u64) {
        let should_remove = self.notice_records.get(&record_id).is_some_and(|record| {
            record.recipient_attachment_ids.is_empty()
                || record.pending_recipient_attachment_ids.is_empty()
        });
        if should_remove {
            self.notice_records.remove(&record_id);
        }
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.input_records
            .retain(|record| record.session_id != session_id);
        let output_record_ids = self
            .output_records
            .iter()
            .filter_map(|(record_id, record)| {
                (record.session_id == session_id).then_some(*record_id)
            })
            .collect::<Vec<_>>();
        for record_id in output_record_ids {
            self.output_records.remove(&record_id);
        }
        self.pending_output_by_attachment
            .retain(|(pending_session_id, _), _| pending_session_id != session_id);
        self.last_output_record_id = self.output_records.keys().next_back().copied();
        self.notice_records
            .retain(|_, record| record.session_id != session_id);
        self.pending_notice_by_attachment
            .retain(|(pending_session_id, _), _| pending_session_id != session_id);
        self.pending_broadcast_notice_by_session.remove(session_id);
        self.completion_records
            .retain(|_, record| record.session_id != session_id);
        self.pending_completion_by_attachment
            .retain(|(pending_session_id, _), _| pending_session_id != session_id);
        self.refresh_health();
    }

    pub fn remove_attachment(&mut self, session_id: &str, attachment_id: &str) -> bool {
        let mut changed = false;
        let key = (session_id.to_string(), attachment_id.to_string());
        if let Some(record_ids) = self.pending_output_by_attachment.remove(&key) {
            changed = true;
            for record_id in record_ids {
                if let Some(record) = self.output_records.get_mut(&record_id) {
                    record
                        .pending_recipient_attachment_ids
                        .retain(|id| id != attachment_id);
                }
                self.remove_output_record_if_drained(record_id);
            }
        }
        if let Some(record_ids) = self.pending_notice_by_attachment.remove(&key) {
            changed = true;
            for record_id in record_ids {
                if let Some(record) = self.notice_records.get_mut(&record_id) {
                    record
                        .pending_recipient_attachment_ids
                        .retain(|id| id != attachment_id);
                }
                self.remove_notice_record_if_drained(record_id);
            }
        }
        let broadcast_notice_ids = self
            .pending_broadcast_notice_by_session
            .remove(session_id)
            .unwrap_or_default();
        changed |= !broadcast_notice_ids.is_empty();
        for record_id in broadcast_notice_ids {
            self.notice_records.remove(&record_id);
        }
        if let Some(record_ids) = self.pending_completion_by_attachment.remove(&key) {
            changed = true;
            for record_id in record_ids {
                if let Some(record) = self.completion_records.get_mut(&record_id) {
                    record
                        .pending_recipient_attachment_ids
                        .retain(|id| id != attachment_id);
                }
                self.remove_completion_record_if_drained(record_id);
            }
        }
        for record in self.notice_records.values_mut() {
            if record.session_id == session_id {
                let previous_len = record.pending_recipient_attachment_ids.len();
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
                changed |= record.pending_recipient_attachment_ids.len() != previous_len;
            }
        }
        for record in self.completion_records.values_mut() {
            if record.session_id == session_id {
                let previous_len = record.pending_recipient_attachment_ids.len();
                record
                    .pending_recipient_attachment_ids
                    .retain(|id| id != attachment_id);
                changed |= record.pending_recipient_attachment_ids.len() != previous_len;
            }
        }
        self.notice_records.retain(|_, record| {
            !record.recipient_attachment_ids.is_empty()
                && !record.pending_recipient_attachment_ids.is_empty()
        });
        self.completion_records
            .retain(|_, record| !record.pending_recipient_attachment_ids.is_empty());
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
    TerminalOutputRecord {
        session_id: record.session_id.clone(),
        provider_run_id: record.provider_run_id.clone(),
        agent_id: record.agent_id.clone(),
        prompt_id: record.prompt_id.clone(),
        source_attachment_id: record.source_attachment_id.clone(),
        kind: record.kind.clone(),
        merge_key: record.merge_key.clone(),
        recipient_attachment_ids: vec![attachment_id.to_string()],
        pending_recipient_attachment_ids: vec![attachment_id.to_string()],
        bytes: record.bytes.clone(),
    }
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

fn terminal_output_record_scoped_json_bytes(
    record: &TerminalOutputRecord,
    attachment_id: &str,
) -> usize {
    let mut total = 2_usize;
    let mut field_count = 0_usize;
    add_json_field(
        &mut total,
        &mut field_count,
        "session_id",
        json_string_len(&record.session_id),
    );
    add_json_field(
        &mut total,
        &mut field_count,
        "provider_run_id",
        json_string_len(&record.provider_run_id),
    );
    if let Some(agent_id) = &record.agent_id {
        add_json_field(
            &mut total,
            &mut field_count,
            "agent_id",
            json_string_len(agent_id),
        );
    }
    if let Some(prompt_id) = &record.prompt_id {
        add_json_field(
            &mut total,
            &mut field_count,
            "prompt_id",
            json_string_len(prompt_id),
        );
    }
    if let Some(source_attachment_id) = &record.source_attachment_id {
        add_json_field(
            &mut total,
            &mut field_count,
            "source_attachment_id",
            json_string_len(source_attachment_id),
        );
    }
    add_json_field(
        &mut total,
        &mut field_count,
        "kind",
        json_string_len(terminal_output_kind_json(&record.kind)),
    );
    if let Some(merge_key) = &record.merge_key {
        add_json_field(
            &mut total,
            &mut field_count,
            "merge_key",
            json_string_len(merge_key),
        );
    }
    let scoped_attachment_array_len = json_string_array_len(std::slice::from_ref(&attachment_id));
    add_json_field(
        &mut total,
        &mut field_count,
        "recipient_attachment_ids",
        scoped_attachment_array_len,
    );
    add_json_field(
        &mut total,
        &mut field_count,
        "pending_recipient_attachment_ids",
        scoped_attachment_array_len,
    );
    add_json_field(
        &mut total,
        &mut field_count,
        "bytes",
        json_byte_array_len(&record.bytes),
    );
    total
}

fn add_json_field(total: &mut usize, field_count: &mut usize, field: &str, value_len: usize) {
    if *field_count > 0 {
        *total = total.saturating_add(1);
    }
    *field_count = field_count.saturating_add(1);
    *total = total
        .saturating_add(json_string_len(field))
        .saturating_add(1)
        .saturating_add(value_len);
}

fn terminal_output_kind_json(kind: &TerminalOutputKind) -> &'static str {
    match kind {
        TerminalOutputKind::ProviderOutput => "provider_output",
        TerminalOutputKind::PromptEcho => "prompt_echo",
        TerminalOutputKind::ProviderReasoning => "provider_reasoning",
        TerminalOutputKind::ProviderTool => "provider_tool",
        TerminalOutputKind::ProviderError => "provider_error",
        TerminalOutputKind::ProviderStatus => "provider_status",
    }
}

fn json_string_array_len(values: &[&str]) -> usize {
    let commas = values.len().saturating_sub(1);
    2_usize
        .saturating_add(commas)
        .saturating_add(values.iter().fold(0_usize, |total, value| {
            total.saturating_add(json_string_len(value))
        }))
}

fn json_string_len(value: &str) -> usize {
    value.chars().fold(2_usize, |total, character| {
        total.saturating_add(match character {
            '"' | '\\' => 2,
            '\u{08}' | '\u{0c}' | '\n' | '\r' | '\t' => 2,
            character if character <= '\u{1f}' => 6,
            character => character.len_utf8(),
        })
    })
}

fn json_byte_array_len(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 2;
    }
    2_usize
        .saturating_add(bytes.len().saturating_sub(1))
        .saturating_add(bytes.iter().fold(0_usize, |total, byte| {
            total.saturating_add(match byte {
                0..=9 => 1,
                10..=99 => 2,
                _ => 3,
            })
        }))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::{
        scoped_output_record, terminal_output_record_scoped_json_bytes, TerminalOutputKind,
        TerminalOutputRecord, TerminalStreamService, TerminalStreamStore,
    };

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
    fn output_drain_size_estimator_bounds_scoped_json() {
        let record = TerminalOutputRecord {
            session_id: "session-\n1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            agent_id: Some("agent-\"1\"".to_string()),
            prompt_id: Some("prompt-1".to_string()),
            source_attachment_id: Some("attachment-source".to_string()),
            kind: TerminalOutputKind::ProviderReasoning,
            merge_key: Some("merge\\key".to_string()),
            recipient_attachment_ids: vec!["attachment-1".to_string(), "attachment-2".to_string()],
            pending_recipient_attachment_ids: vec![
                "attachment-1".to_string(),
                "attachment-2".to_string(),
            ],
            bytes: vec![0, 9, 10, 99, 100, 255],
        };

        let scoped = scoped_output_record(&record, "attachment-2");
        let actual_len = serde_json::to_vec(&scoped)
            .expect("scoped terminal output should serialize")
            .len();
        let estimated_len = terminal_output_record_scoped_json_bytes(&record, "attachment-2");

        assert!(
            estimated_len >= actual_len,
            "estimated scoped JSON length {estimated_len} should bound actual length {actual_len}"
        );
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
    fn notice_pending_index_tracks_recipient_drain() {
        let mut terminal = TerminalStreamService::new();
        terminal.record_notice(
            "session-1",
            None,
            None,
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            "indexed notice",
        );

        assert_eq!(
            terminal
                .pending_notice_by_attachment
                .get(&("session-1".to_string(), "attachment-1".to_string()))
                .map(VecDeque::len),
            Some(1)
        );
        assert_eq!(
            terminal
                .pending_notice_by_attachment
                .get(&("session-1".to_string(), "attachment-2".to_string()))
                .map(VecDeque::len),
            Some(1)
        );

        let first = terminal.drain_notice_records("session-1", "attachment-1");
        assert_eq!(first.len(), 1);
        assert!(!terminal
            .pending_notice_by_attachment
            .contains_key(&("session-1".to_string(), "attachment-1".to_string())));
        assert_eq!(
            terminal
                .pending_notice_by_attachment
                .get(&("session-1".to_string(), "attachment-2".to_string()))
                .map(VecDeque::len),
            Some(1)
        );

        let second = terminal.drain_notice_records("session-1", "attachment-2");
        assert_eq!(second.len(), 1);
        assert!(terminal.pending_notice_by_attachment.is_empty());
        assert!(terminal.notice_records().is_empty());
    }

    #[test]
    fn broadcast_notice_pending_index_tracks_session_drain() {
        let mut terminal = TerminalStreamService::new();
        terminal.record_notice("session-1", None, None, Vec::new(), "broadcast notice");
        terminal.record_notice("session-2", None, None, Vec::new(), "other session notice");

        assert_eq!(
            terminal
                .pending_broadcast_notice_by_session
                .get("session-1")
                .map(VecDeque::len),
            Some(1)
        );
        assert_eq!(
            terminal
                .pending_broadcast_notice_by_session
                .get("session-2")
                .map(VecDeque::len),
            Some(1)
        );

        let first = terminal.drain_notice_records("session-1", "attachment-1");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].message, "broadcast notice");
        assert!(!terminal
            .pending_broadcast_notice_by_session
            .contains_key("session-1"));
        assert!(terminal
            .pending_broadcast_notice_by_session
            .contains_key("session-2"));
        assert_eq!(terminal.notice_records().len(), 1);
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
    fn completion_pending_index_tracks_recipient_drain() {
        let mut terminal = TerminalStreamService::new();
        terminal.record_assistant_message_completion(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            vec!["attachment-1".to_string(), "attachment-2".to_string()],
            "message-1",
            42,
        );

        assert_eq!(
            terminal
                .pending_completion_by_attachment
                .get(&("session-1".to_string(), "attachment-1".to_string()))
                .map(VecDeque::len),
            Some(1)
        );
        assert_eq!(
            terminal
                .pending_completion_by_attachment
                .get(&("session-1".to_string(), "attachment-2".to_string()))
                .map(VecDeque::len),
            Some(1)
        );

        let first = terminal.drain_completion_records("session-1", "attachment-1");
        assert_eq!(first.len(), 1);
        assert!(!terminal
            .pending_completion_by_attachment
            .contains_key(&("session-1".to_string(), "attachment-1".to_string())));
        assert_eq!(
            terminal
                .pending_completion_by_attachment
                .get(&("session-1".to_string(), "attachment-2".to_string()))
                .map(VecDeque::len),
            Some(1)
        );

        let second = terminal.drain_completion_records("session-1", "attachment-2");
        assert_eq!(second.len(), 1);
        assert!(terminal.pending_completion_by_attachment.is_empty());
        assert!(terminal.completion_records.is_empty());
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
            terminal
                .completion_records
                .values()
                .next()
                .expect("completion record should remain")
                .pending_recipient_attachment_ids,
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
        let sequence = terminal.attachment_change_sequence("session-1", "attachment-1");
        let waiter = {
            let terminal = terminal.clone();
            tokio::spawn(async move {
                terminal
                    .wait_for_attachment_change_after("session-1", "attachment-1", sequence)
                    .await;
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
        assert!(terminal.attachment_change_sequence("session-1", "attachment-1") > sequence);
    }

    #[tokio::test]
    async fn terminal_stream_store_does_not_wake_unrelated_attachment_waiters() {
        let terminal = TerminalStreamStore::new();
        let sequence = terminal.attachment_change_sequence("session-1", "attachment-2");

        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            None,
            TerminalOutputKind::ProviderOutput,
            None,
            vec!["attachment-1".to_string()],
            b"output",
        );

        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(25),
                terminal.wait_for_attachment_change_after("session-1", "attachment-2", sequence),
            )
            .await
            .is_err(),
            "unrelated attachment waiter should not wake"
        );
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

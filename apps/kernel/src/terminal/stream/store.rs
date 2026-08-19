use super::*;
use std::hash::{Hash, Hasher};

const TERMINAL_STREAM_SHARD_COUNT: usize = 64;

#[derive(Debug, Clone)]
pub struct TerminalStreamStore {
    shards: Arc<[StdMutex<TerminalStreamService>]>,
    health_store: TerminalStreamHealthStore,
    changes: Arc<TerminalStreamChangeSignal>,
    session_changes: Arc<[StdMutex<BTreeMap<String, Arc<TerminalStreamChangeSignal>>>]>,
    attachment_changes:
        Arc<[StdMutex<BTreeMap<(String, String), Arc<TerminalStreamChangeSignal>>>]>,
}

#[derive(Debug, Default)]
struct TerminalStreamChangeSignal {
    sequence: AtomicU64,
    notify: Notify,
}

impl TerminalStreamStore {
    pub fn new() -> Self {
        let shards = (0..TERMINAL_STREAM_SHARD_COUNT)
            .map(|_| StdMutex::new(TerminalStreamService::new()))
            .collect::<Vec<_>>();
        let health_store = TerminalStreamHealthStore::aggregate(shards.iter().map(|shard| {
            shard
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .health_store()
        }));
        Self {
            shards: shards.into(),
            health_store,
            changes: Arc::new(TerminalStreamChangeSignal::default()),
            session_changes: (0..TERMINAL_STREAM_SHARD_COUNT)
                .map(|_| StdMutex::new(BTreeMap::new()))
                .collect::<Vec<_>>()
                .into(),
            attachment_changes: (0..TERMINAL_STREAM_SHARD_COUNT)
                .map(|_| StdMutex::new(BTreeMap::new()))
                .collect::<Vec<_>>()
                .into(),
        }
    }

    fn shard_index(session_id: &str) -> usize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        session_id.hash(&mut hasher);
        (hasher.finish() as usize) % TERMINAL_STREAM_SHARD_COUNT
    }

    fn shard(&self, session_id: &str) -> &StdMutex<TerminalStreamService> {
        &self.shards[Self::shard_index(session_id)]
    }

    #[cfg(test)]
    pub(super) fn shard_index_for_test(session_id: &str) -> usize {
        Self::shard_index(session_id)
    }

    #[cfg(test)]
    pub(super) fn hold_session_shard_for_test(
        &self,
        session_id: &str,
        release: &std::sync::Barrier,
    ) {
        let _guard = self
            .shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        release.wait();
        release.wait();
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

    pub fn session_change_sequence(&self, session_id: &str) -> u64 {
        self.session_signal(session_id)
            .sequence
            .load(Ordering::Acquire)
    }

    pub async fn wait_for_session_change_after(&self, session_id: &str, sequence: u64) {
        let signal = self.session_signal(session_id);
        if signal.sequence.load(Ordering::Acquire) != sequence {
            return;
        }
        let notified = signal.notify.notified();
        if signal.sequence.load(Ordering::Acquire) != sequence {
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
        self.health_store.clone()
    }

    pub fn record_input(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        bytes: &[u8],
    ) {
        self.shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        self.fan_out_output_with_prompt_origin(
            session_id,
            provider_run_id,
            agent_id,
            kind,
            merge_key,
            None,
            recipient_attachment_ids,
            bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_output_with_prompt_origin(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        prompt_origin: Option<PromptOrigin>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        self.fan_out_output_with_prompt_metadata(
            session_id,
            provider_run_id,
            agent_id,
            kind,
            merge_key,
            prompt_origin,
            None,
            recipient_attachment_ids,
            bytes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_output_with_prompt_metadata(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        prompt_origin: Option<PromptOrigin>,
        source_attachment_id: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = self
            .shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fan_out_output_with_prompt_metadata(
                session_id,
                provider_run_id,
                agent_id,
                kind,
                merge_key,
                prompt_origin,
                source_attachment_id,
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
        let output_count = outputs.len();
        let mut by_shard = BTreeMap::<usize, Vec<(usize, TerminalOutputAppend)>>::new();
        for (index, output) in outputs.into_iter().enumerate() {
            by_shard
                .entry(Self::shard_index(&output.session_id))
                .or_default()
                .push((index, output));
        }
        let mut records = vec![None; output_count];
        for (shard_index, indexed_outputs) in by_shard {
            let indexes = indexed_outputs
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>();
            let fanout = self.shards[shard_index]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .fan_out_outputs(
                    indexed_outputs
                        .into_iter()
                        .map(|(_, output)| output)
                        .collect(),
                );
            self.record_change_for_keys(fanout.changed_keys);
            for (index, record) in indexes.into_iter().zip(fanout.records) {
                records[index] = Some(record);
            }
        }
        records.into_iter().flatten().collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_external_observed_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
        external_observation_metadata: TerminalOutputExternalObservationMetadata,
        source_attachment_id: Option<String>,
    ) -> TerminalOutputRecord {
        let record = self
            .shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fan_out_external_observed_output(
                session_id,
                provider_run_id,
                agent_id,
                kind,
                merge_key,
                recipient_attachment_ids,
                bytes,
                external_observation_metadata,
                source_attachment_id,
            );
        self.record_change_for_record(&record);
        record
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_prompt_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        prompt_id: &str,
        prompt_origin: Option<PromptOrigin>,
        source_attachment_id: &str,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = self
            .shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fan_out_prompt_output(
                session_id,
                provider_run_id,
                agent_id,
                prompt_id,
                prompt_origin,
                source_attachment_id,
                recipient_attachment_ids,
                bytes,
            );
        self.record_change_for_record(&record);
        record
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fan_out_prompt_output_with_merge_key(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        prompt_id: &str,
        prompt_origin: Option<PromptOrigin>,
        source_attachment_id: &str,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let record = self
            .shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fan_out_prompt_output_with_merge_key(
                session_id,
                provider_run_id,
                agent_id,
                prompt_id,
                prompt_origin,
                source_attachment_id,
                merge_key,
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
            .shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        self.shards
            .iter()
            .flat_map(|shard| {
                shard
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .input_records()
                    .to_vec()
            })
            .collect()
    }

    pub fn output_records(&self) -> Vec<TerminalOutputRecord> {
        self.shards
            .iter()
            .flat_map(|shard| {
                shard
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .output_records()
            })
            .collect()
    }

    pub fn notice_records(&self) -> Vec<RuntimeNoticeRecord> {
        self.shards
            .iter()
            .flat_map(|shard| {
                shard
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .notice_records()
            })
            .collect()
    }

    pub fn health_snapshot(&self) -> TerminalStreamHealthSnapshot {
        self.health_store().snapshot()
    }

    pub fn drain_output_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<TerminalOutputRecord> {
        self.shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain_output_records(session_id, attachment_id)
    }

    pub fn has_pending_output_records(&self, session_id: &str, attachment_id: &str) -> bool {
        self.shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .has_pending_output_records(session_id, attachment_id)
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
            .shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        self.shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain_completion_records(session_id, attachment_id)
    }

    pub fn drain_notice_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<RuntimeNoticeRecord> {
        self.shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain_notice_records(session_id, attachment_id)
    }

    pub fn remove_session(&self, session_id: &str) {
        self.shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove_session(session_id);
        let shard_index = Self::shard_index(session_id);
        self.attachment_changes[shard_index]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(signal_session_id, _), _| signal_session_id != session_id);
        self.session_changes[shard_index]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(session_id);
        self.record_change();
    }

    pub fn remove_attachment(&self, session_id: &str, attachment_id: &str) {
        let changed = self
            .shard(session_id)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove_attachment(session_id, attachment_id);
        if changed {
            self.attachment_changes[Self::shard_index(session_id)]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&(session_id.to_string(), attachment_id.to_string()));
            self.record_change_for_session(session_id);
        }
    }

    pub fn notify_terminal_projection_change(&self, session_id: &str) {
        self.record_change_for_session(session_id);
    }

    fn record_change(&self) {
        self.changes.sequence.fetch_add(1, Ordering::AcqRel);
        self.changes.notify.notify_waiters();
    }

    fn record_change_for_record(&self, record: &TerminalOutputRecord) {
        if record.recipient_attachment_ids.is_empty() {
            return;
        }
        self.record_change_for_attachment_ids(&record.session_id, &record.recipient_attachment_ids);
    }

    fn record_change_for_notice(&self, record: &RuntimeNoticeRecord) {
        if record.recipient_attachment_ids.is_empty() {
            self.record_change_for_session(&record.session_id);
        } else {
            self.record_change_for_attachment_ids(
                &record.session_id,
                &record.recipient_attachment_ids,
            );
        }
    }

    fn record_change_for_completion(&self, record: &AssistantMessageCompletionRecord) {
        if record.recipient_attachment_ids.is_empty() {
            return;
        }
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

    fn record_change_for_session(&self, session_id: &str) {
        let signal = self.session_signal(session_id);
        signal.sequence.fetch_add(1, Ordering::AcqRel);
        signal.notify.notify_waiters();
    }

    fn session_signal(&self, session_id: &str) -> Arc<TerminalStreamChangeSignal> {
        self.session_changes[Self::shard_index(session_id)]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    fn attachment_signal(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Arc<TerminalStreamChangeSignal> {
        self.attachment_changes[Self::shard_index(session_id)]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry((session_id.to_string(), attachment_id.to_string()))
            .or_default()
            .clone()
    }
}

impl Default for TerminalStreamStore {
    fn default() -> Self {
        Self::new()
    }
}

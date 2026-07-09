use super::*;

#[derive(Debug, Clone, Default)]
pub struct TerminalStreamStore {
    inner: Arc<StdMutex<TerminalStreamService>>,
    changes: Arc<TerminalStreamChangeSignal>,
    session_changes: Arc<StdMutex<BTreeMap<String, Arc<TerminalStreamChangeSignal>>>>,
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
            session_changes: Arc::new(StdMutex::new(BTreeMap::new())),
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
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
            .inner
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
        let fanout = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fan_out_outputs(outputs);
        if !fanout.records.is_empty() {
            self.record_change_for_keys(fanout.changed_keys);
        }
        fanout.records
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
    ) -> TerminalOutputRecord {
        let record = self
            .inner
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
            .inner
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
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .input_records()
            .to_vec()
    }

    pub fn output_records(&self) -> Vec<TerminalOutputRecord> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .output_records()
    }

    pub fn notice_records(&self) -> Vec<RuntimeNoticeRecord> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain_output_records(session_id, attachment_id)
    }

    pub fn has_pending_output_records(&self, session_id: &str, attachment_id: &str) -> bool {
        self.inner
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
            .inner
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
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain_completion_records(session_id, attachment_id)
    }

    pub fn drain_notice_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<RuntimeNoticeRecord> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain_notice_records(session_id, attachment_id)
    }

    pub fn remove_session(&self, session_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove_session(session_id);
        self.attachment_changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(signal_session_id, _), _| signal_session_id != session_id);
        self.session_changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(session_id);
        self.record_change();
    }

    pub fn remove_attachment(&self, session_id: &str, attachment_id: &str) {
        let changed = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove_attachment(session_id, attachment_id);
        if changed {
            self.attachment_changes
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
            self.record_change_for_session(&record.session_id);
        } else {
            self.record_change_for_attachment_ids(
                &record.session_id,
                &record.recipient_attachment_ids,
            );
        }
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
            self.record_change_for_session(&record.session_id);
        } else {
            self.record_change_for_attachment_ids(
                &record.session_id,
                &record.recipient_attachment_ids,
            );
        }
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
        self.session_changes
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
        self.attachment_changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry((session_id.to_string(), attachment_id.to_string()))
            .or_default()
            .clone()
    }
}
